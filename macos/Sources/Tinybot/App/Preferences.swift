import Foundation

enum Preferences {
    private enum Key {
        static let onboarded = "tinybot.onboarded"
        static let selection = "tinybot.selection"
        static let sendOnReturn = "tinybot.sendOnReturn"
        static let relayURL = "tinybot.relayURL"
        static let cliPort = "tinybot.cliPort"
        static let showTimestamps = "tinybot.showTimestamps"
        static let dictationLanguage = "tinybot.dictationLanguage"
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

    static var relayURL: String {
        get { defaults.string(forKey: Key.relayURL) ?? "https://tinybot.dev" }
        set { defaults.set(newValue, forKey: Key.relayURL) }
    }

    /// `TINYBOT_PORT` wins, so a second app instance can run against its own CLI.
    static var cliPort: Int {
        get {
            if let raw = ProcessInfo.processInfo.environment["TINYBOT_PORT"], let port = Int(raw), port > 0 {
                return port
            }
            let stored = defaults.integer(forKey: Key.cliPort)
            return stored == 0 ? 4862 : stored
        }
        set { defaults.set(newValue, forKey: Key.cliPort) }
    }

    static func reset() {
        for key in [Key.onboarded, Key.selection] {
            defaults.removeObject(forKey: key)
        }
    }
}
