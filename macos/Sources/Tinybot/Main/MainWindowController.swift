import AppKit

final class MainWindowController: NSWindowController, NSWindowDelegate {
    let root = RootSplitViewController()

    init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1180, height: 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "Tinybot"
        window.titleVisibility = .visible
        window.toolbarStyle = .unified
        window.titlebarSeparatorStyle = .automatic
        window.minSize = NSSize(width: 860, height: 520)
        window.contentViewController = root
        window.setFrameAutosaveName("TinybotMainWindow")
        window.tabbingMode = .disallowed

        super.init(window: window)
        window.delegate = self

        let toolbar = NSToolbar(identifier: "TinybotToolbar")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        window.toolbar = toolbar

        root.onSelectionChange = { [weak self] in self?.updateTitle() }
        updateTitle()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func focusSearch() {
        root.focusSearch()
    }

    private func updateTitle() {
        guard let window else { return }
        switch root.selection {
        case let .chat(id):
            guard let chat = AppStore.shared.chat(id) else { return }
            window.title = AppStore.shared.title(for: chat)
            window.subtitle = AppStore.shared.subtitle(for: chat)
        case let .computer(id):
            guard let computer = AppStore.shared.computer(id) else { return }
            window.title = computer.name
            window.subtitle = computer.model
        case nil:
            window.title = "Tinybot"
            window.subtitle = ""
        }
    }

    func windowDidBecomeKey(_ notification: Notification) {
        root.windowBecameKey()
    }
}

// MARK: - Toolbar

extension NSToolbarItem.Identifier {
    static let newChat = NSToolbarItem.Identifier("tinybot.newChat")
    static let sidebar = NSToolbarItem.Identifier("tinybot.sidebar")
}

extension MainWindowController: NSToolbarDelegate {
    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [
            .flexibleSpace,
            .newChat,
            .sidebar,
            .sidebarTrackingSeparator,
            .flexibleSpace,
            .inspectorTrackingSeparator,
            .toggleInspector,
        ]
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(toolbar)
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        switch identifier {
        case .newChat:
            return borderedItem(
                identifier,
                symbol: "square.and.pencil",
                label: "New Chat",
                tooltip: "New Chat (⌘N)",
                action: #selector(newChatFromToolbar(_:))
            )
        case .sidebar:
            return borderedItem(
                identifier,
                symbol: "sidebar.leading",
                label: "Hide Sidebar",
                tooltip: "Hide or show the sidebar",
                action: #selector(toggleSidebarFromToolbar(_:))
            )
        default:
            return nil
        }
    }

    private func borderedItem(
        _ identifier: NSToolbarItem.Identifier,
        symbol: String,
        label: String,
        tooltip: String,
        action: Selector
    ) -> NSToolbarItem {
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.label = label
        item.paletteLabel = label
        item.toolTip = tooltip
        item.image = NSImage(systemSymbolName: symbol, accessibilityDescription: label)
        item.isBordered = true
        item.target = self
        item.action = action
        return item
    }

    @objc private func newChatFromToolbar(_ sender: Any?) {
        root.presentNewChat()
    }

    @objc private func toggleSidebarFromToolbar(_ sender: Any?) {
        root.toggleSidebar(sender)
    }
}
