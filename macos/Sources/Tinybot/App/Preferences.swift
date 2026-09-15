import Foundation

enum Preferences {
    private enum Key {
        static let onboarded = "tinybot.onboarded"
        static let selection = "tinybot.selection"
        static let sendOnReturn = "tinybot.sendOnReturn"
        static let relayURL = "tinybot.relayURL"
        static let cliPort = "tinybot.cliPort"
        static let showTimestamps = "tinybot.showTimestamps"
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

    static var relayURL: String {
        get { defaults.string(forKey: Key.relayURL) ?? "https://tinybot.dev" }
        set { defaults.set(newValue, forKey: Key.relayURL) }
    }

    static var cliPort: Int {
        get {
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
