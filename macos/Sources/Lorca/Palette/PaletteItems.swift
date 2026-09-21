import AppKit

/// One row of the command palette.
struct PaletteItem {
    enum Icon {
        case symbol(String)
        case chat([Bot])
    }

    let icon: Icon
    let title: String
    var subtitle = ""
    /// The menu item's key equivalent, as the menu bar spells it.
    var shortcut = ""
    /// Words that find the item besides its title.
    var keywords: [String] = []
    /// Left out of the list until a query finds it.
    var isSearchOnly = false
    var chatID: Chat.ID? = nil
    let run: () -> Void
}

struct PaletteSection {
    let title: String
    var items: [PaletteItem]
}

/// What the palette lists: the menu bar's commands, the chats, and the settings.
@MainActor
enum PaletteIndex {
    /// The menu bar's commands worth offering, by action. The palette takes each one's title,
    /// key equivalent, and enabled state from its menu item, and runs it by sending the item's
    /// action, so a command reads and behaves as it does in the menu.
    private static let commands: [(action: Selector, symbol: String, keywords: String)] = [
        (#selector(AppDelegate.newBot(_:)), "plus.message", "create add"),
        (#selector(AppDelegate.newGroupChat(_:)), "person.2", "create room"),
        (#selector(AppDelegate.pairDevice(_:)), "qrcode", "phone link runner"),
        (#selector(RootSplitViewController.addBotToChat(_:)), "person.badge.plus", "invite member group"),
        (#selector(RootSplitViewController.renameChat(_:)), "pencil", "title name"),
        (#selector(RootSplitViewController.togglePinChat(_:)), "pin", "unpin favorite"),
        (#selector(ChatViewController.stopResponding(_:)), "stop.circle", "cancel interrupt"),
        (#selector(ChatViewController.scrollToLatest(_:)), "arrow.down.to.line", "bottom newest jump"),
        (#selector(NSSplitViewController.toggleSidebar(_:)), "sidebar.leading", "show hide"),
        (#selector(RootSplitViewController.toggleInspector(_:)), "sidebar.trailing", "show hide details"),
        (#selector(NSWindow.toggleFullScreen(_:)), "arrow.up.left.and.arrow.down.right", "fullscreen"),
        (#selector(AppDelegate.showSettings(_:)), "gearshape", "preferences"),
        (#selector(AppDelegate.checkForUpdates(_:)), "arrow.triangle.2.circlepath", "upgrade version"),
        (#selector(RootSplitViewController.deleteChat(_:)), "trash", "remove"),
        (#selector(AppDelegate.showHelp(_:)), "questionmark.circle", "about"),
    ]

    /// Read while the main window is still key, so the menu validates against its responder chain.
    static func sections(root: RootSplitViewController, store: AppStore) -> [PaletteSection] {
        [
            PaletteSection(title: L("Actions"), items: actions()),
            PaletteSection(title: L("Chats"), items: chats(root: root, store: store)),
            PaletteSection(title: L("Settings"), items: settings(root: root, store: store)),
        ]
    }

    private static func actions() -> [PaletteItem] {
        guard let menu = NSApp.mainMenu else { return [] }
        for item in menu.items { item.submenu?.update() }
        return commands.compactMap { command in
            guard let item = find(command.action, in: menu), item.isEnabled else { return nil }
            return PaletteItem(
                icon: .symbol(command.symbol), title: item.title, shortcut: shortcut(of: item),
                keywords: [command.keywords]
            ) {
                NSApp.sendAction(command.action, to: item.target, from: item)
            }
        }
    }

    private static func chats(root: RootSplitViewController, store: AppStore) -> [PaletteItem] {
        guard store.isConnected else { return [] }
        return store.chats.map { chat in
            let bots = store.bots(in: chat)
            return PaletteItem(
                icon: .chat(bots), title: store.title(for: chat), subtitle: store.subtitle(for: chat),
                keywords: bots.map(\.name) + [store.preview(for: chat)], chatID: chat.id
            ) { [weak root] in
                root?.open(chat.id)
            }
        }
    }

    /// The panes, then the settings on them, which only a query brings up.
    private static func settings(root: RootSplitViewController, store: AppStore) -> [PaletteItem] {
        let device = root.settingsDeviceID.flatMap { store.device($0) }
        let panes = SettingsPane.allCases.map { pane in
            PaletteItem(icon: .symbol(pane.symbolName), title: pane.title, keywords: [L("Settings")]) { [weak root] in
                root?.showSettings(pane)
            }
        }
        let entries = SettingsPane.allCases.flatMap { pane in
            SettingsSearch.entries(in: pane, device: device, store: store).map { entry in
                PaletteItem(
                    icon: .symbol(pane.symbolName), title: entry.title, subtitle: pane.title,
                    keywords: entry.keywords, isSearchOnly: true
                ) { [weak root] in
                    root?.reveal(entry)
                }
            }
        }
        return panes + entries
    }

    static func searchSections(
        _ results: Wire.SearchResults, root: RootSplitViewController, store: AppStore
    ) -> [PaletteSection] {
        let chats = results.chats.compactMap { hit -> PaletteItem? in
            guard let chat = store.chat(hit.chatId) else { return nil }
            return PaletteItem(
                icon: .chat(store.bots(in: chat)), title: store.title(for: chat), subtitle: hit.snippet,
                isSearchOnly: true, chatID: chat.id
            ) { [weak root] in
                root?.open(chat.id)
            }
        }
        let messages = results.messages.compactMap { hit -> PaletteItem? in
            guard let chat = store.chat(hit.chatId) else { return nil }
            return PaletteItem(
                icon: .chat(store.bots(in: chat)), title: store.title(for: chat), subtitle: hit.snippet,
                isSearchOnly: true, chatID: chat.id
            ) { [weak root] in
                root?.open(chat.id)
            }
        }
        return [
            PaletteSection(title: L("Chats"), items: chats),
            PaletteSection(title: L("Messages"), items: messages),
        ].filter { !$0.items.isEmpty }
    }

    // MARK: - Menu items

    private static func find(_ action: Selector, in menu: NSMenu) -> NSMenuItem? {
        for item in menu.items {
            if item.action == action, !item.isHidden { return item }
            if let found = item.submenu.flatMap({ find(action, in: $0) }) { return found }
        }
        return nil
    }

    private static func shortcut(of item: NSMenuItem) -> String {
        let key = item.keyEquivalent
        guard !key.isEmpty else { return "" }
        let modifiers = item.keyEquivalentModifierMask
        var text = ""
        if modifiers.contains(.control) { text += "⌃" }
        if modifiers.contains(.option) { text += "⌥" }
        if modifiers.contains(.shift) { text += "⇧" }
        if modifiers.contains(.command) { text += "⌘" }
        switch key {
        case "\u{8}", "\u{7F}": text += "⌫"
        case "\r": text += "↩"
        case " ": text += "␣"
        default: text += key.uppercased()
        }
        return text
    }

    // MARK: - Search

    /// The sections narrowed to a query, best match first. With no query, everything but the
    /// items only a search brings up.
    static func filter(_ sections: [PaletteSection], query: String) -> [PaletteSection] {
        let query = fold(query.trimmingCharacters(in: .whitespaces))
        return sections.compactMap { section in
            var section = section
            if query.isEmpty {
                section.items = section.items.filter { !$0.isSearchOnly }
            } else {
                section.items = section.items
                    .compactMap { item in score(item, query).map { (item, $0) } }
                    .sorted { $0.1 > $1.1 }
                    .map(\.0)
            }
            return section.items.isEmpty ? nil : section
        }
    }

    private static func fold(_ text: String) -> String {
        text.folding(options: [.caseInsensitive, .diacriticInsensitive, .widthInsensitive], locale: .current)
    }

    /// A title that starts with the query beats a word that does, then a substring, then the
    /// query's letters in order ("ngc" finds New Group Chat). Keywords and subtitles rank last.
    private static func score(_ item: PaletteItem, _ query: String) -> Int? {
        let title = fold(item.title)
        if title.hasPrefix(query) { return 100 }
        let words = title.split { !$0.isLetter && !$0.isNumber }
        if words.contains(where: { $0.hasPrefix(query) }) { return 80 }
        if title.contains(query) { return 60 }
        if isSubsequence(query, of: String(words.compactMap(\.first))) { return 50 }
        if ([item.subtitle] + item.keywords).contains(where: { fold($0).contains(query) }) { return 40 }
        if query.count > 1, isSubsequence(query, of: title) { return 20 }
        return nil
    }

    private static func isSubsequence(_ needle: String, of haystack: String) -> Bool {
        var rest = haystack[...]
        for character in needle {
            guard let index = rest.firstIndex(of: character) else { return false }
            rest = rest[rest.index(after: index)...]
        }
        return true
    }
}
