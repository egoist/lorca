import Foundation

enum Preferences {
    private enum Key {
        static let onboarded = "lorca.onboarded"
        static let selection = "lorca.selection"
        static let sendOnReturn = "lorca.sendOnReturn"
        static let relayURL = "lorca.relayURL"
        static let cliPort = "lorca.cliPort"
        static let showTimestamps = "lorca.showTimestamps"
        static let dictationLanguage = "lorca.dictationLanguage"
    }

    private static let defaults = UserDefaults.standard

    static var hasOnboarded: Bool {
        get { defaults.bool(forKey: Key.onboarded) }
        set { defaults.set(newValue, forKey: Key.onboarded) }
    }

    /// Persisted so a dev-mode relaunch lands back on the same conversation.
    static var selection: String? {
        get { defaults.string(forKey: Key.selection) }
        set { defaults.set(newValue, forKey: Key.selection) }
    }

    static var sendOnReturn: Bool {
        get { defaults.object(forKey: Key.sendOnReturn) as? Bool ?? true }
        set { defaults.set(newValue, forKey: Key.sendOnReturn) }
    }

    static var showTimestamps: Bool {
        get { defaults.object(forKey: Key.showTimestamps) as? Bool ?? true }
        set { defaults.set(newValue, forKey: Key.showTimestamps) }
    }

    /// Speech recognizer locale identifier; nil follows the system's preferred languages.
    static var dictationLanguage: String? {
        get { defaults.string(forKey: Key.dictationLanguage) }
        set { defaults.set(newValue, forKey: Key.dictationLanguage) }
    }

    /// The language the app's own words are in ("en", "zh-Hans"); nil follows the system. It is
    /// the app's `AppleLanguages` default, the one System Settings › Language & Region writes
    /// per app, so either place changes it and the other shows it. Read at launch.
    static var appLanguage: String? {
        get {
            let domain = Bundle.main.bundleIdentifier.flatMap { defaults.persistentDomain(forName: $0) }
            return (domain?["AppleLanguages"] as? [String])?.first
        }
        set {
            if let newValue { defaults.set([newValue], forKey: "AppleLanguages") } else { defaults.removeObject(forKey: "AppleLanguages") }
        }
    }

    static var relayURL: String {
        get { defaults.string(forKey: Key.relayURL) ?? "" }
        set { defaults.set(newValue, forKey: Key.relayURL) }
    }

    /// `LORCA_PORT` wins, so a second app instance can run against its own CLI.
    static var cliPort: Int {
        get {
            if let raw = ProcessInfo.processInfo.environment["LORCA_PORT"], let port = Int(raw), port > 0 {
                return port
            }
            let stored = defaults.integer(forKey: Key.cliPort)
            return stored == 0 ? AppInfo.defaultCLIPort : stored
        }
        set { defaults.set(newValue, forKey: Key.cliPort) }
    }

    static func reset() {
        for key in [Key.onboarded, Key.selection] {
            defaults.removeObject(forKey: key)
        }
    }
}
