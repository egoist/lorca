import AppKit

final class ComposerTextView: NSTextView {
    var onKeyCommand: ((Selector) -> Bool)?
    /// Files or an image on the pasteboard become attachments instead of text. Return true when
    /// the paste was taken.
    var onPasteFiles: (([URL]) -> Bool)?
    var onPasteImage: ((Data) -> Bool)?
    var placeholder: String = "" {
        didSet { needsDisplay = true }
    }

    override func doCommand(by selector: Selector) {
        if onKeyCommand?(selector) == true { return }
        super.doCommand(by: selector)
    }

    override func paste(_ sender: Any?) {
        let pasteboard = NSPasteboard.general
        if let urls = pasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL],
            !urls.isEmpty, onPasteFiles?(urls) == true
        {
            return
        }
        // An image with no text beside it (a screenshot, a copied picture) is an attachment.
        let hasText = pasteboard.types?.contains(.string) ?? false
        if !hasText, let image = NSImage(pasteboard: pasteboard), let tiff = image.tiffRepresentation,
            let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]),
            onPasteImage?(png) == true
        {
            return
        }
        super.paste(sender)
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, !placeholder.isEmpty else { return }
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font ?? Theme.Font.message,
            .foregroundColor: NSColor.placeholderTextColor,
        ]
        let origin = NSPoint(
            x: textContainerInset.width + (textContainer?.lineFragmentPadding ?? 0),
            y: textContainerInset.height
        )
        placeholder.draw(at: origin, withAttributes: attributes)
    }
}

/// Round symbol button for the composer: a solid disc for the primary action,
/// a quiet disc for secondary ones, or bare glyph that only fills under the pointer.
/// `listening` is the Dictate button while it records: red, breathing.
final class ComposerButton: NSButton {
    enum Style { case primary, secondary, plain, listening }

    var style: Style = .plain {
        didSet {
            needsDisplay = true
            if style == .listening { startPulse() } else { stopPulse() }
        }
    }

    private let diameter: CGFloat = 28
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    init(symbol: String, pointSize: CGFloat, weight: NSFont.Weight, tooltip: String, target: AnyObject?, action: Selector) {
        super.init(frame: .zero)
        image = NSImage(systemSymbolName: symbol, accessibilityDescription: tooltip)
        symbolConfiguration = NSImage.SymbolConfiguration(pointSize: pointSize, weight: weight)
        imagePosition = .imageOnly
        isBordered = false
        toolTip = tooltip
        self.target = target
        self.action = action
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize { NSSize(width: diameter, height: diameter) }
    override var alignmentRectInsets: NSEdgeInsets { NSEdgeInsets() }

    func setSymbol(_ name: String, pointSize: CGFloat, weight: NSFont.Weight) {
        image = NSImage(systemSymbolName: name, accessibilityDescription: toolTip)
        symbolConfiguration = NSImage.SymbolConfiguration(pointSize: pointSize, weight: weight)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { isHovered = true }
    override func mouseExited(with event: NSEvent) { isHovered = false }

    private func startPulse() {
        let pulse = CABasicAnimation(keyPath: "opacity")
        pulse.fromValue = 1
        pulse.toValue = 0.55
        pulse.duration = 0.9
        pulse.autoreverses = true
        pulse.repeatCount = .infinity
        pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        layer?.add(pulse, forKey: "pulse")
    }

    private func stopPulse() {
        layer?.removeAnimation(forKey: "pulse")
    }

    override func draw(_ dirtyRect: NSRect) {
        let fill: NSColor?
        switch style {
        case .primary:
            fill = Theme.composerPrimary.withAlphaComponent(isHighlighted ? 0.8 : 1)
            contentTintColor = Theme.composerPrimaryContent
        case .secondary:
            fill = Theme.composerControl.withAlphaComponent(isHighlighted || isHovered ? 0.18 : 0.1)
            contentTintColor = .labelColor
        case .plain:
            fill = isHighlighted || isHovered ? Theme.composerControl : nil
            contentTintColor = .secondaryLabelColor
        case .listening:
            fill = NSColor.systemRed.withAlphaComponent(isHighlighted ? 0.8 : 1)
            contentTintColor = .white
        }
        if let fill {
            fill.setFill()
            NSBezierPath(ovalIn: bounds).fill()
        }
        super.draw(dirtyRect)
    }
}

final class ComposerView: NSView {
    private let field = BackgroundView()
    /// Liquid glass under the pill on macOS 26+; the field is then a clear view that draws
    /// the hairline edge over it, as the phone composer does. Elsewhere the field is filled.
    private var glass: NSView?
    private let scrollView = NSScrollView()
    private let textView: ComposerTextView
    private let strip = ComposerAttachmentStrip()
    private let attachButton: ComposerButton
    private let voiceButton: ComposerButton
    private let sendButton: ComposerButton
    private let stopButton: ComposerButton
    private let trailing: NSStackView
    private let mentions = MentionPanel()
    private let dictation = Dictation()

    /// Single line: controls sit beside the text in a pill.
    /// Expanded: text spans the field with the controls in a row underneath. Attachments
    /// always expand the field, with their chips above the text.
    private enum Mode { case compact, expanded }
    private var mode: Mode = .compact
    private var compactConstraints: [NSLayoutConstraint] = []
    private var expandedConstraints: [NSLayoutConstraint] = []
    private var heightConstraint: NSLayoutConstraint!
    private var stripTopConstraint: NSLayoutConstraint!
    private var stripHeightConstraint: NSLayoutConstraint!

    private let controlSize: CGFloat = 28
    private let controlInset: CGFloat = 8
    private let controlSpacing: CGFloat = 4
    private let textInset: CGFloat = 8
    private let expandedTextInset: CGFloat = 12
    private let maxTextHeight: CGFloat = 168
    private var lineHeight: CGFloat {
        ceil(textView.layoutManager?.defaultLineHeight(for: Theme.Font.message) ?? 17)
    }
    private var minTextHeight: CGFloat { lineHeight + textView.textContainerInset.height * 2 }

    private(set) var attachments: [OutgoingAttachment] = []
    /// While listening the field shows the pill; the transcript lands at this caret location
    /// when the user stops or sends, the way Grok Bot commits a recording.
    private let pill = RecordingPill()
    private var dictationLocation = 0
    private var dictationTranscript = ""
    private var pendingSend = false
    private var dictationTimer: Timer?
    /// Escape while recording, wherever focus is: the text view is hidden then, so key
    /// commands would not reach it.
    private var escapeMonitor: Any?
    private var placeholder = ""

    /// The text, its files, and the bots its `@Name`s picked from the menu, by id.
    var onSend: ((String, [OutgoingAttachment], [Bot.ID]) -> Void)?
    var onStop: (() -> Void)?
    var mentionableBots: [Bot] = []
    /// The bots picked from the `@` menu since the last send, in order. Two bots can share a
    /// name; the pick says which one the user meant.
    private var pickedMentions: [Bot] = []

    var isResponding = false {
        didSet {
            updateButtons()
            updateLayout()
        }
    }

    var text: String {
        get { textView.string }
        set {
            textView.string = newValue
            pickedMentions = []
            handleTextChange()
        }
    }

    init() {
        // An explicit TextKit 1 stack keeps `layoutManager` available for height tracking.
        let storage = NSTextStorage()
        let layoutManager = NSLayoutManager()
        storage.addLayoutManager(layoutManager)
        let container = NSTextContainer(size: NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = true
        layoutManager.addTextContainer(container)
        textView = ComposerTextView(frame: .zero, textContainer: container)

        attachButton = ComposerButton(
            symbol: "plus", pointSize: 14, weight: .medium, tooltip: L("Attach files"),
            target: nil, action: #selector(attach))
        voiceButton = ComposerButton(
            symbol: "mic.fill", pointSize: 13, weight: .medium, tooltip: L("Dictate · right-click for the language"),
            target: nil, action: #selector(voice))
        sendButton = ComposerButton(
            symbol: "arrow.up", pointSize: 13, weight: .bold, tooltip: L("Send"),
            target: nil, action: #selector(send))
        stopButton = ComposerButton(
            symbol: "stop.fill", pointSize: 11, weight: .bold, tooltip: L("Stop responding (⌘.)"),
            target: nil, action: #selector(stop))
        // Send stays at the trailing edge when a draft steers work in progress; Stop sits just
        // before it as the separate hard-cancel action.
        trailing = Build.stack([voiceButton, stopButton, sendButton], orientation: .horizontal, spacing: 4)

        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        registerForDraggedTypes([.fileURL])

        configureTextView()
        for button in [attachButton, voiceButton, sendButton, stopButton] {
            button.target = self
        }
        attachButton.style = .secondary
        sendButton.style = .primary
        stopButton.style = .primary
        trailing.spacing = controlSpacing

        field.cornerRadius = (controlSize + controlInset * 2) / 2
        field.borderColor = Theme.composerBorder
        if #available(macOS 26, *) {
            let glass = NSGlassEffectView()
            glass.cornerRadius = field.cornerRadius
            glass.translatesAutoresizingMaskIntoConstraints = false
            field.fillColor = .clear
            field.addSubview(glass)
            NSLayoutConstraint.activate([
                glass.topAnchor.constraint(equalTo: field.topAnchor),
                glass.leadingAnchor.constraint(equalTo: field.leadingAnchor),
                glass.trailingAnchor.constraint(equalTo: field.trailingAnchor),
                glass.bottomAnchor.constraint(equalTo: field.bottomAnchor),
            ])
            self.glass = glass
        } else {
            field.fillColor = Theme.composerField
        }

        scrollView.documentView = textView
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = false
        scrollView.verticalScrollElasticity = .none
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        addSubview(field)
        field.addSubview(strip)
        field.addSubview(scrollView)
        field.addSubview(pill)
        field.addSubview(attachButton)
        field.addSubview(trailing)
        pill.isHidden = true
        pill.onStop = { [weak self] in self?.dictation.stop() }

        heightConstraint = scrollView.heightAnchor.constraint(equalToConstant: minTextHeight)
        stripTopConstraint = strip.topAnchor.constraint(equalTo: field.topAnchor)
        stripHeightConstraint = strip.heightAnchor.constraint(equalToConstant: 0)

        NSLayoutConstraint.activate([
            field.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 20),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -20),
            field.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -14),

            strip.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: expandedTextInset),
            strip.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -expandedTextInset),
            stripTopConstraint,
            stripHeightConstraint,

            attachButton.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: controlInset),
            attachButton.widthAnchor.constraint(equalToConstant: controlSize),
            attachButton.heightAnchor.constraint(equalToConstant: controlSize),

            trailing.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -controlInset),
            trailing.heightAnchor.constraint(equalToConstant: controlSize),
            trailing.centerYAnchor.constraint(equalTo: attachButton.centerYAnchor),

            // The recording pill sits at the right, beside Send, where Grok Bot puts it.
            pill.trailingAnchor.constraint(equalTo: trailing.leadingAnchor, constant: -textInset),
            pill.leadingAnchor.constraint(greaterThanOrEqualTo: scrollView.leadingAnchor),
            pill.centerYAnchor.constraint(equalTo: trailing.centerYAnchor),
            pill.heightAnchor.constraint(equalToConstant: controlSize),

            heightConstraint,
        ])

        compactConstraints = [
            attachButton.centerYAnchor.constraint(equalTo: field.centerYAnchor),
            attachButton.topAnchor.constraint(equalTo: field.topAnchor, constant: controlInset),
            scrollView.leadingAnchor.constraint(equalTo: attachButton.trailingAnchor, constant: textInset),
            scrollView.trailingAnchor.constraint(equalTo: trailing.leadingAnchor, constant: -textInset),
            scrollView.centerYAnchor.constraint(equalTo: field.centerYAnchor),
        ]

        expandedConstraints = [
            scrollView.topAnchor.constraint(equalTo: strip.bottomAnchor, constant: expandedTextInset),
            scrollView.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: expandedTextInset),
            scrollView.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -expandedTextInset),
            attachButton.topAnchor.constraint(equalTo: scrollView.bottomAnchor, constant: 6),
            attachButton.bottomAnchor.constraint(equalTo: field.bottomAnchor, constant: -controlInset),
        ]

        NSLayoutConstraint.activate(compactConstraints)

        mentions.onPick = { [weak self] bot in self?.insertMention(bot) }
        strip.onRemove = { [weak self] index in self?.removeAttachment(at: index) }
        textView.onPasteFiles = { [weak self] urls in self?.addFiles(urls) ?? false }
        textView.onPasteImage = { [weak self] png in self?.addPastedImage(png) ?? false }
        configureDictation()
        let languages = NSMenu()
        languages.delegate = self
        voiceButton.menu = languages
        updateButtons()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The composer floats over the transcript; only the pill takes clicks, so the margins
    /// beside it still reach the messages underneath.
    override func hitTest(_ point: NSPoint) -> NSView? {
        let local = convert(point, from: superview)
        guard field.frame.contains(local) else { return nil }
        return super.hitTest(point)
    }

    private func configureTextView() {
        textView.delegate = self
        textView.font = Theme.Font.message
        textView.textColor = .labelColor
        textView.drawsBackground = false
        textView.isRichText = false
        textView.isEditable = true
        textView.isSelectable = true
        textView.allowsUndo = true
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainerInset = NSSize(width: 2, height: 4)
        textView.insertionPointColor = .controlAccentColor
        textView.onKeyCommand = { [weak self] selector in
            self?.handle(selector) ?? false
        }
    }

    // MARK: - Focus

    func focus() {
        window?.makeFirstResponder(textView)
    }

    func configure(placeholder: String, bots: [Bot]) {
        self.placeholder = placeholder
        if !dictation.isListening { textView.placeholder = placeholder }
        mentionableBots = bots
        updateButtons()
    }

    // MARK: - Actions

    private var hasContent: Bool {
        !textView.string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !attachments.isEmpty
    }

    @objc private func send() {
        if dictation.isListening {
            // The recording ends, its words land in the field, and the message goes.
            pendingSend = true
            dictation.stop()
            return
        }
        guard hasContent else { return }
        let value = textView.string.trimmingCharacters(in: .whitespacesAndNewlines)
        let files = attachments
        // A pick counts while its `@Name` is still in the text.
        let mentioned = pickedMentions.filter { value.range(of: "@\($0.name)", options: .caseInsensitive) != nil }.map(\.id)
        pickedMentions = []
        textView.string = ""
        attachments = []
        mentions.dismiss()
        updateAttachments()
        handleTextChange()
        onSend?(value, files, mentioned)
    }

    @objc private func stop() {
        onStop?()
    }

    @objc private func attach() {
        guard let window else { return }
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = false
        panel.allowsMultipleSelection = true
        panel.message = L("Attach files to your message")
        panel.prompt = L("Attach")
        panel.beginSheetModal(for: window) { [weak self] response in
            guard response == .OK, let self else { return }
            _ = addFiles(panel.urls)
            focus()
        }
    }

    // MARK: - Attachments

    /// Adds the files it can; the rest get one alert. True when any were taken.
    @discardableResult
    func addFiles(_ urls: [URL]) -> Bool {
        var problems: [String] = []
        var added = false
        for url in urls {
            if attachments.count >= OutgoingAttachment.maxCount {
                problems.append(L("At most %d files per message.", OutgoingAttachment.maxCount))
                break
            }
            do {
                attachments.append(try OutgoingAttachment.make(url: url))
                added = true
            } catch {
                problems.append(error.localizedDescription)
            }
        }
        if added { updateAttachments() }
        if !problems.isEmpty, let window {
            let alert = NSAlert()
            alert.messageText = L("Some files were not attached")
            alert.informativeText = problems.joined(separator: "\n")
            alert.beginSheetModal(for: window)
        }
        return added || !problems.isEmpty
    }

    private func addPastedImage(_ png: Data) -> Bool {
        guard attachments.count < OutgoingAttachment.maxCount, let outgoing = try? OutgoingAttachment.make(pastedPNG: png) else {
            return false
        }
        attachments.append(outgoing)
        updateAttachments()
        return true
    }

    private func removeAttachment(at index: Int) {
        guard attachments.indices.contains(index) else { return }
        attachments.remove(at: index)
        updateAttachments()
        focus()
    }

    private func updateAttachments() {
        strip.configure(attachments)
        let width = field.bounds.width - expandedTextInset * 2
        stripTopConstraint.constant = attachments.isEmpty ? 0 : 10
        stripHeightConstraint.constant = attachments.isEmpty ? 0 : strip.heightThatFits(width: max(120, width))
        strip.isHidden = attachments.isEmpty
        updateButtons()
        updateLayout()
    }

    override func draggingEntered(_ sender: NSDraggingInfo) -> NSDragOperation {
        sender.draggingPasteboard.canReadObject(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) ? .copy : []
    }

    override func performDragOperation(_ sender: NSDraggingInfo) -> Bool {
        guard let urls = sender.draggingPasteboard.readObjects(forClasses: [NSURL.self], options: [.urlReadingFileURLsOnly: true]) as? [URL] else {
            return false
        }
        return addFiles(urls)
    }

    // MARK: - Dictation

    private func configureDictation() {
        dictation.onTranscript = { [weak self] transcript, _ in
            self?.dictationTranscript = transcript
        }
        dictation.onLevel = { [weak self] level in
            self?.pill.level = level
        }
        dictation.onEnd = { [weak self] failure in
            guard let self else { return }
            dictationTimer?.invalidate()
            dictationTimer = nil
            if let escapeMonitor { NSEvent.removeMonitor(escapeMonitor) }
            escapeMonitor = nil
            pill.isHidden = true
            scrollView.isHidden = false
            commitDictation()
            updateButtons()
            updateLayout()
            focus()
            if let failure {
                pendingSend = false
                report(failure)
            } else if pendingSend {
                pendingSend = false
                send()
            }
        }
    }

    @objc private func voice() {
        if dictation.isListening {
            dictation.stop()
            return
        }
        focus()
        // The words go where the caret is, after a space when they follow other text.
        let caret = textView.selectedRange()
        if caret.length > 0 {
            textView.insertText("", replacementRange: caret)
        }
        dictationLocation = textView.selectedRange().location
        dictationTranscript = ""
        pendingSend = false
        pill.reset()
        pill.isHidden = false
        scrollView.isHidden = true
        dictationTimer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            Task { @MainActor in
                guard let self, let startedAt = self.dictation.startedAt else { return }
                self.pill.setElapsed(Date().timeIntervalSince(startedAt))
            }
        }
        dictation.start()
        escapeMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            guard let self, event.keyCode == 53, event.window == window else { return event }
            cancelDictation()
            return nil
        }
        // Now that listening is on, Send takes the microphone's place beside the pill.
        updateButtons()
        updateLayout()
    }

    /// Drops the recording and its words.
    private func cancelDictation() {
        dictationTranscript = ""
        pendingSend = false
        dictation.cancel()
    }

    private func commitDictation() {
        let transcript = dictationTranscript.trimmingCharacters(in: .whitespacesAndNewlines)
        dictationTranscript = ""
        guard !transcript.isEmpty else { return }
        let text = textView.string as NSString
        let location = min(dictationLocation, text.length)
        var replacement = transcript
        if location > 0, let scalar = UnicodeScalar(text.character(at: location - 1)),
            !CharacterSet.whitespacesAndNewlines.contains(scalar)
        {
            replacement = " " + transcript
        }
        textView.insertText(replacement, replacementRange: NSRange(location: location, length: 0))
        handleTextChange()
    }

    private func report(_ failure: Dictation.Failure) {
        guard let window else { return }
        let alert = NSAlert()
        alert.messageText = L("Dictation could not start")
        alert.informativeText = failure.localizedDescription
        alert.addButton(withTitle: L("OK"))
        if failure.settingsPane != nil {
            alert.addButton(withTitle: L("Open System Settings"))
        }
        alert.beginSheetModal(for: window) { response in
            if response == .alertSecondButtonReturn, let pane = failure.settingsPane,
                let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?\(pane)")
            {
                NSWorkspace.shared.open(url)
            }
        }
    }

    @objc private func chooseLanguage(_ sender: NSMenuItem) {
        Preferences.dictationLanguage = sender.representedObject as? String
    }

    private func handle(_ selector: Selector) -> Bool {
        if mentions.isVisible {
            switch selector {
            case #selector(NSResponder.moveUp(_:)):
                mentions.moveSelection(by: -1)
                return true
            case #selector(NSResponder.moveDown(_:)):
                mentions.moveSelection(by: 1)
                return true
            case #selector(NSResponder.insertTab(_:)), #selector(NSResponder.insertNewline(_:)):
                mentions.commitSelection()
                return true
            case #selector(NSResponder.cancelOperation(_:)):
                mentions.dismiss()
                return true
            default:
                break
            }
        }

        if selector == #selector(NSResponder.cancelOperation(_:)), dictation.isListening {
            cancelDictation()
            return true
        }

        guard selector == #selector(NSResponder.insertNewline(_:)) else { return false }

        // Shift-Return always breaks the line; plain Return sends unless the
        // preference reserves sending for ⌘Return.
        let shift = NSApp.currentEvent?.modifierFlags.contains(.shift) ?? false
        if Preferences.sendOnReturn {
            if shift { return false }
            send()
            return true
        }
        let command = NSApp.currentEvent?.modifierFlags.contains(.command) ?? false
        if command {
            send()
            return true
        }
        return false
    }

    // MARK: - Layout

    override func layout() {
        super.layout()
        // Width changes can wrap a line that fit, or fit one that wrapped, and reflow the chips.
        if !attachments.isEmpty {
            let height = strip.heightThatFits(width: max(120, field.bounds.width - expandedTextInset * 2))
            if abs(height - stripHeightConstraint.constant) > 0.5 { stripHeightConstraint.constant = height }
        }
        updateLayout()
    }

    /// Text width available beside the controls in the pill.
    private var compactTextWidth: CGFloat {
        let visible = CGFloat(trailing.arrangedSubviews.filter { !$0.isHidden }.count)
        let trailingWidth = visible * controlSize + max(0, visible - 1) * controlSpacing
        let chrome = controlInset * 2 + controlSize + textInset * 2 + trailingWidth
        let containerPadding = (textView.textContainer?.lineFragmentPadding ?? 5) * 2
        return field.bounds.width - chrome - textView.textContainerInset.width * 2 - containerPadding
    }

    private func updateLayout() {
        guard let layoutManager = textView.layoutManager, let container = textView.textContainer
        else { return }
        layoutManager.ensureLayout(for: container)
        let used = layoutManager.usedRect(for: container)

        let wanted: Mode
        if !attachments.isEmpty || textView.string.contains("\n") || used.height > lineHeight * 1.5 {
            wanted = .expanded
        } else if mode == .expanded, used.width > compactTextWidth - 12 {
            // One line in the wide layout that would wrap beside the controls.
            wanted = .expanded
        } else {
            wanted = .compact
        }

        if wanted != mode {
            mode = wanted
            NSLayoutConstraint.deactivate(wanted == .compact ? expandedConstraints : compactConstraints)
            NSLayoutConstraint.activate(wanted == .compact ? compactConstraints : expandedConstraints)
            // Re-measure at the new width before sizing the text.
            layoutSubtreeIfNeeded()
            layoutManager.ensureLayout(for: container)
        }

        let textHeight = layoutManager.usedRect(for: container).height + textView.textContainerInset.height * 2
        let clamped = min(maxTextHeight, max(minTextHeight, ceil(textHeight)))
        if abs(clamped - heightConstraint.constant) > 0.5 {
            heightConstraint.constant = clamped
            scrollView.hasVerticalScroller = clamped >= maxTextHeight
        }
    }

    private func updateButtons() {
        let listening = dictation.isListening
        // While recording, Send commits the words. While a turn runs, another message steers
        // it, so Send remains available beside the separate hard Stop control.
        sendButton.isHidden = !hasContent && !listening
        stopButton.isHidden = !isResponding
        stopButton.style = sendButton.isHidden ? .primary : .secondary
        voiceButton.isHidden = listening
        // Dictation is the primary action only while nothing else is.
        voiceButton.style = sendButton.isHidden && stopButton.isHidden ? .primary : .plain
        let mentionHint = mentionableBots.count > 1 ? L(" · @ to mention") : ""
        sendButton.toolTip =
            Preferences.sendOnReturn
            ? L("Send (Return) · Shift-Return for a new line") + mentionHint
            : L("Send (⌘Return)") + mentionHint
    }

    // MARK: - Mentions

    /// Range of the `@token` the caret currently sits in, if any.
    private func mentionRange() -> NSRange? {
        let text = textView.string as NSString
        let caret = textView.selectedRange().location
        guard caret <= text.length, caret > 0 else { return nil }

        var index = caret - 1
        while index >= 0 {
            let character = text.character(at: index)
            let scalar = UnicodeScalar(character).map(Character.init)
            if scalar == "@" {
                let isStart = index == 0 || isBoundary(text.character(at: index - 1))
                guard isStart else { return nil }
                return NSRange(location: index, length: caret - index)
            }
            if let scalar, scalar == " " || scalar == "\n" { return nil }
            if caret - index > 24 { return nil }
            index -= 1
        }
        return nil
    }

    private func isBoundary(_ character: unichar) -> Bool {
        guard let scalar = UnicodeScalar(character) else { return true }
        return CharacterSet.whitespacesAndNewlines.contains(scalar)
    }

    private func updateMentions() {
        // Any chat with a bot completes `@`; a direct chat just offers its one bot.
        guard !mentionableBots.isEmpty, let range = mentionRange() else {
            mentions.dismiss()
            return
        }

        let text = textView.string as NSString
        let query = text.substring(with: range).dropFirst().lowercased()
        let matches = mentionableBots.filter {
            query.isEmpty || $0.name.lowercased().hasPrefix(query)
        }

        guard !matches.isEmpty else {
            mentions.dismiss()
            return
        }

        let caretRect = textView.firstRect(forCharacterRange: range, actualRange: nil)
        mentions.show(bots: matches, near: caretRect, relativeTo: textView)
    }

    private func insertMention(_ bot: Bot) {
        guard let range = mentionRange() else { return }
        textView.insertText("@\(bot.name) ", replacementRange: range)
        pickedMentions.append(bot)
        mentions.dismiss()
        handleTextChange()
    }

    /// Tints `@Name` runs so mentions read as addressed, not as typed punctuation.
    private func highlightMentions() {
        guard let storage = textView.textStorage else { return }
        let full = NSRange(location: 0, length: storage.length)
        storage.removeAttribute(.foregroundColor, range: full)
        storage.addAttribute(.foregroundColor, value: NSColor.labelColor, range: full)
        storage.addAttribute(.font, value: Theme.Font.message, range: full)

        let text = storage.string as NSString
        for bot in mentionableBots {
            var searchRange = NSRange(location: 0, length: text.length)
            while searchRange.location < text.length {
                let found = text.range(
                    of: "@\(bot.name)", options: [.caseInsensitive], range: searchRange)
                guard found.location != NSNotFound else { break }
                storage.addAttribute(.foregroundColor, value: bot.accent.color, range: found)
                storage.addAttribute(.font, value: Theme.Font.messageBold, range: found)
                let next = found.location + found.length
                searchRange = NSRange(location: next, length: max(0, text.length - next))
            }
        }
    }
}

extension ComposerView: NSTextViewDelegate {
    func textDidChange(_ notification: Notification) {
        handleTextChange()
    }

    private func handleTextChange() {
        highlightMentions()
        updateButtons()
        updateLayout()
        updateMentions()
        textView.needsDisplay = true
    }

    func textViewDidChangeSelection(_ notification: Notification) {
        updateMentions()
    }
}

extension ComposerView: NSMenuDelegate {
    /// The Dictate button's right-click menu: the language the recognizer listens in.
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let chosen = Preferences.dictationLanguage
        let automatic = NSMenuItem(
            title: L("Automatic (%@)", Dictation.displayName(Dictation.automaticLocale())), action: #selector(chooseLanguage(_:)), keyEquivalent: "")
        automatic.target = self
        automatic.state = chosen == nil ? .on : .off
        menu.addItem(automatic)
        menu.addItem(.separator())
        for locale in Dictation.supportedLocales {
            let item = NSMenuItem(title: Dictation.displayName(locale), action: #selector(chooseLanguage(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = locale.identifier
            item.state = chosen == locale.identifier ? .on : .off
            menu.addItem(item)
        }
    }
}

/// The recording state, after Grok Bot: a stop square, the elapsed time, and bars that follow
/// the microphone. The whole pill is the stop button.
final class RecordingPill: BackgroundView {
    var onStop: (() -> Void)?
    var level: Float = 0 {
        didSet { bars.level = level }
    }

    private let stop = NSImageView()
    private let elapsed = Build.label("0:00", font: .monospacedDigitSystemFont(ofSize: 13, weight: .regular))
    private let bars = LevelBarsView()
    private var isHovered = false {
        didSet { fillColor = Theme.composerControl.withAlphaComponent(isHovered ? 0.18 : 0.1) }
    }

    override init() {
        super.init()
        cornerRadius = 14
        fillColor = Theme.composerControl.withAlphaComponent(0.1)
        stop.image = Glyph.symbol("stop.fill", pointSize: 10, weight: .bold, color: .labelColor)
        stop.translatesAutoresizingMaskIntoConstraints = false
        toolTip = L("Stop recording")
        setAccessibilityElement(true)
        setAccessibilityRole(.button)
        setAccessibilityLabel(L("Stop recording"))
        elapsed.textColor = .labelColor
        elapsed.setContentCompressionResistancePriority(.required, for: .horizontal)
        let stack = Build.stack([stop, elapsed, bars], orientation: .horizontal, spacing: 6)
        addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            stack.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            stack.centerYAnchor.constraint(equalTo: centerYAnchor),
            stop.widthAnchor.constraint(equalToConstant: 22),
            stop.heightAnchor.constraint(equalToConstant: 22),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func reset() {
        level = 0
        bars.clear()
        elapsed.stringValue = "0:00"
    }

    func setElapsed(_ seconds: TimeInterval) {
        let whole = Int(seconds)
        elapsed.stringValue = String(format: "%d:%02d", whole / 60, whole % 60)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for area in trackingAreas { removeTrackingArea(area) }
        addTrackingArea(NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self))
    }

    override func mouseEntered(with event: NSEvent) { isHovered = true }
    override func mouseExited(with event: NSEvent) { isHovered = false }

    override func mouseDown(with event: NSEvent) {
        onStop?()
    }

    override func accessibilityPerformPress() -> Bool {
        onStop?()
        return true
    }
}

/// Five bars, dots at rest, that grow with the recent input levels.
final class LevelBarsView: NSView {
    private var history: [Float] = Array(repeating: 0, count: 5)

    var level: Float = 0 {
        didSet {
            history.removeFirst()
            history.append(level)
            needsDisplay = true
        }
    }

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize { NSSize(width: 24, height: 14) }

    func clear() {
        history = Array(repeating: 0, count: 5)
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        let width: CGFloat = 3
        let gap: CGFloat = 2.25
        NSColor.labelColor.withAlphaComponent(0.85).setFill()
        for (index, value) in history.enumerated() {
            let height = 3 + CGFloat(min(1, max(0, value))) * (bounds.height - 3)
            let x = CGFloat(index) * (width + gap)
            let rect = NSRect(x: x, y: (bounds.height - height) / 2, width: width, height: height)
            NSBezierPath(roundedRect: rect, xRadius: width / 2, yRadius: width / 2).fill()
        }
    }
}
