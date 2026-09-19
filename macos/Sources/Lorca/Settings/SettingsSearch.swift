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
/// setting, those settings under it, and the Devices that match.
@MainActor
enum SettingsSearch {
    struct PaneResult {
        let pane: SettingsPane
        let entries: [SettingsEntry]
    }

    static func entries(in pane: SettingsPane, store: AppStore) -> [SettingsEntry] {
        switch pane {
        case .general: [.sendOnReturn, .timestamps, .appearance, .dictationLanguage]
        case .providers: (store.thisDevice?.providers ?? []).map { .provider($0.kind) }
        case .autoReview: [.autoReviewSwitch, .autoReviewRules]
        case .advanced: [.relayURL, .cliPort, .onboarding]
        }
    }

    static func panes(matching query: String, store: AppStore) -> [PaneResult] {
        SettingsPane.allCases.compactMap { pane in
            let entries = entries(in: pane, store: store).filter { $0.matches(query) }
            guard !entries.isEmpty || pane.title.localizedStandardContains(query) else { return nil }
            return PaneResult(pane: pane, entries: entries)
        }
    }

    static func devices(matching query: String, store: AppStore) -> [Device] {
        store.devices.filter { device in
            [device.name, device.model, device.os.displayName, device.isThisDevice ? "This Mac" : ""]
                .contains { $0.localizedStandardContains(query) }
        }
    }
}
