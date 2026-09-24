import AppKit

final class RootSplitViewController: NSSplitViewController {
    private let store = AppStore.shared

    private let sidebarContainer = ContentContainerViewController()
    private lazy var sidebar = makeSidebar()
    private var settingsSidebarStorage: SettingsSidebarViewController?
    private var settingsSidebar: SettingsSidebarViewController {
        if let controller = settingsSidebarStorage { return controller }
        let controller = SettingsSidebarViewController()
        controller.onSelect = { [weak self] selection in self?.select(selection) }
        controller.onReveal = { [weak self] entry in
            (self?.settingsController(for: entry.pane) as? SettingsPaneViewController)?.reveal(entry)
        }
        controller.onBack = { [weak self] in self?.closeSettings() }
        controller.setDevice(settingsDeviceID)
        settingsSidebarStorage = controller
        return controller
    }
    private let content = ContentContainerViewController()
    private let inspectorContainer = ContentContainerViewController()
    private lazy var inspector = makeInspector()

    private var sidebarItem: NSSplitViewItem!
    private var inspectorItem: NSSplitViewItem!
    /// The sidebars' bars as split view item accessories, made on first use, keyed by the bar.
    private var sidebarAccessories: [ObjectIdentifier: NSViewController] = [:]

    private var userWantsInspector = Preferences.showsInspector
    private var displayedConnection: Bool?
    private var displayedStarting: Bool?
    private var displayedChatID: Chat.ID?
    private var chatController: ChatViewController?
    /// The Device the Plugins, Bots, and Devices panes show, picked in the window's
    /// toolbar. This computer until another is picked.
    private(set) var settingsDeviceID: Device.ID?
    private var settingsControllers: [SettingsPane: NSViewController] = [:]
    /// The chat Back returns to.
    private var lastChatID: Chat.ID?
    /// What held the keyboard when Settings opened, to hand it back on the way out.
    private weak var focusBeforeSettings: NSView?
    private lazy var offlineController = makeOfflineController()
    private var loadingController = LoadingViewController()
    private lazy var placeholderController = makePlaceholderController()

    var onSelectionChange: (() -> Void)?

    /// The panes visited since Settings opened, for the toolbar's back and forward.
    private var paneHistory: [SettingsPane] = []
    private var paneIndex = 0
    private var isWalkingHistory = false

    var canGoBack: Bool { paneIndex > 0 }
    var canGoForward: Bool { paneIndex < paneHistory.count - 1 }

    func goBack() { walkHistory(to: paneIndex - 1) }
    func goForward() { walkHistory(to: paneIndex + 1) }

    private func walkHistory(to index: Int) {
        guard paneHistory.indices.contains(index) else { return }
        paneIndex = index
        isWalkingHistory = true
        select(.settings(paneHistory[index]))
        isWalkingHistory = false
        onSelectionChange?()
    }

    private func recordHistory() {
        guard case let .settings(pane) = selection else {
            paneHistory = []
            paneIndex = 0
            return
        }
        guard !isWalkingHistory else { return }
        paneHistory = Array(paneHistory.prefix(paneIndex + 1))
        if paneHistory.last != pane { paneHistory.append(pane) }
        paneIndex = paneHistory.count - 1
    }

    private(set) var selection: Selection? {
        didSet {
            guard selection != oldValue else { return }
            recordHistory()
            if case let .chat(id) = oldValue { lastChatID = id }
            Preferences.selection = encode(selection)
            updateContent()
            onSelectionChange?()
        }
    }

    // MARK: - Lifecycle

    override func viewDidLoad() {
        super.viewDidLoad()

        sidebarItem = NSSplitViewItem(sidebarWithViewController: sidebarContainer)
        sidebarItem.minimumThickness = 232
        sidebarItem.maximumThickness = 340
        sidebarItem.canCollapse = true

        let contentItem = NSSplitViewItem(viewController: content)
        contentItem.minimumThickness = 460
        contentItem.canCollapse = false

        inspectorItem = NSSplitViewItem(inspectorWithViewController: inspectorContainer)
        inspectorItem.minimumThickness = 268
        inspectorItem.maximumThickness = 320
        inspectorItem.isCollapsed = !userWantsInspector || Preferences.selection?.hasPrefix("settings:") == true

        addSplitViewItem(sidebarItem)
        addSplitViewItem(contentItem)
        addSplitViewItem(inspectorItem)

        store.observe(self) { [weak self] event in
            self?.handle(event)
        }

        showSettingsDevice(nil)
        restoreSelection()
    }

    private func restoreSelection() {
        let restored: Selection?
        if let encoded = Preferences.selection, let decoded = decode(encoded), exists(decoded) {
            restored = decoded
        } else {
            restored = store.chats.first.map { .chat($0.id) }
        }
        if restored == selection { updateContent() } else { selection = restored }
        syncSidebar()
    }

    private func exists(_ selection: Selection) -> Bool {
        switch selection {
        case let .chat(id): store.chat(id) != nil
        case .settings: true
        }
    }

    private func encode(_ selection: Selection?) -> String? {
        switch selection {
        case let .chat(id): "chat:\(id)"
        case let .settings(pane): "settings:\(pane.rawValue)"
        case nil: nil
        }
    }

    private func decode(_ raw: String) -> Selection? {
        let parts = raw.split(separator: ":", maxSplits: 1).map(String.init)
        guard parts.count == 2 else { return nil }
        switch parts[0] {
        case "chat": return .chat(parts[1])
        case "settings": return SettingsPane(rawValue: parts[1]).map { .settings($0) }
        default: return nil
        }
    }

    // MARK: - Panes

    /// Views take their words when they are built, so a new language builds every pane again
    /// inside this split view. The sidebar and the inspector keep their widths and stay shown or
    /// collapsed; the selection, the settings history, and the picked Device stay as they are.
    func languageChanged() {
        sidebar = makeSidebar()
        settingsSidebarStorage = nil
        sidebarAccessories = [:]
        chatController = nil
        inspector = makeInspector()
        settingsControllers = [:]
        offlineController = makeOfflineController()
        loadingController = LoadingViewController()
        placeholderController = makePlaceholderController()
        guard isViewLoaded else { return }
        updateContent()
        syncSidebar()
    }

    private func makeSidebar() -> SidebarViewController {
        let controller = SidebarViewController()
        controller.onSelect = { [weak self] selection in
            self?.select(selection)
        }
        controller.onDoubleClick = { [weak self] selection in
            guard let self, case let .chat(id) = selection, store.chat(id)?.isGroup == true else { return }
            renameChat(nil)
        }
        controller.onOpenDevice = { [weak self] deviceID in
            self?.openDevice(deviceID)
        }
        controller.onOpenMarketplace = { [weak self] in self?.presentMarketplace() }
        return controller
    }

    private func makeInspector() -> InspectorViewController {
        let controller = InspectorViewController()
        controller.onOpenDevice = { [weak self] id in self?.openDevice(id) }
        controller.onRemoveBot = { [weak self] botID in
            guard case let .chat(chatID) = self?.selection else { return }
            self?.store.removeBot(botID, from: chatID)
        }
        controller.onAddBot = { [weak self] in self?.addBotToChat(nil) }
        controller.onComposePrompt = { [weak self] text in self?.chatController?.prefill(text) }
        controller.onOpenMarketplace = { [weak self] runnerID in self?.presentMarketplace(runnerID: runnerID) }
        return controller
    }

    private func makeOfflineController() -> OfflineViewController {
        let controller = OfflineViewController()
        controller.onRetry = { [weak self] in self?.store.reconnect() }
        return controller
    }

    private func makePlaceholderController() -> PlaceholderViewController {
        let controller = PlaceholderViewController()
        controller.onNewBot = { [weak self] in self?.presentNewBot() }
        return controller
    }

    // MARK: - Selection

    func select(_ newSelection: Selection?) {
        if newSelection?.isSettings == true, selection?.isSettings != true {
            focusBeforeSettings = view.window?.firstResponder as? NSView
        }
        selection = newSelection
        if case let .chat(id) = newSelection {
            store.markRead(id)
        }
        syncSidebar()
    }

    /// Settings lives in this window: its panes take the content area, and the sidebar lists them
    /// in place of the chats.
    func showSettings() {
        if sidebarItem.isCollapsed { sidebarItem.animator().isCollapsed = false }
        guard selection?.isSettings != true else { return }
        select(.settings(.general))
    }

    func showSettings(_ pane: SettingsPane) {
        if sidebarItem.isCollapsed { sidebarItem.animator().isCollapsed = false }
        select(.settings(pane))
    }

    /// Opens a setting's pane, scrolls to its row, and flashes it.
    func reveal(_ entry: SettingsEntry) {
        showSettings(entry.pane)
        (settingsController(for: entry.pane) as? SettingsPaneViewController)?.reveal(entry)
    }

    /// The Devices pane on that Device, with the other Device panes on it too.
    func openDevice(_ id: Device.ID) {
        showSettingsDevice(id)
        if sidebarItem.isCollapsed { sidebarItem.animator().isCollapsed = false }
        select(.settings(.device))
    }

    func showSettingsDevice(_ id: Device.ID?) {
        settingsDeviceID = id.flatMap { store.device($0) }?.id ?? store.thisDevice?.id
        settingsSidebarStorage?.setDevice(settingsDeviceID)
        for case let controller as DevicePaneViewController in settingsControllers.values {
            controller.show(deviceID: settingsDeviceID)
        }
        onSelectionChange?()
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

    /// Shows the sidebar the selection belongs to, with its row selected. When the sidebars trade
    /// places the keyboard moves with them: to the settings list on the way in, back to whatever
    /// had it on the way out.
    private func syncSidebar() {
        let isSettings = selection?.isSettings == true
        let wasSettings = settingsSidebarStorage?.parent != nil
        sidebarContainer.show(isSettings ? settingsSidebar : sidebar)
        if isSettings {
            settingsSidebar.setSelection(selection)
        } else {
            sidebar.setSelection(selection)
        }
        if #available(macOS 26.0, *) { syncSidebarAccessories(isSettings: isSettings) }
        guard isSettings != wasSettings else { return }
        if !isSettings { settingsSidebar.resetSearch() }
        guard !sidebarItem.isCollapsed else { return }
        if isSettings { settingsSidebar.focusList() } else { restoreFocusAfterSettings() }
    }

    // MARK: - Focus

    /// Where the keyboard goes when the user has not put it anywhere: the composer in a chat,
    /// the settings list on a settings or Device page. With no chat to type in (the offline
    /// page, an empty roster) the window holds it, so the chat that shows next finds the
    /// keyboard free and takes it in `ChatViewController.viewDidAppear`.
    func focusContent() {
        guard let window = view.window else { return }
        if selection?.isSettings == true {
            settingsSidebar.focusList()
        } else if let chatController, chatController.view.window === window {
            chatController.focusComposer()
        } else {
            window.makeFirstResponder(nil)
        }
    }

    /// AppKit hands the keyboard to the window when the view that had it leaves. Nothing the
    /// user does puts it there, so a window found holding it gives it to the content.
    func reclaimFocusIfLost() {
        guard let window = view.window, window.firstResponder === window else { return }
        focusContent()
    }

    private func restoreFocusAfterSettings() {
        let held = focusBeforeSettings
        focusBeforeSettings = nil
        if let held, let window = view.window, held.window === window, window.makeFirstResponder(held) { return }
        focusContent()
    }

    /// The showing sidebar's bars, above and below its list. The list fills the pane and scrolls
    /// under them; AppKit insets it by their heights and softens the edge behind them.
    @available(macOS 26.0, *)
    private func syncSidebarAccessories(isSettings: Bool) {
        let top = accessory(for: isSettings ? settingsSidebar.header : sidebar.searchBar)
        let bottom = [accessory(for: isSettings ? settingsSidebar.footer : sidebar.footer)]
        if sidebarItem.topAlignedAccessoryViewControllers != [top] {
            sidebarItem.topAlignedAccessoryViewControllers = [top]
        }
        if sidebarItem.bottomAlignedAccessoryViewControllers != bottom {
            sidebarItem.bottomAlignedAccessoryViewControllers = bottom
        }
    }

    @available(macOS 26.0, *)
    private func accessory(for bar: NSView) -> NSSplitViewItemAccessoryViewController {
        if let made = sidebarAccessories[ObjectIdentifier(bar)] as? NSSplitViewItemAccessoryViewController {
            return made
        }
        let controller = NSSplitViewItemAccessoryViewController()
        controller.view = bar
        // The bars carry the sidebar's own margins.
        controller.automaticallyAppliesContentInsets = false
        if #available(macOS 26.1, *) { controller.preferredScrollEdgeEffectStyle = .soft }
        sidebarAccessories[ObjectIdentifier(bar)] = controller
        return controller
    }

    func windowBecameKey() {
        if case let .chat(id) = selection { store.markRead(id) }
        reclaimFocusIfLost()
    }

    private func handle(_ event: StoreEvent) {
        switch event {
        case .connectionChanged:
            if displayedConnection != store.isConnected || displayedStarting != store.isStarting { updateContent() }
        case .snapshotReplaced:
            showSettingsDevice(settingsDeviceID)
            restoreSelection()
        case .chatsChanged:
            if case let .chat(id) = selection, store.chat(id) == nil {
                select(store.chats.first.map { .chat($0.id) })
            }
        case .rosterChanged:
            // An unpaired Device leaves the pickers, which go back to this computer.
            showSettingsDevice(settingsDeviceID)
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
        displayedConnection = store.isConnected
        displayedStarting = store.isStarting

        // The relay URL and the CLI port are what a computer with no CLI answering needs, so the
        // panes show either way.
        if case let .settings(pane) = selection {
            content.show(settingsController(for: pane))
            setInspector(visible: false)
            return
        }

        guard store.isConnected else {
            content.show(store.isStarting ? loadingController : offlineController)
            // Keep the saved inspector width throughout startup. Only its contents wait for
            // the snapshot, so the transcript never expands and then shrinks on first load.
            if !store.isStarting, displayedChatID != nil { setInspector(visible: false) }
            return
        }

        switch selection {
        case let .chat(id):
            guard let chat = store.chat(id) else { return }
            let controller = chatController ?? ChatViewController()
            controller.onRedirect = { [weak self] chatID in self?.select(.chat(chatID)) }
            chatController = controller
            if content.children.first !== controller || displayedChatID != chat.id {
                content.show(controller)
                controller.show(chatID: chat.id)
                inspectorContainer.show(inspector)
                inspector.show(selection: .chat(chat.id))
                displayedChatID = chat.id
            }
            setInspector(visible: userWantsInspector)

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
            case .autoReview: AutoReviewSettingsViewController()
            case .advanced: AdvancedSettingsViewController()
            case .bots: BotsSettingsViewController()
            case .providers: ProvidersSettingsViewController()
            case .plugins: PluginsSettingsViewController()
            case .device: AboutDeviceSettingsViewController()
            }
        if let controller = controller as? DevicePaneViewController {
            controller.show(deviceID: settingsDeviceID)
        }
        (controller as? BotsSettingsViewController)?.onOpenChat = { [weak self] chatID in
            self?.select(.chat(chatID))
        }
        (controller as? PluginsSettingsViewController)?.onOpenMarketplace = { [weak self] runnerID in
            self?.presentMarketplace(runnerID: runnerID)
        }
        settingsControllers[pane] = controller
        return controller
    }

    private func setInspector(visible: Bool) {
        guard inspectorItem.isCollapsed == visible else { return }
        inspectorItem.isCollapsed = !visible
    }

    // MARK: - Actions

    override func toggleInspector(_ sender: Any?) {
        guard case .chat = selection, store.isConnected else { NSSound.beep(); return }
        userWantsInspector = inspectorItem.isCollapsed
        Preferences.showsInspector = userWantsInspector
        super.toggleInspector(sender)
    }

    func focusSearch() {
        if selection?.isSettings == true { settingsSidebar.focusSearch() } else { sidebar.focusSearch() }
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

    func open(_ chatID: Chat.ID) {
        select(.chat(chatID))
        chatController?.focusComposer()
    }

    /// The marketplace, as a sheet sized to the window: Grok Bot's is 800 by 700. From a bot's
    /// inspector it opens on that bot's Runner, from Settings on the picked Device; a bot added
    /// there lands in its chat.
    func presentMarketplace(runnerID: Device.ID? = nil) {
        guard presentedViewControllers?.contains(where: { $0 is MarketplaceViewController }) != true else { return }
        let room = view.window?.contentLayoutRect.size ?? NSSize(width: 1000, height: 760)
        let size = NSSize(width: min(800, max(640, room.width - 60)), height: min(700, max(460, room.height - 60)))
        let sheet = MarketplaceViewController(runnerID: runnerID, size: size) { [weak self] chatID in
            self?.open(chatID)
        }
        presentAsSheet(sheet)
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
            title: L("Add a bot to %@", store.title(for: chat)),
            bots: available
        ) { [weak self] botID in
            self?.store.addBot(botID, to: chatID)
        }
        presentAsSheet(sheet)
    }

    @objc func renameChat(_ sender: Any?) {
        guard case let .chat(chatID) = selection, let chat = store.chat(chatID), chat.isGroup else {
            NSSound.beep()
            return
        }
        let alert = NSAlert()
        alert.messageText = L("Rename Chat")
        alert.informativeText = L("Chat names live inside the encrypted roster blob, never on the relay.")
        alert.addButton(withTitle: L("Rename"))
        alert.addButton(withTitle: L("Cancel"))

        let field = NSTextField(frame: NSRect(x: 0, y: 0, width: 260, height: 24))
        field.stringValue = store.title(for: chat)
        field.placeholderString = L("Chat name")
        alert.accessoryView = field

        guard let window = view.window else { return }
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn else { return }
            self?.store.rename(chatID, to: field.stringValue)
        }
        // The accessory view only becomes first responder once the sheet exists.
        DispatchQueue.main.async { alert.window.makeFirstResponder(field) }
    }

    /// ⌘1–⌘9: the menu item's tag is the chat's place in the sidebar. The composer takes the
    /// keyboard, ready for a reply.
    @objc func goToChat(_ sender: NSMenuItem) {
        guard case let .chat(chatID) = sidebar.chatSelection(forShortcut: sender.tag) else { return }
        open(chatID)
        sidebar.scrollSelectionToVisible()
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
        alert.messageText = L("Delete \"%@\"?", store.title(for: chat))
        let deletesBot = chat.isDM && chat.botIDs.first.flatMap(store.bot) != nil
        alert.informativeText = deletesBot
            ? L("The bot, its routines, and this direct chat are removed from this Device and from paired Devices.")
            : L("The transcript is removed from this Device and from paired Devices.")
        alert.alertStyle = .warning
        alert.addButton(withTitle: deletesBot ? L("Delete Bot") : L("Delete"))
        alert.addButton(withTitle: L("Cancel"))
        alert.buttons.first?.hasDestructiveAction = true
        alert.beginSheetModal(for: window) { [weak self] response in
            guard response == .alertFirstButtonReturn else { return }
            self?.store.deleteChat(chatID)
        }
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
        if menuItem.action == #selector(deleteChat(_:)) {
            guard case let .chat(id) = selection, let chat = store.chat(id) else { return false }
            menuItem.title = chat.isDM ? L("Delete Bot") : L("Delete Chat")
            return true
        }
        if menuItem.action == #selector(addBotToChat(_:)) {
            guard case let .chat(id) = selection, let chat = store.chat(id) else { return false }
            return !botsAvailableToAdd(to: chat).isEmpty
        }
        if menuItem.action == #selector(renameChat(_:)) {
            guard case let .chat(id) = selection, let chat = store.chat(id) else { return false }
            return chat.isGroup
        }
        if menuItem.action == #selector(goToChat(_:)) {
            return sidebar.chatSelection(forShortcut: menuItem.tag) != nil
        }
        return true
    }
}
