import AppKit

final class MainWindowController: NSWindowController, NSWindowDelegate {
    let root = RootSplitViewController()
    /// The Device the Plugins, Bots, and Devices panes show. Its toolbar item is in the
    /// toolbar only while one of those panes is up.
    private lazy var devicePicker: NSPopUpButton = {
        let picker = NSPopUpButton()
        picker.target = self
        picker.action = #selector(pickDevice)
        return picker
    }()
    /// The sidebar and create buttons beside the traffic lights.
    private var titlebarButtons: NSTitlebarAccessoryViewController?
    /// Creating bots and chats belongs to the chats; Settings hides it.
    private var createButton: HoverButton?
    private weak var navigation: NSToolbarItemGroup?
    /// The picker's glass capsule, where the pop-up has one of its own.
    private var devicePlatter: NSView?
    private var palette: CommandPalette?
    /// The chat's running commands: shown while it has any, or while their popover is open, with
    /// how many as the item's badge on the symbol's corner once there is more than one.
    private lazy var tasksButton = HoverButton(
        symbol: "terminal", tooltip: L("Running tasks"), target: self, action: #selector(toggleRunningTasks(_:)))
    private weak var tasksItem: NSToolbarItem?
    /// The count the item's badge shows; 0 is none.
    private var tasksBadge = 0
    private var tasksPopover: NSPopover?
    /// When the popover last closed. A click on the button closes it before the button acts.
    private var tasksClosedAt = Date.distantPast

    init() {
        StartupTrace.mark("window objects initialized")
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 1180, height: 760),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered,
            defer: true
        )
        StartupTrace.mark("NSWindow created")
        window.title = AppInfo.name
        window.titleVisibility = .visible
        window.toolbarStyle = .unified
        window.titlebarSeparatorStyle = .automatic
        window.minSize = NSSize(width: 860, height: 520)
        window.setFrameAutosaveName("LorcaMainWindow")
        window.tabbingMode = .disallowed
        window.animationBehavior = .none

        super.init(window: window)
        window.delegate = self

        let toolbar = NSToolbar(identifier: "LorcaToolbar")
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        window.toolbar = toolbar
        StartupTrace.mark("toolbar installed")

        installTitlebarButtons()
        StartupTrace.mark("titlebar accessories installed")

        // Attach the split view once the saved frame and titlebar are configured, so its
        // first layout uses the final content area instead of relaying out for each change.
        // The window takes the size of the view it is handed, so the split view comes in at
        // the saved size.
        if let contentView = window.contentView { root.view.frame = contentView.frame }
        window.contentViewController = root
        StartupTrace.mark("window content attached")

        root.onSelectionChange = { [weak self] in
            self?.updateTitle()
            self?.updateToolbar()
            Notifier.shared.watchingChanged()
        }
        AppStore.shared.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced: self?.updateToolbar()
            case .messageAdded, .messageChanged, .messageRemoved, .chatsChanged, .runningTasksChanged: self?.updateRunningTasks()
            default: break
            }
        }
        updateTitle()
        updateToolbar()
        StartupTrace.mark("window configured")
    }

    /// Views take their words when they are built, so a new language makes the panes, the
    /// titlebar buttons, and the toolbar's items again. The window stays, and with it its frame,
    /// its Space, and whether it is zoomed, tiled, or in full screen.
    func languageChanged() {
        palette?.close()
        palette = nil
        installTitlebarButtons()
        tasksPopover?.close()
        tasksButton.toolTip = L("Running tasks")
        if let toolbar = window?.toolbar {
            let worded: Set<NSToolbarItem.Identifier> = [.settingsNavigation, .devicePicker, .inspectorToggle, .runningTasks]
            for (index, item) in toolbar.items.enumerated() where worded.contains(item.itemIdentifier) {
                toolbar.removeItem(at: index)
                toolbar.insertItem(withItemIdentifier: item.itemIdentifier, at: index)
            }
        }
        root.languageChanged()
        updateTitle()
        updateToolbar()
    }

    /// The sidebar buttons form a leading titlebar accessory, so they keep their place beside the
    /// traffic lights when the sidebar collapses. A new language replaces them.
    private func installTitlebarButtons() {
        titlebarButtons?.removeFromParent()
        let createButton = HoverButton(
            symbol: "plus", tooltip: L("Create"), target: nil, action: #selector(AppDelegate.newBot(_:)))
        createButton.menu = Self.createMenu()
        self.createButton = createButton
        let accessory = Self.leadingAccessory([
            HoverButton(
                symbol: "sidebar.leading", tooltip: L("Toggle Sidebar (⌘B)"), target: root,
                action: #selector(NSSplitViewController.toggleSidebar(_:))),
            createButton,
        ])
        window?.addTitlebarAccessoryViewController(accessory)
        titlebarButtons = accessory
    }

    // MARK: - Settings toolbar

    private func updateToolbar() {
        guard let toolbar = window?.toolbar else { return }
        var isScoped = false
        if case let .settings(pane) = root.selection { isScoped = pane.isDeviceScoped }

        // Settings has no inspector and nothing to create.
        let isSettings = root.selection?.isSettings == true
        let arrows = toolbar.items.firstIndex { $0.itemIdentifier == .settingsNavigation }
        if isSettings, arrows == nil {
            let at = toolbar.items.firstIndex { $0.itemIdentifier == .sidebarTrackingSeparator }.map { $0 + 1 } ?? 0
            toolbar.insertItem(withItemIdentifier: .settingsNavigation, at: at)
        } else if !isSettings, let arrows {
            toolbar.removeItem(at: arrows)
        }
        updateNavigation()
        createButton?.isHidden = isSettings
        let toggle = toolbar.items.firstIndex { $0.itemIdentifier == .inspectorToggle }
        if isSettings, let toggle {
            toolbar.removeItem(at: toggle)
        } else if !isSettings, toggle == nil {
            toolbar.insertItem(withItemIdentifier: .inspectorToggle, at: toolbar.items.count)
        }
        // Running tasks belong to the chats, at the content area's trailing edge. The item stays
        // while chats show and only its button hides, so the toolbar keeps its layout.
        let tasks = toolbar.items.firstIndex { $0.itemIdentifier == .runningTasks }
        if isSettings, let tasks {
            toolbar.removeItem(at: tasks)
        } else if !isSettings, tasks == nil {
            let at = toolbar.items.firstIndex { $0.itemIdentifier == .inspectorTrackingSeparator } ?? toolbar.items.count
            toolbar.insertItem(withItemIdentifier: .runningTasks, at: at)
        }
        updateRunningTasks()

        // The picker joins the toolbar with Settings. An item entering, leaving, or hiding makes
        // the toolbar lay its glass out again, which blinks the back and forward buttons.
        let index = toolbar.items.firstIndex { $0.itemIdentifier == .devicePicker }
        if isSettings, index == nil {
            // At the content area's trailing edge, ahead of the inspector's section.
            let at = toolbar.items.firstIndex { $0.itemIdentifier == .inspectorTrackingSeparator } ?? toolbar.items.count
            toolbar.insertItem(withItemIdentifier: .devicePicker, at: at)
        } else if !isSettings, let index {
            toolbar.removeItem(at: index)
        }
        // The item keeps its place and width; only the pop-up inside shows and hides, so the
        // toolbar never lays out again between panes.
        // The platter exists once the item was made. Before that this hides the pop-up itself,
        // which must not stay hidden inside a platter that shows.
        if let devicePlatter {
            devicePlatter.isHidden = !isScoped
            devicePicker.isHidden = false
        } else if isSettings {
            devicePicker.isHidden = !isScoped
        }
        guard isScoped else { return }

        devicePicker.removeAllItems()
        for device in AppStore.shared.devices {
            devicePicker.addItem(withTitle: "")
            guard let item = devicePicker.lastItem else { continue }
            item.title = device.isThisDevice ? L("%@ (This computer)", device.name) : device.name
            item.representedObject = device.id
            item.image = NSImage(systemSymbolName: device.symbolName, accessibilityDescription: nil)
        }
        selectPickedDevice()
        devicePicker.menu?.addItem(.separator())
        let pair = NSMenuItem(title: L("Pair a Device…"), action: #selector(pairDevice), keyEquivalent: "")
        pair.target = self
        devicePicker.menu?.addItem(pair)
        devicePicker.sizeToFit()
    }

    /// Back and forward follow the pane history. Only a state that changed is written, to the
    /// group's segmented control when it has one.
    private func updateNavigation() {
        guard let navigation else { return }
        let enabled = [root.canGoBack, root.canGoForward]
        if let control = navigation.view as? NSSegmentedControl {
            for (index, isEnabled) in enabled.enumerated() where control.isEnabled(forSegment: index) != isEnabled {
                control.setEnabled(isEnabled, forSegment: index)
            }
        } else {
            for (index, item) in navigation.subitems.enumerated() where item.isEnabled != enabled[index] {
                item.isEnabled = enabled[index]
            }
        }
    }

    @objc private func navigateSettings(_ sender: NSToolbarItemGroup) {
        if sender.selectedIndex == 0 { root.goBack() } else { root.goForward() }
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

    // MARK: - Running tasks

    /// The chat on screen, when a chat is.
    private var selectedChatID: Chat.ID? {
        if case let .chat(id) = root.selection { return id }
        return nil
    }

    /// Shows the Running tasks button with how many commands the chat on screen has running,
    /// and closes the popover when it belongs to a chat no longer on screen.
    private func updateRunningTasks() {
        let chatID = selectedChatID
        if let popover = tasksPopover, (popover.contentViewController as? RunningTasksViewController)?.chatID != chatID {
            popover.close()
        }
        let count = chatID.map { AppStore.shared.runningCommands(in: $0).count } ?? 0
        // While its popover is open the button stays, for the popover to point at.
        tasksButton.isHidden = count == 0 && tasksPopover == nil
        // The toolbar's own badge, on the symbol's top-right corner, for two or more: the button
        // alone says one runs. Before macOS 26 there is none, and the popover says how many.
        let badge = count > 1 ? count : 0
        if #available(macOS 26.0, *), tasksBadge != badge, let tasksItem {
            tasksItem.badge = badge > 0 ? .count(badge) : nil
            tasksBadge = badge
        }
        tasksButton.setAccessibilityLabel(L("Running tasks (%d)", count))
    }

    @objc private func toggleRunningTasks(_ sender: Any?) {
        if let popover = tasksPopover {
            popover.close()
            return
        }
        // The click that closed the popover.
        guard Date().timeIntervalSince(tasksClosedAt) > 0.3, let chatID = selectedChatID else { return }
        let content = RunningTasksViewController(chatID: chatID)
        _ = content.view
        guard !content.tasks.isEmpty else { return }
        content.onShowCommand = { [weak self] title, command in
            guard let self else { return }
            tasksPopover?.close()
            root.presentAsSheet(CommandSheetViewController(title: title, command: command))
        }
        content.onEmpty = { [weak self] in self?.tasksPopover?.close() }
        let popover = NSPopover()
        popover.behavior = .transient
        popover.contentViewController = content
        popover.delegate = self
        tasksPopover = popover
        popover.show(relativeTo: tasksButton.bounds, of: tasksButton, preferredEdge: .maxY)
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

    /// A new bot opens its own direct chat, so the menu offers bots and groups, then pairing.
    /// Items go through the responder chain to the same actions as the File menu.
    private static func createMenu() -> NSMenu {
        let menu = NSMenu()
        menu.addItem(withTitle: L("Create New Bot…"), action: #selector(AppDelegate.newBot(_:)), keyEquivalent: "")
        menu.addItem(
            withTitle: L("Create Group Chat…"), action: #selector(AppDelegate.newGroupChat(_:)), keyEquivalent: "")
        menu.addItem(.separator())
        menu.addItem(withTitle: L("Pair a Device…"), action: #selector(AppDelegate.pairDevice(_:)), keyEquivalent: "")
        return menu
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func focusSearch() {
        root.focusSearch()
    }

    func toggleCommandPalette() {
        guard let window else { return }
        let palette = palette ?? CommandPalette(parent: window, root: root)
        self.palette = palette
        palette.toggle()
    }

    /// A window coming on screen starts with the keyboard in its content, whatever held it when
    /// the window closed and whichever key view AppKit would pick for a new one.
    override func showWindow(_ sender: Any?) {
        let isOpening = window?.isVisible != true
        super.showWindow(sender)
        StartupTrace.mark("window ordered front")
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
            window.title = AppInfo.name
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
    static let settingsNavigation = NSToolbarItem.Identifier("lorca.settingsNavigation")
    static let runningTasks = NSToolbarItem.Identifier("lorca.runningTasks")
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
        toolbarDefaultItemIdentifiers(toolbar) + [.devicePicker, .settingsNavigation, .runningTasks]
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        if identifier == .settingsNavigation {
            // Back and forward through the settings panes, ahead of the title as in System Settings.
            // The images carry their words: the group writes a label into an image without one,
            // and AppKit hands that same image out again, so a new language would read the old.
            let labels = [L("Back"), L("Forward")]
            let group = NSToolbarItemGroup(
                itemIdentifier: identifier,
                images: zip(["chevron.left", "chevron.right"], labels).map {
                    NSImage(systemSymbolName: $0, accessibilityDescription: $1)!
                },
                selectionMode: .momentary, labels: labels, target: self,
                action: #selector(navigateSettings(_:)))
            group.label = L("Back/Forward")
            group.isNavigational = true
            group.autovalidates = false
            for item in group.subitems { item.autovalidates = false }
            navigation = group
            updateNavigation()
            return group
        }
        if identifier == .devicePicker {
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.label = L("Device")
            devicePicker.setAccessibilityLabel(L("Device"))
            devicePicker.toolTip = L("The Device this page shows")
            // A plain container gets no platter from the toolbar; the pop-up sits on a glass
            // capsule of its own inside it, so hiding the capsule leaves nothing behind and the
            // toolbar's layout stays as it is.
            if #available(macOS 26.0, *) {
                devicePicker.isBordered = false
                devicePicker.font = .systemFont(ofSize: 13)
                devicePicker.translatesAutoresizingMaskIntoConstraints = false
                let padded = NSView()
                padded.addSubview(devicePicker)
                let glass = NSGlassEffectView()
                glass.cornerRadius = 18
                glass.contentView = padded
                glass.translatesAutoresizingMaskIntoConstraints = false
                let container = NSView()
                container.translatesAutoresizingMaskIntoConstraints = false
                container.addSubview(glass)
                NSLayoutConstraint.activate([
                    devicePicker.leadingAnchor.constraint(equalTo: padded.leadingAnchor, constant: 12),
                    devicePicker.trailingAnchor.constraint(equalTo: padded.trailingAnchor, constant: -10),
                    devicePicker.centerYAnchor.constraint(equalTo: padded.centerYAnchor),
                    glass.heightAnchor.constraint(equalToConstant: 36),
                    glass.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                    glass.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                    glass.topAnchor.constraint(equalTo: container.topAnchor),
                    glass.bottomAnchor.constraint(equalTo: container.bottomAnchor),
                ])
                devicePlatter = glass
                item.view = container
                item.isBordered = false
            } else {
                item.view = devicePicker
            }
            return item
        }
        if identifier == .runningTasks {
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.label = L("Running tasks")
            item.view = tasksButton
            item.isBordered = false
            // A new item, as after a language change, takes the badge afresh.
            tasksItem = item
            tasksBadge = 0
            return item
        }
        guard identifier == .inspectorToggle else { return nil }
        let item = NSToolbarItem(itemIdentifier: identifier)
        item.label = L("Inspector")
        item.view = HoverButton(
            symbol: "sidebar.trailing", tooltip: L("Toggle Inspector (⇧⌘B)"), target: root,
            action: #selector(RootSplitViewController.toggleInspector(_:)))
        item.isBordered = false
        return item
    }
}

extension MainWindowController: NSPopoverDelegate {
    func popoverDidClose(_ notification: Notification) {
        guard (notification.object as? NSPopover) === tasksPopover else { return }
        tasksPopover = nil
        tasksClosedAt = Date()
        updateRunningTasks()
    }
}
