import AppKit

final class SidebarViewController: NSViewController {
    private let store = AppStore.shared

    private let outlineView = NSOutlineView()
    private let scrollView = NSScrollView()
    /// The views above and below the list. Where the sidebar's chrome floats, the root hangs them
    /// on the split view item; otherwise they are laid out in this view.
    let searchBar = SidebarSearchBar()
    let footer = SidebarFooterView()

    private var nodes: [SidebarNode] = []
    private var searchQuery = ""
    private var isApplyingSelection = false
    private var isNotifyingSelection = false

    var onSelect: ((Selection?) -> Void)?
    var onDoubleClick: ((Selection) -> Void)?

    override func loadView() {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false

        configureOutlineView()

        scrollView.documentView = outlineView
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.drawsBackground = false
        scrollView.translatesAutoresizingMaskIntoConstraints = false
        scrollView.automaticallyAdjustsContentInsets = false
        scrollView.contentInsets = NSEdgeInsets(top: 7, left: 0, bottom: 8, right: 0)

        searchBar.onQueryChange = { [weak self] query in
            self?.setSearchQuery(query)
        }
        searchBar.onMoveDown = { [weak self] in
            self?.focusList()
        }

        // Both land in the settings sidebar, which lists the panes and this Mac's page.
        footer.onSettings = { [weak self] in
            self?.onSelect?(.settings(.general))
        }
        footer.onDevice = { [weak self] in
            guard let id = self?.store.thisDevice?.id else { return }
            self?.onSelect?(.device(id))
        }

        if SidebarChrome.floats {
            // The root hangs the search bar and the footer on the split view item, and the list
            // runs the pane's full height beneath them, inset by the safe area they extend.
            scrollView.automaticallyAdjustsContentInsets = true
            scrollView.contentInsets = NSEdgeInsets()
            // The gap between the search bar and the first chat.
            container.additionalSafeAreaInsets.top = 5
            container.addSubview(scrollView)
            scrollView.pin(to: container)
            view = container
            return
        }

        let divider = HairlineView()

        container.addSubview(searchBar)
        container.addSubview(scrollView)
        container.addSubview(divider)
        container.addSubview(footer)

        NSLayoutConstraint.activate([
            // The safe area keeps the field clear of the titlebar the sidebar runs under.
            searchBar.topAnchor.constraint(equalTo: container.safeAreaLayoutGuide.topAnchor),
            searchBar.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            searchBar.trailingAnchor.constraint(equalTo: container.trailingAnchor),

            scrollView.topAnchor.constraint(equalTo: searchBar.bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: divider.topAnchor),

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
        searchBar.window?.makeFirstResponder(searchBar.field)
    }

    func focusList() {
        view.window?.makeFirstResponder(outlineView)
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
    }

    // MARK: - Data

    private func rebuild() {
        let fresh = filteredChats().map { SidebarNode(.chat($0.id)) }

        if fresh.map(\.kind) == nodes.map(\.kind) {
            // Same rows in the same order (an unread count cleared, a pin toggled): update the
            // cells in place. A full reload replaces the row views, and a row view built while
            // the outline view is still handling the click that selected it draws its selection
            // unemphasized (gray) until the next selection change.
            refreshVisibleCells()
        } else if isNotifyingSelection {
            DispatchQueue.main.async { [weak self] in self?.rebuild() }
            return
        } else {
            let previous = currentSelection()
            nodes = fresh
            outlineView.reloadData()
            if let previous { setSelection(previous) }
        }
        footer.update()
    }

    private func refreshVisibleCells() {
        for row in 0..<outlineView.numberOfRows {
            guard let node = outlineView.item(atRow: row) as? SidebarNode,
                let cell = outlineView.view(atColumn: 0, row: row, makeIfNecessary: false)
            else { continue }
            if case let .chat(id) = node.kind, let cell = cell as? SidebarChatCell, let chat = store.chat(id) {
                cell.configure(chat: chat, store: store)
            }
        }
    }

    private func filteredChats() -> [Chat] {
        guard !searchQuery.isEmpty else { return store.chats }
        return store.chats.filter { chat in
            let haystack = [
                store.title(for: chat),
                store.preview(for: chat),
                store.bots(in: chat).map(\.name).joined(separator: " "),
            ].joined(separator: " ")
            return haystack.localizedCaseInsensitiveContains(searchQuery)
        }
    }

    func setSearchQuery(_ query: String) {
        searchQuery = query.trimmingCharacters(in: .whitespaces)
        rebuild()
    }

    // MARK: - Selection

    private func currentSelection() -> Selection? {
        let row = outlineView.selectedRow
        guard row >= 0, let node = outlineView.item(atRow: row) as? SidebarNode else { return nil }
        return node.selection
    }

    func setSelection(_ selection: Selection?) {
        guard let selection else {
            outlineView.deselectAll(nil)
            return
        }
        for row in 0..<outlineView.numberOfRows {
            guard let node = outlineView.item(atRow: row) as? SidebarNode,
                node.selection == selection
            else { continue }
            isApplyingSelection = true
            outlineView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
            isApplyingSelection = false
            return
        }
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
            cell.configure(chat: chat, store: store)
            return cell

        case .header, .pane, .setting, .device:
            return nil
        }
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !isApplyingSelection else { return }
        isNotifyingSelection = true
        defer { isNotifyingSelection = false }
        onSelect?(currentSelection())
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
            item(pinned ? "Unpin" : "Pin", #selector(RootSplitViewController.togglePinChat(_:))))
        menu.addItem(item("Rename…", #selector(RootSplitViewController.renameChat(_:))))
        // Only groups take new members; a DM is fixed to its one bot.
        if chat?.isGroup == true {
            menu.addItem(item("Add Bot…", #selector(RootSplitViewController.addBotToChat(_:))))
        }
        menu.addItem(.separator())
        menu.addItem(item("Delete", #selector(RootSplitViewController.deleteChat(_:))))
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
    let field = NSSearchField()
    private var query = ""

    var onQueryChange: ((String) -> Void)?
    /// Down arrow in the field: the list below takes the keyboard.
    var onMoveDown: (() -> Void)?
    /// Return in the field. Returns whether the bar's owner used it.
    var onSubmit: (() -> Bool)?

    init(placeholder: String = "Search") {
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

// MARK: - Footer

/// Two buttons at the foot of the sidebar: Settings, and this Mac, whose icon turns red while
/// the CLI is not answering.
final class SidebarFooterView: NSView {
    private lazy var settings = HoverButton(
        symbol: "gearshape", tooltip: "Settings (⌘,)", target: self, action: #selector(openSettings))
    private lazy var device = HoverButton(
        symbol: "laptopcomputer", tooltip: "", target: self, action: #selector(openDevice))

    var onSettings: (() -> Void)?
    var onDevice: (() -> Void)?

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
        let name = store.thisDevice?.name ?? "This Mac"
        let status = connected ? "CLI on 127.0.0.1:\(Preferences.cliPort)" : "CLI not running · start it with: lorca serve"
        // Device symbols fill their screen in monochrome, which sits heavier than the gear's
        // outline; a palette with a clear second layer leaves the outline alone. The iMac's chin
        // stays solid either way, so a desktop shows as a plain display here.
        let symbol = store.thisDevice?.symbolName ?? "laptopcomputer"
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
