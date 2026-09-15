import AppKit

final class SidebarViewController: NSViewController {
    private let store = AppStore.shared

    private let outlineView = NSOutlineView()
    private let scrollView = NSScrollView()
    private let searchBar = SidebarSearchBar()
    private let footer = SidebarFooterView()

    private var nodes: [SidebarNode] = []
    private var searchQuery = ""
    private var isApplyingSelection = false

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
        scrollView.contentInsets = NSEdgeInsets(top: 2, left: 0, bottom: 8, right: 0)

        searchBar.onQueryChange = { [weak self] query in
            self?.setSearchQuery(query)
        }

        footer.onClick = { [weak self] in
            guard let id = self?.store.thisComputer?.id else { return }
            self?.onSelect?(.computer(id))
            self?.setSelection(.computer(id))
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
            footer.heightAnchor.constraint(equalToConstant: 38),
        ])

        view = container
    }

    func focusSearch() {
        view.window?.makeFirstResponder(searchBar.field)
    }

    private func configureOutlineView() {
        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("main"))
        column.resizingMask = .autoresizingMask
        outlineView.addTableColumn(column)
        outlineView.outlineTableColumn = column
        outlineView.headerView = nil
        outlineView.style = .sourceList
        outlineView.selectionHighlightStyle = .sourceList
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
        let previous = currentSelection()

        let chatsHeader = SidebarNode(.header("Chats"))
        chatsHeader.children = filteredChats().map { SidebarNode(.chat($0.id)) }

        let computersHeader = SidebarNode(.header("Computers"))
        computersHeader.children = filteredComputers().map { SidebarNode(.computer($0.id)) }

        nodes = [chatsHeader, computersHeader].filter { !$0.children.isEmpty || searchQuery.isEmpty }

        outlineView.reloadData()
        for node in nodes { outlineView.expandItem(node) }

        if let previous { setSelection(previous) }
        footer.update()
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

    private func filteredComputers() -> [Computer] {
        guard !searchQuery.isEmpty else { return store.computers }
        return store.computers.filter {
            $0.name.localizedCaseInsensitiveContains(searchQuery)
                || $0.model.localizedCaseInsensitiveContains(searchQuery)
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
        (item as? SidebarNode)?.isHeader ?? false
    }
}

// MARK: - Delegate

extension SidebarViewController: NSOutlineViewDelegate {
    func outlineView(_ outlineView: NSOutlineView, isGroupItem item: Any) -> Bool {
        (item as? SidebarNode)?.isHeader ?? false
    }

    func outlineView(_ outlineView: NSOutlineView, shouldSelectItem item: Any) -> Bool {
        !((item as? SidebarNode)?.isHeader ?? false)
    }

    func outlineView(_ outlineView: NSOutlineView, heightOfRowByItem item: Any) -> CGFloat {
        guard let node = item as? SidebarNode else { return 32 }
        switch node.kind {
        case .header: return 28
        case .chat: return 54
        case .computer: return 32
        }
    }

    func outlineView(_ outlineView: NSOutlineView, viewFor tableColumn: NSTableColumn?, item: Any)
        -> NSView?
    {
        guard let node = item as? SidebarNode else { return nil }

        switch node.kind {
        case let .header(title):
            let cell =
                outlineView.makeView(withIdentifier: SidebarHeaderCell.identifier, owner: self)
                as? SidebarHeaderCell ?? {
                    let new = SidebarHeaderCell()
                    new.identifier = SidebarHeaderCell.identifier
                    return new
                }()
            cell.configure(title)
            return cell

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

        case let .computer(id):
            guard let computer = store.computer(id) else { return nil }
            let cell =
                outlineView.makeView(withIdentifier: SidebarComputerCell.identifier, owner: self)
                as? SidebarComputerCell ?? {
                    let new = SidebarComputerCell()
                    new.identifier = SidebarComputerCell.identifier
                    return new
                }()
            cell.configure(computer: computer, store: store)
            return cell
        }
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !isApplyingSelection else { return }
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

        switch node.kind {
        case let .chat(id):
            let pinned = store.chat(id)?.isPinned ?? false
            menu.addItem(
                item(pinned ? "Unpin" : "Pin", #selector(RootSplitViewController.togglePinChat(_:))))
            menu.addItem(item("Rename…", #selector(RootSplitViewController.renameChat(_:))))
            menu.addItem(item("Add Bot…", #selector(RootSplitViewController.addBotToChat(_:))))
            menu.addItem(.separator())
            menu.addItem(item("Delete", #selector(RootSplitViewController.deleteChat(_:))))

        case .computer:
            menu.addItem(item("Pair a Computer…", #selector(AppDelegate.pairComputer(_:))))

        case .header:
            break
        }
    }

    private func item(_ title: String, _ action: Selector) -> NSMenuItem {
        let menuItem = NSMenuItem(title: title, action: action, keyEquivalent: "")
        menuItem.target = nil
        return menuItem
    }
}

// MARK: - Search

final class SidebarSearchBar: NSView {
    let field = NSSearchField()

    var onQueryChange: ((String) -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        field.placeholderString = "Search"
        field.sendsWholeSearchString = false
        field.sendsSearchStringImmediately = true
        field.translatesAutoresizingMaskIntoConstraints = false
        field.target = self
        field.action = #selector(searchChanged)
        field.setContentHuggingPriority(.defaultLow, for: .horizontal)

        addSubview(field)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 36),
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            field.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func searchChanged() { onQueryChange?(field.stringValue) }
}

// MARK: - Footer

final class SidebarFooterView: NSView {
    private let dot = StatusDotView(size: 7)
    private let label = Build.label("", font: .systemFont(ofSize: 11), color: .secondaryLabelColor)
    private let detail = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor)
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    var onClick: (() -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true

        let text = Build.stack([label, detail], spacing: 0)
        addSubview(dot)
        addSubview(text)

        NSLayoutConstraint.activate([
            dot.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 14),
            dot.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.leadingAnchor.constraint(equalTo: dot.trailingAnchor, constant: 8),
            text.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -10),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var allowsVibrancy: Bool { true }

    func update() {
        let store = AppStore.shared
        let connected = store.isConnected
        dot.status = connected ? .online : .offline
        label.stringValue = store.thisComputer?.name ?? "This Mac"
        detail.stringValue =
            connected
            ? "CLI on 127.0.0.1:\(Preferences.cliPort)"
            : "CLI not running"
        detail.textColor = connected ? .tertiaryLabelColor : .systemRed
        toolTip = connected ? "Connected to the local Tinybot CLI" : "Start the CLI with: tinybot serve"
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { isHovered = true }
    override func mouseExited(with event: NSEvent) { isHovered = false }
    override func mouseDown(with event: NSEvent) { onClick?() }

    override func draw(_ dirtyRect: NSRect) {
        guard isHovered else { return }
        NSColor.labelColor.withAlphaComponent(0.06).setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 6, dy: 4), xRadius: 6, yRadius: 6).fill()
    }
}
