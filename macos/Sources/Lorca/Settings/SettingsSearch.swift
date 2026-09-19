import AppKit

/// One searchable setting. The panes label their rows from these, so the search index and the
/// pages share one spelling.
struct SettingsEntry: Hashable {
    let pane: SettingsPane
    /// What the results list shows.
    let title: String
    /// The row's label on the pane, or a section's title. The pane finds the row by it.
    let row: String
    /// Words that find the setting besides its title.
    let keywords: [String]

    init(_ pane: SettingsPane, _ title: String, row: String? = nil, keywords: [String] = []) {
        self.pane = pane
        self.title = title
        self.row = row ?? title
        self.keywords = keywords
    }

    static let sendOnReturn = SettingsEntry(
        .general, "Return sends the message", keywords: ["enter", "send", "newline", "keyboard", "chats"])
    static let timestamps = SettingsEntry(
        .general, "Show timestamps in transcripts", keywords: ["time", "date", "messages", "chats"])
    static let appearance = SettingsEntry(
        .general, "Appearance", keywords: ["theme", "dark mode", "light mode", "system"])
    static let dictationLanguage = SettingsEntry(
        .general, "Dictation Language", row: "Language",
        keywords: ["dictate", "speech", "voice", "microphone", "locale"])
    static let version = SettingsEntry(
        .general, "Check for Updates", row: "Version", keywords: ["update", "upgrade", "release", "sparkle"])
    static let automaticChecks = SettingsEntry(
        .general, "Check for updates automatically", keywords: ["update", "upgrade", "background"])
    static let automaticDownloads = SettingsEntry(
        .general, "Download and install updates automatically", keywords: ["update", "upgrade", "background"])

    static let autoReviewSwitch = SettingsEntry(
        .autoReview, "Check actions before they run",
        keywords: ["auto-review", "approve", "approval", "permission", "plugin", "ask"])
    static let autoReviewRules = SettingsEntry(
        .autoReview, "Auto-review Rules", keywords: ["rule", "allow automatically", "ask first", "always allow"])

    static let relayURL = SettingsEntry(
        .advanced, "Relay URL", keywords: ["server", "self-host", "sync", "pairing", "connection"])
    static let cliPort = SettingsEntry(
        .advanced, "CLI port", keywords: ["localhost", "127.0.0.1", "serve", "connection"])
    static let onboarding = SettingsEntry(
        .advanced, "Onboarding", keywords: ["show onboarding again", "setup", "welcome", "restore"])

    static let machineKey = SettingsEntry(
        .device, "Machine key", keywords: ["device", "os", "role", "runner", "last seen", "relay"])
    static let pairing = SettingsEntry(.device, "Pairing", keywords: ["unpair", "remove device", "paired"])

    static func bot(_ bot: Bot) -> SettingsEntry {
        SettingsEntry(.bots, bot.name, keywords: [bot.label, bot.provider.rawValue, "bot", "runner"])
    }

    static func plugin(_ plugin: InstalledPlugin) -> SettingsEntry {
        SettingsEntry(.plugins, plugin.name, keywords: [plugin.description, "plugin", "mcp", "marketplace"])
    }

    static func provider(_ kind: ProviderCredential.Kind) -> SettingsEntry {
        SettingsEntry(
            .providers, kind.rawValue,
            keywords: [kind.subtitle, "credential", "connect", "disconnect", "sign in", "model"])
    }

    func matches(_ query: String) -> Bool {
        ([title] + keywords).contains { $0.localizedStandardContains(query) }
    }
}

/// What the settings sidebar lists for a query: each pane that matches by name or holds a matching
/// setting, and those settings under it. The Device panes answer for the picked Device.
@MainActor
enum SettingsSearch {
    struct PaneResult {
        let pane: SettingsPane
        let entries: [SettingsEntry]
    }

    static func entries(in pane: SettingsPane, device: Device?, store: AppStore) -> [SettingsEntry] {
        switch pane {
        case .general:
            [.sendOnReturn, .timestamps, .appearance, .dictationLanguage]
                + (Updater.isEnabled ? [.version, .automaticChecks, .automaticDownloads] : [])
        case .autoReview: [.autoReviewSwitch, .autoReviewRules]
        case .advanced: [.relayURL, .cliPort, .onboarding]
        case .bots: (device.map { store.bots(on: $0.id) } ?? []).map { .bot($0) }
        case .providers: store.providers.map { .provider($0.kind) }
        case .plugins: (device?.plugins ?? []).map { .plugin($0) }
        case .device: device?.isThisDevice == false ? [.machineKey, .pairing] : [.machineKey]
        }
    }

    static func panes(matching query: String, device: Device?, store: AppStore) -> [PaneResult] {
        SettingsPane.allCases.compactMap { pane in
            let entries = entries(in: pane, device: device, store: store).filter { $0.matches(query) }
            guard !entries.isEmpty || pane.title.localizedStandardContains(query) else { return nil }
            return PaneResult(pane: pane, entries: entries)
        }
    }
}
