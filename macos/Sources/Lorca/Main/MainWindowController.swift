import AppKit

final class MainWindowController: NSWindowController, NSWindowDelegate {
    let root = RootSplitViewController()
    /// The Device the Providers, Plugins, Bots, and Devices panes show. Its toolbar item is in the
    /// toolbar only while one of those panes is up.
    private let devicePicker = NSPopUpButton()
    /// Creating bots and chats belongs to the chats; Settings hides it.
    private var createButton: HoverButton?

    init() {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1180, height: 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        window.title = "Lorca"
        window.titleVisibility = .visible
        window.toolbarStyle = .unified
        window.titlebarSeparatorStyle = .automatic
        window.minSize = NSSize(width: 860, height: 520)
        window.contentViewController = root
        window.setFrameAutosaveName("LorcaMainWindow")
        window.tabbingMode = .disallowed

        super.init(window: window)
        window.delegate = self

        let toolbar = NSToolbar(identifier: "LorcaToolbar")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        window.toolbar = toolbar

        // The sidebar buttons form a leading titlebar accessory, so they keep their place beside
        // the traffic lights when the sidebar collapses.
        let createButton = HoverButton(
            symbol: "plus", tooltip: "Create", target: nil, action: #selector(AppDelegate.newBot(_:)))
        createButton.menu = Self.createMenu()
        self.createButton = createButton
        window.addTitlebarAccessoryViewController(
            Self.leadingAccessory([
                HoverButton(
                    symbol: "sidebar.leading", tooltip: "Toggle Sidebar (⌃⌘S)", target: root,
                    action: #selector(NSSplitViewController.toggleSidebar(_:))),
                createButton,
            ]))

        devicePicker.target = self
        devicePicker.action = #selector(pickDevice)
        devicePicker.setAccessibilityLabel("Device")
        devicePicker.toolTip = "The Device this page shows"

        root.onSelectionChange = { [weak self] in
            self?.updateTitle()
            self?.updateDevicePicker()
            Notifier.shared.watchingChanged()
        }
        AppStore.shared.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced: self?.updateDevicePicker()
            default: break
            }
        }
        updateTitle()
        updateDevicePicker()
    }

    // MARK: - Device picker

    private func updateDevicePicker() {
        guard let toolbar = window?.toolbar else { return }
        var isScoped = false
        if case let .settings(pane) = root.selection { isScoped = pane.isDeviceScoped }

        // Settings has no inspector and nothing to create.
        let isSettings = root.selection?.isSettings == true
        createButton?.isHidden = isSettings
        let toggle = toolbar.items.firstIndex { $0.itemIdentifier == .inspectorToggle }
        if isSettings, let toggle {
            toolbar.removeItem(at: toggle)
        } else if !isSettings, toggle == nil {
            toolbar.insertItem(withItemIdentifier: .inspectorToggle, at: toolbar.items.count)
        }

        let index = toolbar.items.firstIndex { $0.itemIdentifier == .devicePicker }
        if isScoped, index == nil {
            // At the content area's trailing edge, ahead of the inspector's section.
            let at = toolbar.items.firstIndex { $0.itemIdentifier == .inspectorTrackingSeparator } ?? toolbar.items.count
            toolbar.insertItem(withItemIdentifier: .devicePicker, at: at)
        } else if !isScoped, let index {
            toolbar.removeItem(at: index)
        }
        guard isScoped else { return }

        devicePicker.removeAllItems()
        for device in AppStore.shared.devices {
            devicePicker.addItem(withTitle: "")
            guard let item = devicePicker.lastItem else { continue }
            item.title = device.isThisDevice ? "\(device.name) (This Mac)" : device.name
            item.representedObject = device.id
            item.image = NSImage(systemSymbolName: device.symbolName, accessibilityDescription: nil)
        }
        selectPickedDevice()
        devicePicker.menu?.addItem(.separator())
        let pair = NSMenuItem(title: "Pair a Device…", action: #selector(pairDevice), keyEquivalent: "")
        pair.target = self
        devicePicker.menu?.addItem(pair)
        devicePicker.sizeToFit()
    }

    private func selectPickedDevice() {
        let picked = root.settingsDeviceID
        if let item = devicePicker.itemArray.first(where: { $0.representedObject as? String == picked }) {
            devicePicker.select(item)
        }
    }

    @objc private func pickDevice() {
        guard let id = devicePicker.selectedItem?.representedObject as? String else { return }
        root.showSettingsDevice(id)
    }

    /// Pairing is a command, so the pop-up goes back to showing the picked Device.
    @objc private func pairDevice() {
        selectPickedDevice()
        NSApp.sendAction(#selector(AppDelegate.pairDevice(_:)), to: nil, from: nil)
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

    /// A window coming on screen starts with the keyboard in its content, whatever held it when
    /// the window closed and whichever key view AppKit would pick for a new one.
    override func showWindow(_ sender: Any?) {
        let isOpening = window?.isVisible != true
        super.showWindow(sender)
        if isOpening { root.focusContent() }
    }

    private func updateTitle() {
        guard let window else { return }
        switch root.selection {
        case let .chat(id):
            guard let chat = AppStore.shared.chat(id) else { return }
            window.title = AppStore.shared.title(for: chat)
            window.subtitle = AppStore.shared.subtitle(for: chat)
        case let .settings(pane):
            window.title = pane.title
            window.subtitle = ""
        case nil:
            window.title = "Lorca"
            window.subtitle = ""
        }
    }

    func windowDidBecomeKey(_ notification: Notification) {
        root.windowBecameKey()
        Notifier.shared.watchingChanged()
    }

    func windowDidMiniaturize(_ notification: Notification) {
        Notifier.shared.watchingChanged()
    }

    func windowWillClose(_ notification: Notification) {
        // The window still counts as visible here; report once it has gone.
        DispatchQueue.main.async { Notifier.shared.watchingChanged() }
    }
}

// MARK: - Toolbar

extension NSToolbarItem.Identifier {
    static let inspectorToggle = NSToolbarItem.Identifier("lorca.inspectorToggle")
    static let devicePicker = NSToolbarItem.Identifier("lorca.devicePicker")
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
        toolbarDefaultItemIdentifiers(toolbar) + [.devicePicker]
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        if identifier == .devicePicker {
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.label = "Device"
            item.view = devicePicker
            return item
        }
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
