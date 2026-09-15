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

        // The sidebar buttons form a leading titlebar accessory, so they keep their place beside
        // the traffic lights when the sidebar collapses.
        let createButton = HoverButton(
            symbol: "plus", tooltip: "Create", target: nil, action: #selector(AppDelegate.newBot(_:)))
        createButton.menu = Self.createMenu()
        window.addTitlebarAccessoryViewController(
            Self.leadingAccessory([
                HoverButton(
                    symbol: "sidebar.leading", tooltip: "Toggle Sidebar (⌃⌘S)", target: root,
                    action: #selector(NSSplitViewController.toggleSidebar(_:))),
                createButton,
            ]))

        root.onSelectionChange = { [weak self] in self?.updateTitle() }
        updateTitle()
    }

    /// Lays square plain buttons out as a leading titlebar accessory, just past the traffic lights.
    private static func leadingAccessory(_ buttons: [HoverButton]) -> NSTitlebarAccessoryViewController {
        let inset: CGFloat = 8
        let spacing: CGFloat = 4
        let side = buttons.first?.intrinsicContentSize.width ?? 0
        let width = inset + CGFloat(buttons.count) * side + CGFloat(max(buttons.count - 1, 0)) * spacing
        // The accessory takes its width from the view's frame and fills the titlebar's height.
        let container = NSView(frame: NSRect(x: 0, y: 0, width: width, height: side))
        var previous: NSView?
        for button in buttons {
            button.translatesAutoresizingMaskIntoConstraints = false
            container.addSubview(button)
            NSLayoutConstraint.activate([
                button.leadingAnchor.constraint(
                    equalTo: previous?.trailingAnchor ?? container.leadingAnchor,
                    constant: previous == nil ? inset : spacing),
                button.centerYAnchor.constraint(equalTo: container.centerYAnchor),
                button.widthAnchor.constraint(equalToConstant: side),
                button.heightAnchor.constraint(equalToConstant: side),
            ])
            previous = button
        }
        let accessory = NSTitlebarAccessoryViewController()
        accessory.view = container
        accessory.layoutAttribute = .leading
        return accessory
    }

    /// A new bot opens its own direct chat, so the menu offers only bots and groups. Items go
    /// through the responder chain to the same actions as the File menu.
    private static func createMenu() -> NSMenu {
        let menu = NSMenu()
        menu.addItem(withTitle: "Create New Bot…", action: #selector(AppDelegate.newBot(_:)), keyEquivalent: "")
        menu.addItem(
            withTitle: "Create Group Chat…", action: #selector(AppDelegate.newGroupChat(_:)), keyEquivalent: "")
        return menu
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
        case let .device(id):
            guard let device = AppStore.shared.device(id) else { return }
            window.title = device.name
            window.subtitle = device.model
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
    static let inspectorToggle = NSToolbarItem.Identifier("tinybot.inspectorToggle")
}

// Standard toolbar items sit on glass platters; a borderless custom-view item doesn't. AppKit moves
// the inspector toggle between the inspector's section and the window's trailing edge as the
// inspector opens and closes.
extension MainWindowController: NSToolbarDelegate {
    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [
            .sidebarTrackingSeparator,
            .flexibleSpace,
            .inspectorTrackingSeparator,
            .flexibleSpace,
            .inspectorToggle,
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
        guard identifier == .inspectorToggle else { return nil }
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.label = "Inspector"
        item.view = HoverButton(
            symbol: "sidebar.trailing", tooltip: "Toggle Inspector (⌥⌘I)", target: root,
            action: #selector(RootSplitViewController.toggleInspector(_:)))
        item.isBordered = false
        return item
    }
}
