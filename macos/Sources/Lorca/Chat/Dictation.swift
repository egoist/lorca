import AVFoundation
import Carbon
import Speech

/// Live speech-to-text for the composer's Dictate button: the microphone feeds Apple's speech
/// recognizer in the language the user speaks (the first of the system's preferred languages
/// the recognizer knows, unless Preferences names one), partial transcripts accumulate until
/// the user stops or sends, and the input level drives the pill's bars.
@MainActor
final class Dictation {
    enum Failure: LocalizedError {
        case speechDenied
        case microphoneDenied
        case unavailable
        case engine(Error)

        var errorDescription: String? {
            switch self {
            case .speechDenied: L("Speech recognition is turned off for Lorca.")
            case .microphoneDenied: L("Lorca cannot use the microphone.")
            case .unavailable: L("Speech recognition is not available for this language right now.")
            case let .engine(error): error.localizedDescription
            }
        }

        var settingsPane: String? {
            switch self {
            case .speechDenied: "Privacy_SpeechRecognition"
            case .microphoneDenied: "Privacy_Microphone"
            default: nil
            }
        }
    }

    /// The transcript so far, and whether the recognizer considers it final.
    var onTranscript: ((String, Bool) -> Void)?
    /// Input level, 0…1, a few times a second while listening.
    var onLevel: ((Float) -> Void)?
    /// Listening ended: after `stop()`, a final result, or a failure.
    var onEnd: ((Failure?) -> Void)?

    private(set) var isListening = false
    private(set) var startedAt: Date?
    private var engine: AVAudioEngine?
    private var request: SFSpeechAudioBufferRecognitionRequest?
    private var task: SFSpeechRecognitionTask?
    private var recognizer: SFSpeechRecognizer?
    private var stopTimer: Timer?

    // MARK: - Language

    /// The recognizer knows some sixty locales; the menus offer the ones most people speak, the
    /// same list as the phone (`COMMON` in `mobile/src/ui/dictation.ts`).
    private static let common: Set<String> = [
        "ar-SA", "yue-CN", "zh-CN", "zh-HK", "zh-TW", "nl-NL", "en-AU", "en-IN", "en-GB", "en-US", "fr-FR", "de-DE",
        "hi-IN", "id-ID", "it-IT", "ja-JP", "ko-KR", "pt-BR", "ru-RU", "es-MX", "es-ES", "th-TH", "tr-TR", "vi-VN",
    ]

    /// The languages the menus list, by name: the common ones the recognizer supports, and the
    /// one in the preference when it is another.
    static var supportedLocales: [Locale] {
        SFSpeechRecognizer.supportedLocales()
            .filter { locale in
                let tag = locale.identifier.replacingOccurrences(of: "_", with: "-")
                return common.contains(tag) || locale.identifier == Preferences.dictationLanguage
            }
            .sorted { displayName($0).compare(displayName($1), locale: AppLanguage.locale) == .orderedAscending }
    }

    static func displayName(_ locale: Locale) -> String {
        AppLanguage.locale.localizedString(forIdentifier: locale.identifier) ?? locale.identifier
    }

    /// The locale to listen in: the preference, else the language of the keyboard input source
    /// in use (someone typing with a Pinyin input method on an English system is about to
    /// speak Chinese), else the first system language the recognizer supports.
    static func locale() -> Locale {
        let supported = SFSpeechRecognizer.supportedLocales()
        if let chosen = Preferences.dictationLanguage, let locale = supported.first(where: { $0.identifier == chosen }) {
            return locale
        }
        return automaticLocale(supported: supported)
    }

    static func automaticLocale(supported: Set<Locale> = SFSpeechRecognizer.supportedLocales()) -> Locale {
        let candidates = inputSourceLanguages() + Locale.preferredLanguages
        for tag in candidates {
            if let match = bestMatch(for: tag, in: supported) { return match }
        }
        return Locale.current
    }

    /// Language tags of the active keyboard input source, such as ["zh-Hans"] for a Pinyin
    /// input method or ["en"] for the U.S. layout.
    private static func inputSourceLanguages() -> [String] {
        guard let source = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue(),
            let pointer = TISGetInputSourceProperty(source, kTISPropertyInputSourceLanguages)
        else { return [] }
        return Unmanaged<CFArray>.fromOpaque(pointer).takeUnretainedValue() as? [String] ?? []
    }

    /// "zh-Hans-CN" and "zh-Hans" find "zh-CN"; "zh-Hant" finds "zh-TW"; "en" finds the
    /// English the system prefers, else "en-US".
    private static func bestMatch(for tag: String, in supported: Set<Locale>) -> Locale? {
        let wanted = Locale(identifier: tag)
        guard let language = wanted.language.languageCode?.identifier else { return nil }
        let sameLanguage = supported.filter { $0.language.languageCode?.identifier == language }
        if sameLanguage.isEmpty { return nil }
        if let region = wanted.region?.identifier, let match = sameLanguage.first(where: { $0.region?.identifier == region }) {
            return match
        }
        let script = wanted.language.script?.identifier
        if script == "Hans", let match = sameLanguage.first(where: { $0.region?.identifier == "CN" }) { return match }
        if script == "Hant", let match = sameLanguage.first(where: { $0.region?.identifier == "TW" }) { return match }
        // A bare language: the region the system prefers for it, else a stable first choice.
        for preferred in Locale.preferredLanguages {
            let candidate = Locale(identifier: preferred)
            if candidate.language.languageCode?.identifier == language, let region = candidate.region?.identifier,
                let match = sameLanguage.first(where: { $0.region?.identifier == region })
            {
                return match
            }
        }
        let defaults = ["en": "US", "zh": "CN", "es": "ES", "fr": "FR", "de": "DE", "pt": "BR", "ja": "JP", "ko": "KR"]
        if let region = defaults[language], let match = sameLanguage.first(where: { $0.region?.identifier == region }) { return match }
        return sameLanguage.sorted { $0.identifier < $1.identifier }.first
    }

    // MARK: - Listening

    func start() {
        guard !isListening else { return }
        isListening = true
        startedAt = Date()
        authorize { [weak self] failure in
            guard let self else { return }
            if let failure {
                finish(failure)
                return
            }
            do {
                try beginCapture()
            } catch {
                finish(.engine(error))
            }
        }
    }

    /// Ends the audio and gives the recognizer a moment to settle its last words.
    func stop() {
        guard isListening, stopTimer == nil else { return }
        engine?.inputNode.removeTap(onBus: 0)
        engine?.stop()
        request?.endAudio()
        stopTimer = Timer.scheduledTimer(withTimeInterval: 1.5, repeats: false) { [weak self] _ in
            Task { @MainActor in self?.finish(nil) }
        }
    }

    /// Ends listening at once, dropping whatever was not committed.
    func cancel() {
        finish(nil)
    }

    private func authorize(_ completion: @escaping (Failure?) -> Void) {
        SFSpeechRecognizer.requestAuthorization { status in
            DispatchQueue.main.async {
                guard status == .authorized else {
                    completion(.speechDenied)
                    return
                }
                AVCaptureDevice.requestAccess(for: .audio) { granted in
                    DispatchQueue.main.async {
                        completion(granted ? nil : .microphoneDenied)
                    }
                }
            }
        }
    }

    private func beginCapture() throws {
        guard let recognizer = SFSpeechRecognizer(locale: Self.locale()), recognizer.isAvailable else { throw Failure.unavailable }
        self.recognizer = recognizer

        let request = SFSpeechAudioBufferRecognitionRequest()
        request.shouldReportPartialResults = true
        request.addsPunctuation = true
        self.request = request

        let engine = AVAudioEngine()
        let input = engine.inputNode
        let format = input.outputFormat(forBus: 0)
        guard format.sampleRate > 0 else { throw Failure.microphoneDenied }
        var lastLevelAt = Date.distantPast
        input.installTap(onBus: 0, bufferSize: 1024, format: format) { [weak self] buffer, _ in
            request.append(buffer)
            let now = Date()
            guard now.timeIntervalSince(lastLevelAt) > 0.06 else { return }
            lastLevelAt = now
            let level = Self.level(of: buffer)
            DispatchQueue.main.async { self?.onLevel?(level) }
        }
        engine.prepare()
        try engine.start()
        self.engine = engine

        task = recognizer.recognitionTask(with: request) { [weak self] result, error in
            DispatchQueue.main.async {
                guard let self, self.isListening else { return }
                if let result {
                    self.onTranscript?(result.bestTranscription.formattedString, result.isFinal)
                    if result.isFinal { self.finish(nil) }
                } else if let error {
                    // Ending the audio stream reports a cancellation; that is not a failure.
                    let code = (error as NSError).code
                    self.finish(code == 216 || code == 301 || self.stopTimer != nil ? nil : .engine(error))
                }
            }
        }
    }

    /// RMS of the buffer on a rough decibel scale: silence is 0, speech reaches 1.
    nonisolated private static func level(of buffer: AVAudioPCMBuffer) -> Float {
        guard let channel = buffer.floatChannelData?[0], buffer.frameLength > 0 else { return 0 }
        var sum: Float = 0
        for index in 0..<Int(buffer.frameLength) {
            let sample = channel[index]
            sum += sample * sample
        }
        let rms = (sum / Float(buffer.frameLength)).squareRoot()
        let decibels = 20 * log10(max(rms, 0.000_01))
        return min(1, max(0, (decibels + 50) / 45))
    }

    private func finish(_ failure: Failure?) {
        guard isListening else { return }
        isListening = false
        stopTimer?.invalidate()
        stopTimer = nil
        engine?.inputNode.removeTap(onBus: 0)
        engine?.stop()
        engine = nil
        task?.cancel()
        task = nil
        request = nil
        onEnd?(failure)
    }
}
