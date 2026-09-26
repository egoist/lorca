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

    static var sendOnReturn: SettingsEntry { SettingsEntry(
        .general, L("Return sends the message"), keywords: [L("enter send newline keyboard chats")]) }
    static var timestamps: SettingsEntry { SettingsEntry(
        .general, L("Show timestamps in transcripts"), keywords: [L("time date messages chats")]) }
    static var appearance: SettingsEntry { SettingsEntry(
        .general, L("Appearance"), keywords: [L("theme dark mode light mode system")]) }
    static var appLanguage: SettingsEntry { SettingsEntry(
        .general, L("App Language"), keywords: [L("language locale english chinese translation")]) }
    static var dictationLanguage: SettingsEntry { SettingsEntry(
        .general, L("Dictation Language"), row: L("Language"),
        keywords: [L("dictate speech voice microphone locale")]) }
    static var version: SettingsEntry { SettingsEntry(
        .general, L("Check for Updates"), row: L("Version"), keywords: [L("update upgrade release sparkle")]) }
    static var automaticChecks: SettingsEntry { SettingsEntry(
        .general, L("Check for updates automatically"), keywords: [L("update upgrade background")]) }
    static var automaticDownloads: SettingsEntry { SettingsEntry(
        .general, L("Download and install updates automatically"), keywords: [L("update upgrade background")]) }

    static var autoReviewSwitch: SettingsEntry { SettingsEntry(
        .autoReview, L("Check actions before they run"),
        keywords: [L("auto-review approve approval permission plugin ask")]) }
    static var autoReviewRules: SettingsEntry { SettingsEntry(
        .autoReview, L("Auto-review Rules"), keywords: [L("rule allow automatically ask first always allow")]) }

    static var relayURL: SettingsEntry { SettingsEntry(
        .advanced, L("Relay URL"), keywords: [L("server self-host sync pairing connection")]) }
    static var cliPort: SettingsEntry { SettingsEntry(
        .advanced, L("CLI port"), keywords: [L("localhost 127.0.0.1 serve connection")]) }
    static var onboarding: SettingsEntry { SettingsEntry(
        .advanced, L("Onboarding"), keywords: [L("show onboarding again setup welcome restore")]) }
    static var deleteAccount: SettingsEntry { SettingsEntry(
        .advanced, L("Delete Account"), row: L("Account"), keywords: [L("erase remove wipe relay data identity")]) }

    static var machineKey: SettingsEntry { SettingsEntry(
        .device, L("Machine key"), keywords: [L("device os role runner last seen relay")]) }
    static var pairing: SettingsEntry { SettingsEntry(.device, L("Pairing"), keywords: [L("unpair remove device paired")]) }

    static func bot(_ bot: Bot) -> SettingsEntry {
        SettingsEntry(.bots, bot.name, keywords: [bot.description, bot.runtimeLabel, L("bot runner")])
    }

    static func plugin(_ plugin: InstalledPlugin) -> SettingsEntry {
        SettingsEntry(.plugins, plugin.name, keywords: [plugin.description, L("plugin mcp marketplace")])
    }

    static func provider(_ kind: ProviderCredential.Kind) -> SettingsEntry {
        SettingsEntry(
            .providers, kind.rawValue,
            keywords: [kind.subtitle, L("credential connect disconnect sign in model")])
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
            [.sendOnReturn, .timestamps, .appearance, .appLanguage, .dictationLanguage]
                + (Updater.isEnabled ? [.version, .automaticChecks, .automaticDownloads] : [])
        case .autoReview: [.autoReviewSwitch, .autoReviewRules]
        case .advanced: [.relayURL, .cliPort, .onboarding] + (store.hasIdentity == true ? [.deleteAccount] : [])
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
