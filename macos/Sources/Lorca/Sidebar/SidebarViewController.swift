import AppKit

final class SidebarViewController: NSViewController {
    private let store = AppStore.shared

    private lazy var outlineView = NSOutlineView()
    private lazy var scrollView = NSScrollView()
    private let listHost = NSView()
    private var listInstalled = false
    /// The views above and below the list. Where the sidebar's chrome floats, the root hangs them
    /// on the split view item; otherwise they are laid out in this view.
    let searchBar = SidebarSearchBar()
    let footer = SidebarFooterView()

    private var nodes: [SidebarNode] = []
    /// A chat keeps its node while it is listed, so a row that moves stays the same item to the
    /// outline view, with its row view, cell, and selection.
    private var chatNodes: [Chat.ID: SidebarNode] = [:]
    /// The root can restore selection before the first snapshot has created the outline rows.
    private var selection: Selection?
    private var isApplyingSelection = false
    private var isNotifyingSelection = false

    /// ⌘1–⌘9 open the first nine rows.
    private static let shortcutCount = 9
    private var flagsMonitor: Any?
    private var pendingShortcutHints: DispatchWorkItem?
    private var showsShortcutHints = false {
        didSet { if showsShortcutHints != oldValue { refreshShortcutHints() } }
    }

    var onSelect: ((Selection?) -> Void)?
    var onOpenDevice: ((Device.ID) -> Void)?
    var onDoubleClick: ((Selection) -> Void)?

    override func loadView() {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false

        listHost.translatesAutoresizingMaskIntoConstraints = false

        // The palette searches the chats, so the field opens it instead of taking the keyboard.
        searchBar.onActivate = { [weak self] in self?.focusSearch() }

        // Both land in Settings: General, or this computer's About pane.
        footer.onSettings = { [weak self] in
            self?.onSelect?(.settings(.general))
        }
        footer.onDevice = { [weak self] in
            guard let id = self?.store.thisDevice?.id else { return }
            self?.onOpenDevice?(id)
        }

        if SidebarChrome.floats {
            // The root hangs the search bar and the footer on the split view item, and the list
            // runs the pane's full height beneath them, inset by the safe area they extend.
            // The gap between the search bar and the first chat.
            container.additionalSafeAreaInsets.top = 5
            container.addSubview(listHost)
            listHost.pin(to: container)
            view = container
            return
        }

        let divider = HairlineView()

        container.addSubview(searchBar)
        container.addSubview(listHost)
        container.addSubview(divider)
        container.addSubview(footer)

        NSLayoutConstraint.activate([
            // The safe area keeps the field clear of the titlebar the sidebar runs under.
            searchBar.topAnchor.constraint(equalTo: container.safeAreaLayoutGuide.topAnchor),
            searchBar.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            searchBar.trailingAnchor.constraint(equalTo: container.trailingAnchor),

            listHost.topAnchor.constraint(equalTo: searchBar.bottomAnchor),
            listHost.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            listHost.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            listHost.bottomAnchor.constraint(equalTo: divider.topAnchor),

            divider.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            divider.bottomAnchor.constraint(equalTo: footer.topAnchor),

            footer.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            footer.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            footer.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])

        view = container
    }

    func focusSearch() {
        NSApp.sendAction(#selector(AppDelegate.toggleCommandPalette(_:)), to: nil, from: nil)
    }

    func focusList() {
        guard listInstalled else { return }
        view.window?.makeFirstResponder(outlineView)
    }

    /// The loading sidebar needs its chrome, but no empty table to build and lay out twice.
    private func installList() {
        guard !listInstalled else { return }
        listInstalled = true
        configureOutlineView()
        scrollView.documentView = outlineView
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.automaticallyAdjustsContentInsets = SidebarChrome.floats
        scrollView.contentInsets = SidebarChrome.floats ? NSEdgeInsets() : NSEdgeInsets(top: 7, left: 0, bottom: 8, right: 0)
        listHost.addSubview(scrollView)
        scrollView.pin(to: listHost)
    }

    private func configureOutlineView() {
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("main"))
        column.resizingMask = .autoresizingMask
        outlineView.addTableColumn(column)
        outlineView.outlineTableColumn = column
        outlineView.headerView = nil
        outlineView.style = .sourceList
        outlineView.rowSizeStyle = .custom
        outlineView.indentationPerLevel = 0
        outlineView.floatsGroupRows = false
        outlineView.backgroundColor = .clear
        outlineView.allowsMultipleSelection = false
        outlineView.allowsEmptySelection = true
        outlineView.dataSource = self
        outlineView.delegate = self
        outlineView.target = self
        outlineView.doubleAction = #selector(handleDoubleClick)
        outlineView.menu = makeContextMenu()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        rebuild()
        store.observe(self) { [weak self] event in
            switch event {
            case .chatsChanged, .snapshotReplaced, .chatChanged:
                self?.rebuild()
            case .connectionChanged:
                self?.footer.update()
            default:
                break
            }
        }
        observeCommandKey()
    }

    deinit {
        if let flagsMonitor { NSEvent.removeMonitor(flagsMonitor) }
    }

    // MARK: - Data

    /// Brings the rows in line with the store. Rows move, arrive, or leave only when the order
    /// of the chats changed, and a cell is reconfigured only when what it shows changed, so an
    /// event during a reply touches only the rows it concerns and builds no views.
    private func rebuild() {
        let ids = store.chats.map(\.id)
        if !ids.isEmpty { installList() }

        if listInstalled, ids != nodes.compactMap(\.chatID) {
            // The outline view is still handling the click that selected a row; the rows move
            // once it is done.
            if isNotifyingSelection {
                DispatchQueue.main.async { [weak self] in self?.rebuild() }
                return
            }
            reorder(to: ids)
        }
        refreshCells()
        footer.update()
    }

    /// Moves, inserts, and removes rows until they follow `ids`. Only the first rows come in
    /// with a reload: a reload builds every row view and cell again, and a row view built while
    /// the outline view handles the click that selected it draws its selection unemphasized
    /// (gray) until the next selection change.
    private func reorder(to ids: [Chat.ID]) {
        let wasApplyingSelection = isApplyingSelection
        isApplyingSelection = true
        defer { isApplyingSelection = wasApplyingSelection }

        guard !nodes.isEmpty else {
            nodes = ids.map(node(for:))
            outlineView.reloadData()
            setSelection(selection)
            return
        }

        let listed = Set(ids)
        // Rows jump to their places, as a reload put them; the outline view would slide them.
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            outlineView.beginUpdates()
            let gone = IndexSet(nodes.indices.filter { !listed.contains(nodes[$0].chatID ?? "") })
            for index in gone.reversed() {
                if let id = nodes[index].chatID { chatNodes[id] = nil }
                nodes.remove(at: index)
            }
            if !gone.isEmpty { outlineView.removeItems(at: gone, inParent: nil, withAnimation: []) }
            // Top down, each place takes its chat: the row already there, a row moved up from
            // further down (a chat with new activity takes one move), or a new row.
            for (index, id) in ids.enumerated() where index == nodes.count || nodes[index].chatID != id {
                if let from = nodes[index...].firstIndex(where: { $0.chatID == id }) {
                    nodes.insert(nodes.remove(at: from), at: index)
                    outlineView.moveItem(at: from, inParent: nil, to: index, inParent: nil)
                } else {
                    nodes.insert(node(for: id), at: index)
                    outlineView.insertItems(at: IndexSet(integer: index), inParent: nil, withAnimation: [])
                }
            }
            outlineView.endUpdates()
        }
        setSelection(selection)
    }

    private func node(for id: Chat.ID) -> SidebarNode {
        if let node = chatNodes[id] { return node }
        let node = SidebarNode(.chat(id))
        chatNodes[id] = node
        return node
    }

    /// Updates the rows the outline view holds cells for, on screen or kept ready beside it.
    /// The other rows get theirs when they scroll into view.
    private func refreshCells() {
        guard listInstalled else { return }
        outlineView.enumerateAvailableRowViews { rowView, row in
            guard let cell = rowView.view(atColumn: 0) as? SidebarChatCell,
                let id = (outlineView.item(atRow: row) as? SidebarNode)?.chatID, let chat = store.chat(id)
            else { return }
            cell.configure(SidebarChatCell.Content(chat: chat, store: store))
            cell.shortcutNumber = shortcutNumber(forRow: row)
        }
    }

    // MARK: - Selection

    private func currentSelection() -> Selection? {
        guard listInstalled else { return nil }
        let row = outlineView.selectedRow
        guard row >= 0, let node = outlineView.item(atRow: row) as? SidebarNode else { return nil }
        return node.selection
    }

    func setSelection(_ selection: Selection?) {
        self.selection = selection
        guard listInstalled else { return }
        let wasApplyingSelection = isApplyingSelection
        isApplyingSelection = true
        defer { isApplyingSelection = wasApplyingSelection }
        guard let selection else {
            outlineView.deselectAll(nil)
            return
        }
        guard case let .chat(id) = selection, let node = chatNodes[id] else { return }
        let row = outlineView.row(forItem: node)
        guard row >= 0, row != outlineView.selectedRow else { return }
        outlineView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
    }

    // MARK: - Shortcuts

    /// The chat ⌘`number` opens: the row at that place in the list as it shows, filtered or not.
    func chatSelection(forShortcut number: Int) -> Selection? {
        guard (1...Self.shortcutCount).contains(number), number <= nodes.count else { return nil }
        return nodes[number - 1].selection
    }

    func scrollSelectionToVisible() {
        guard listInstalled, outlineView.selectedRow >= 0 else { return }
        outlineView.scrollRowToVisible(outlineView.selectedRow)
    }

    private func shortcutNumber(forRow row: Int) -> Int? {
        showsShortcutHints && row < Self.shortcutCount ? row + 1 : nil
    }

    private func refreshShortcutHints() {
        guard listInstalled else { return }
        for row in 0..<min(Self.shortcutCount, outlineView.numberOfRows) {
            let cell = outlineView.view(atColumn: 0, row: row, makeIfNecessary: false) as? SidebarChatCell
            cell?.shortcutNumber = shortcutNumber(forRow: row)
        }
    }

    /// Holding ⌘ alone puts each row's number in its stamp. The hints wait a moment, so a quick
    /// ⌘C leaves the stamps alone, and go when the window stops taking keys: the release of a
    /// ⌘-Tab never arrives here.
    private func observeCommandKey() {
        flagsMonitor = NSEvent.addLocalMonitorForEvents(matching: .flagsChanged) { [weak self] event in
            self?.commandKeyChanged(event)
            return event
        }
        let center = NotificationCenter.default
        center.addObserver(
            self, selector: #selector(hideShortcutHints), name: NSApplication.didResignActiveNotification,
            object: nil)
        center.addObserver(
            self, selector: #selector(hideShortcutHints), name: NSWindow.didResignKeyNotification, object: nil)
    }

    private func commandKeyChanged(_ event: NSEvent) {
        let flags = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
            .subtracting([.capsLock, .numericPad, .function])
        guard flags == .command, view.window?.isKeyWindow == true else {
            hideShortcutHints()
            return
        }
        guard !showsShortcutHints, pendingShortcutHints == nil else { return }
        let work = DispatchWorkItem { [weak self] in
            self?.pendingShortcutHints = nil
            self?.showsShortcutHints = true
        }
        pendingShortcutHints = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.25, execute: work)
    }

    @objc private func hideShortcutHints() {
        pendingShortcutHints?.cancel()
        pendingShortcutHints = nil
        showsShortcutHints = false
    }

    @objc private func handleDoubleClick() {
        let row = outlineView.clickedRow
        guard row >= 0, let node = outlineView.item(atRow: row) as? SidebarNode,
            let selection = node.selection
        else { return }
        onDoubleClick?(selection)
    }

    // MARK: - Context menu

    private func makeContextMenu() -> NSMenu {
        let menu = NSMenu()
        menu.delegate = self
        return menu
    }
}

// MARK: - Data source

extension SidebarViewController: NSOutlineViewDataSource {
    func outlineView(_ outlineView: NSOutlineView, numberOfChildrenOfItem item: Any?) -> Int {
        guard let node = item as? SidebarNode else { return nodes.count }
        return node.children.count
    }

    func outlineView(_ outlineView: NSOutlineView, child index: Int, ofItem item: Any?) -> Any {
        guard let node = item as? SidebarNode else { return nodes[index] }
        return node.children[index]
    }

    func outlineView(_ outlineView: NSOutlineView, isItemExpandable item: Any) -> Bool {
        false
    }
}

// MARK: - Delegate

extension SidebarViewController: NSOutlineViewDelegate {
    func outlineView(_ outlineView: NSOutlineView, heightOfRowByItem item: Any) -> CGFloat {
        54
    }

    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any)
        -> NSView?
    {
        guard let node = item as? SidebarNode else { return nil }

        switch node.kind {
        case let .chat(id):
            guard let chat = store.chat(id) else { return nil }
            let cell =
                outlineView.makeView(withIdentifier: SidebarChatCell.identifier, owner: self)
                as? SidebarChatCell ?? {
                    let new = SidebarChatCell()
                    new.identifier = SidebarChatCell.identifier
                    return new
                }()
            cell.configure(SidebarChatCell.Content(chat: chat, store: store))
            cell.shortcutNumber = nodes.firstIndex { $0 === node }.flatMap(shortcutNumber(forRow:))
            return cell

        case .header, .pane, .setting:
            return nil
        }
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !isApplyingSelection else { return }
        isNotifyingSelection = true
        defer { isNotifyingSelection = false }
        selection = currentSelection()
        onSelect?(selection)
    }
}

// MARK: - Context menu

extension SidebarViewController: NSMenuDelegate {
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()

        let row = outlineView.clickedRow
        guard row >= 0, let node = outlineView.item(atRow: row) as? SidebarNode,
            let selection = node.selection
        else { return }

        // Acting on the clicked row means selecting it first; the menu commands read the selection.
        setSelection(selection)
        onSelect?(selection)

        guard case let .chat(id) = node.kind else { return }
        let chat = store.chat(id)
        let pinned = chat?.isPinned ?? false
        menu.addItem(
            item(pinned ? L("Unpin") : L("Pin"), #selector(RootSplitViewController.togglePinChat(_:))))
        if chat?.isGroup == true {
            menu.addItem(item(L("Rename…"), #selector(RootSplitViewController.renameChat(_:))))
        }
        // Only groups take new members; a DM is fixed to its one bot.
        if chat?.isGroup == true {
            menu.addItem(item(L("Add Bot…"), #selector(RootSplitViewController.addBotToChat(_:))))
        }
        menu.addItem(.separator())
        menu.addItem(
            item(chat?.isDM == true ? L("Delete Bot") : L("Delete"),
                 #selector(RootSplitViewController.deleteChat(_:))))
    }

    private func item(_ title: String, _ action: Selector) -> NSMenuItem {
        let menuItem = NSMenuItem(title: title, action: action, keyEquivalent: "")
        menuItem.target = nil
        return menuItem
    }
}

// MARK: - Chrome

/// From macOS 26 a sidebar's list scrolls under the whole pane, as System Settings' does: the
/// bars above and below it are split view item accessories, which extend the safe area the list
/// is inset by and get AppKit's scroll-edge effect behind them. Earlier systems stack the bars
/// and the list inside the pane.
enum SidebarChrome {
    static var floats: Bool {
        if #available(macOS 26.0, *) { true } else { false }
    }
}

// MARK: - Search

/// The strip at the top of a sidebar holding a standard search field, which brings AppKit's
/// capsule, magnifier, clear button and focus ring.
final class SidebarSearchBar: NSView, NSSearchFieldDelegate {
    let field: NSSearchField = ActivatingSearchField()
    private var query = ""

    /// Set where the field is the way into a search elsewhere: a click calls this, and the field
    /// never takes the keyboard.
    var onActivate: (() -> Void)? {
        get { (field as? ActivatingSearchField)?.onActivate }
        set { (field as? ActivatingSearchField)?.onActivate = newValue }
    }

    var onQueryChange: ((String) -> Void)?
    /// Down arrow in the field: the list below takes the keyboard.
    var onMoveDown: (() -> Void)?
    /// Return in the field. Returns whether the bar's owner used it.
    var onSubmit: (() -> Bool)?

    init(placeholder: String = L("Search")) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        field.placeholderString = placeholder
        field.controlSize = .large
        field.sendsSearchStringImmediately = true
        field.sendsWholeSearchString = false
        field.delegate = self
        // The clear button empties the field without a text-change notification; it sends the
        // action instead.
        field.target = self
        field.action = #selector(queryChanged)
        field.translatesAutoresizingMaskIntoConstraints = false
        addSubview(field)

        NSLayoutConstraint.activate([
            // Room for the focus ring between the field and the first row.
            heightAnchor.constraint(equalToConstant: 40),
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            field.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func controlTextDidChange(_ obj: Notification) { queryChanged() }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        switch commandSelector {
        case #selector(NSResponder.cancelOperation(_:)):
            guard !field.stringValue.isEmpty else { return false }
            clear()
            return true
        case #selector(NSResponder.moveDown(_:)):
            guard let onMoveDown else { return false }
            onMoveDown()
            return true
        case #selector(NSResponder.insertNewline(_:)):
            return onSubmit?() ?? false
        default:
            return false
        }
    }

    func clear() {
        field.stringValue = ""
        queryChanged()
    }

    @objc private func queryChanged() {
        guard field.stringValue != query else { return }
        query = field.stringValue
        onQueryChange?(query)
    }
}

private final class ActivatingSearchField: NSSearchField {
    var onActivate: (() -> Void)?

    override var acceptsFirstResponder: Bool { onActivate == nil && super.acceptsFirstResponder }

    override func mouseDown(with event: NSEvent) {
        guard let onActivate else { return super.mouseDown(with: event) }
        onActivate()
    }
}

// MARK: - Footer

/// Two buttons at the foot of the sidebar: Settings, and this computer, whose icon turns red while
/// the CLI is not answering.
final class SidebarFooterView: NSView {
    private lazy var settings = HoverButton(
        symbol: "gearshape", tooltip: L("Settings (⌘,)"), target: self, action: #selector(openSettings))
    private lazy var device = HoverButton(
        symbol: "laptopcomputer", tooltip: "", target: self, action: #selector(openDevice))

    var onSettings: (() -> Void)?
    var onDevice: (() -> Void)?
    /// What the device button shows. The sidebar updates the footer on every store event, and
    /// one that changes none of it leaves the button alone.
    private var shown: (symbol: String, name: String, status: String)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        let buttons = Build.stack([settings, device], orientation: .horizontal, spacing: 4)
        addSubview(buttons)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 38),
            buttons.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            buttons.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func update() {
        let store = AppStore.shared
        let connected = store.isConnected
        let name = store.thisDevice?.name ?? L("This computer")
        let status = connected
            ? L("CLI on 127.0.0.1:%@", String(Preferences.cliPort))
            : L("CLI not running · start it with: lorca serve")
                .replacingOccurrences(of: "lorca serve", with: AppInfo.cliCommand)
        // Device symbols fill their screen in monochrome, which sits heavier than the gear's
        // outline; a palette with a clear second layer leaves the outline alone. The iMac's chin
        // stays solid either way, so a desktop shows as a plain display here.
        let symbol = store.thisDevice?.symbolName ?? "laptopcomputer"
        if let shown, shown == (symbol, name, status) { return }
        shown = (symbol, name, status)
        device.image = NSImage(
            systemSymbolName: symbol == "desktopcomputer" ? "display" : symbol, accessibilityDescription: name)
        let tint: NSColor = connected ? .secondaryLabelColor : .systemRed
        device.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 15, weight: .regular)
            .applying(.init(paletteColors: [tint, .clear]))
        device.toolTip = "\(name) · \(status)"
        device.setAccessibilityLabel(name)
    }

    @objc private func openSettings() { onSettings?() }
    @objc private func openDevice() { onDevice?() }
}
