import AppKit

final class RootSplitViewController: NSSplitViewController {
    private let store = AppStore.shared

    private let sidebarContainer = ContentContainerViewController()
    private let sidebar = SidebarViewController()
    private let settingsSidebar = SettingsSidebarViewController()
    private let content = ContentContainerViewController()
    private let inspector = InspectorViewController()

    private var sidebarItem: NSSplitViewItem!
    private var inspectorItem: NSSplitViewItem!

    private var userWantsInspector = true
    private var chatController: ChatViewController?
    private let deviceController = DeviceViewController()
    private var settingsControllers: [SettingsPane: NSViewController] = [:]
    /// The chat Back returns to.
    private var lastChatID: Chat.ID?
    private let offlineController = OfflineViewController()
    private let placeholderController = PlaceholderViewController()

    var onSelectionChange: (() -> Void)?

    private(set) var selection: Selection? {
        didSet {
            guard selection != oldValue else { return }
            if case let .chat(id) = oldValue { lastChatID = id }
            Preferences.selection = encode(selection)
            updateContent()
            onSelectionChange?()
        }
    }

    // MARK: - Lifecycle

    override func loadView() {
        super.loadView()
        view.frame = NSRect(x: 0, y: 0, width: 1180, height: 760)
    }

    override func viewDidLoad() {
        super.viewDidLoad()

        sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebarContainer)
        sidebarItem.minimumThickness = 232
        sidebarItem.maximumThickness = 340
        sidebarItem.canCollapse = true

        let contentItem = NSSplitViewItem(viewController: content)
        contentItem.minimumThickness = 460
        contentItem.canCollapse = false

        inspectorItem = NSSplitViewItem(inspectorWithViewController: inspector)
        inspectorItem.minimumThickness = 268
        inspectorItem.maximumThickness = 320

        addSplitViewItem(sidebarItem)
        addSplitViewItem(contentItem)
        addSplitViewItem(inspectorItem)

        sidebar.onSelect = { [weak self] selection in
            self?.select(selection)
        }
        sidebar.onDoubleClick = { [weak self] selection in
            if case .chat = selection { self?.renameChat(nil) }
        }
        settingsSidebar.onSelect = { [weak self] selection in
            self?.select(selection)
        }
        settingsSidebar.onBack = { [weak self] in
            self?.closeSettings()
        }
        inspector.onOpenDevice = { [weak self] deviceID in
            self?.select(.device(deviceID))
        }
        inspector.onRemoveBot = { [weak self] botID in
            guard case let .chat(chatID) = self?.selection else { return }
            self?.store.removeBot(botID, from: chatID)
        }
        inspector.onAddBot = { [weak self] in
            self?.addBotToChat(nil)
        }
        inspector.onComposePrompt = { [weak self] text in
            self?.chatController?.prefill(text)
        }
        offlineController.onRetry = { [weak self] in
            self?.store.reconnect()
        }
        placeholderController.onNewBot = { [weak self] in
            self?.presentNewBot()
        }

        store.observe(self) { [weak self] event in
            self?.handle(event)
        }

        restoreSelection()
        updateContent()
    }

    private func restoreSelection() {
        if let encoded = Preferences.selection, let decoded = decode(encoded), exists(decoded) {
            selection = decoded
        } else {
            selection = store.chats.first.map { .chat($0.id) }
        }
        syncSidebar()
    }

    private func exists(_ selection: Selection) -> Bool {
        switch selection {
        case let .chat(id): store.chat(id) != nil
        case .settings: true
        case let .device(id): store.device(id) != nil
        }
    }

    private func encode(_ selection: Selection?) -> String? {
        switch selection {
        case let .chat(id): "chat:\(id)"
        case let .settings(pane): "settings:\(pane.rawValue)"
        case let .device(id): "device:\(id)"
        case nil: nil
        }
    }

    private func decode(_ raw: String) -> Selection? {
        let parts = raw.split(separator: ":", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return nil }
        switch parts[0] {
        case "chat": return .chat(parts[1])
        case "settings": return SettingsPane(rawValue: parts[1]).map { .settings($0) }
        case "device": return .device(parts[1])
        default: return nil
        }
    }

    // MARK: - Selection

    func select(_ newSelection: Selection?) {
        selection = newSelection
        if case let .chat(id) = newSelection {
            store.markRead(id)
        }
        syncSidebar()
    }

    /// Settings lives in this window: its panes and the Device pages take the content area, and
    /// the sidebar lists them in place of the chats.
    func showSettings() {
        if sidebarItem.isCollapsed { sidebarItem.animator().isCollapsed = false }
        guard selection?.isSettings != true else { return }
        select(.settings(.general))
    }

    func closeSettings() {
        let chat = lastChatID.flatMap { store.chat($0) } ?? store.chats.first
        select(chat.map { .chat($0.id) })
    }

    /// Escape leaves Settings. NSResponder has no implementation to call, so anywhere else the
    /// command keeps travelling up the chain.
    override func cancelOperation(_ sender: Any?) {
        guard selection?.isSettings == true else {
            nextResponder?.doCommand(by: #selector(cancelOperation(_:)))
            return
        }
        closeSettings()
    }

    /// Shows the sidebar the selection belongs to, with its row selected. The list takes the
    /// keyboard when the sidebars trade places, so the selected row draws emphasized.
    private func syncSidebar() {
        let isSettings = selection?.isSettings == true
        let wasSettings = settingsSidebar.parent != nil
        sidebarContainer.show(isSettings ? settingsSidebar : sidebar)
        if isSettings {
            settingsSidebar.setSelection(selection)
        } else {
            sidebar.setSelection(selection)
        }
        guard isSettings != wasSettings, !sidebarItem.isCollapsed else { return }
        if isSettings { settingsSidebar.focusList() } else { sidebar.focusList() }
    }

    func windowBecameKey() {
        if case let .chat(id) = selection { store.markRead(id) }
    }

    private func handle(_ event: StoreEvent) {
        switch event {
        case .connectionChanged:
            updateContent()
        case .snapshotReplaced:
            restoreSelection()
            updateContent()
        case .chatsChanged:
            if case let .chat(id) = selection, store.chat(id) == nil {
                select(store.chats.first.map { .chat($0.id) })
            }
        case .rosterChanged:
            // An unpaired Device leaves the list; its page goes with it.
            if case let .device(id) = selection, store.device(id) == nil {
                select(store.thisDevice.map { .device($0.id) } ?? store.chats.first.map { .chat($0.id) })
            }
        case let .chatChanged(id):
            if case .chat(id) = selection {
                inspector.reload()
                onSelectionChange?()
            }
        default:
            break
        }
    }

    private func updateContent() {
        guard isViewLoaded else { return }

        // The relay URL and the CLI port are what a Mac with no CLI answering needs, so the
        // panes show either way.
        if case let .settings(pane) = selection {
            content.show(settingsController(for: pane))
            setInspector(visible: false)
            return
        }

        guard store.isConnected else {
            content.show(offlineController)
            setInspector(visible: false)
            return
        }

        switch selection {
        case let .chat(id):
            guard let chat = store.chat(id) else { return }
            let controller = chatController ?? ChatViewController()
            controller.onRedirect = { [weak self] chatID in self?.select(.chat(chatID)) }
            chatController = controller
            controller.show(chatID: chat.id)
            content.show(controller)
            inspector.show(selection: .chat(chat.id))
            setInspector(visible: userWantsInspector)

        case let .device(id):
            deviceController.show(deviceID: id)
            deviceController.onOpenChat = { [weak self] chatID in
                self?.select(.chat(chatID))
            }
            content.show(deviceController)
            setInspector(visible: false)

        case .settings:
            break

        case nil:
            content.show(placeholderController)
            setInspector(visible: false)
        }
    }

    private func settingsController(for pane: SettingsPane) -> NSViewController {
        if let controller = settingsControllers[pane] { return controller }
        let controller: NSViewController =
            switch pane {
            case .general: GeneralSettingsViewController()
            case .providers: ProvidersSettingsViewController()
            case .autoReview: AutoReviewSettingsViewController()
            case .advanced: AdvancedSettingsViewController()
            }
        settingsControllers[pane] = controller
        return controller
    }

    private func setInspector(visible: Bool) {
        guard inspectorItem.isCollapsed == visible else { return }
        inspectorItem.animator().isCollapsed = !visible
    }

    // MARK: - Actions

    override func toggleInspector(_ sender: Any?) {
        guard case .chat = selection, store.isConnected else { NSSound.beep(); return }
        userWantsInspector = inspectorItem.isCollapsed
        super.toggleInspector(sender)
    }

    func focusSearch() {
        sidebar.focusSearch()
    }

    func presentNewGroupChat() {
        let sheet = NewGroupChatViewController { [weak self] botIDs, title in
            guard let self else { return }
            self.open(self.store.createChat(kind: .group, with: botIDs, title: title))
        }
        presentAsSheet(sheet)
    }

    /// Every bot has a direct chat, so creating one lands in that chat right away.
    func presentNewBot() {
        let sheet = NewBotViewController { [weak self] botID in
            guard let self else { return }
            self.open(self.store.dm(with: botID))
        }
        presentAsSheet(sheet)
    }

    private func open(_ chatID: Chat.ID) {
        select(.chat(chatID))
        chatController?.focusComposer()
    }

    func presentPairing() {
        let sheet = PairingSheetViewController()
        presentAsSheet(sheet)
    }

    /// Bots that could still join the selected chat. Empty for a DM, a full group, or when every
    /// bot is already in it.
    private func botsAvailableToAdd(to chat: Chat) -> [Bot] {
        guard chat.canAddBot else { return [] }
        return store.bots.filter { !chat.botIDs.contains($0.id) }
    }

    @objc func addBotToChat(_ sender: Any?) {
        guard case let .chat(chatID) = selection, let chat = store.chat(chatID) else {
            NSSound.beep()
            return
        }
        let available = botsAvailableToAdd(to: chat)
        guard !available.isEmpty else {
            NSSound.beep()
            return
        }
        let sheet = BotPickerViewController(
            title: "Add a bot to \(store.title(for: chat))",
            bots: available
        ) { [weak self] botID in
            self?.store.addBot(botID, to: chatID)
        }
        presentAsSheet(sheet)
    }

    @objc func renameChat(_ sender: Any?) {
        guard case let .chat(chatID) = selection, let chat = store.chat(chatID) else {
            NSSound.beep()
            return
        }
        let alert = NSAlert()
        alert.messageText = "Rename Chat"
        alert.informativeText = "Chat names live inside the encrypted roster blob, never on the relay."
        alert.addButton(withTitle: "Rename")
        alert.addButton(withTitle: "Cancel")

        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.stringValue = store.title(for: chat)
        field.placeholderString = "Chat name"
        alert.accessoryView = field

        guard let window = view.window else { return }
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn else { return }
            self?.store.rename(chatID, to: field.stringValue)
        }
        // The accessory view only becomes first responder once the sheet exists.
        DispatchQueue.main.async { alert.window.makeFirstResponder(field) }
    }

    @objc func togglePinChat(_ sender: Any?) {
        guard case let .chat(chatID) = selection else { return }
        store.togglePin(chatID)
    }

    @objc func deleteChat(_ sender: Any?) {
        guard case let .chat(chatID) = selection, let chat = store.chat(chatID),
            let window = view.window
        else {
            NSSound.beep()
            return
        }
        let alert = NSAlert()
        alert.messageText = "Delete \"\(store.title(for: chat))\"?"
        alert.informativeText = "The transcript is removed from this Device and from paired Devices."
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Delete")
        alert.addButton(withTitle: "Cancel")
        alert.buttons.first?.hasDestructiveAction = true
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn else { return }
            self?.store.deleteChat(chatID)
        }
    }

    @objc func unpairDevice(_ sender: Any?) {
        guard case let .device(id) = selection, let device = store.device(id), !device.isThisDevice else {
            NSSound.beep()
            return
        }
        UnpairDevice.confirm(device, in: view.window)
    }
}

// MARK: - Content container

final class ContentContainerViewController: NSViewController {
    private var current: NSViewController?

    override func loadView() {
        let container = NSView()
        container.wantsLayer = true
        view = container
    }

    func show(_ controller: NSViewController) {
        guard current !== controller else { return }

        if let current {
            current.view.removeFromSuperview()
            current.removeFromParent()
        }

        addChild(controller)
        controller.view.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(controller.view)
        controller.view.pin(to: view)
        current = controller
    }
}

// MARK: - Menu state

extension RootSplitViewController: NSMenuItemValidation {
    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if menuItem.action == #selector(addBotToChat(_:)) {
            guard case let .chat(id) = selection, let chat = store.chat(id) else { return false }
            return !botsAvailableToAdd(to: chat).isEmpty
        }
        return true
    }
}
