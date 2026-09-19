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
        scrollView.contentInsets = NSEdgeInsets(top: 2, left: 0, bottom: 8, right: 0)

        searchBar.onQueryChange = { [weak self] query in
            self?.setSearchQuery(query)
        }

        // Both land in the settings sidebar, which lists the panes and this Mac's page.
        footer.onSettings = { [weak self] in
            self?.onSelect?(.settings(.general))
        }
        footer.onDevice = { [weak self] in
            guard let id = self?.store.thisDevice?.id else { return }
            self?.onSelect?(.device(id))
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

        case .header, .pane, .device:
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

// MARK: - Search

/// Flat search field: a tinted rounded rect with a magnifier and a clear button.
final class SidebarSearchBar: NSView, NSSearchFieldDelegate {
    let field = NSSearchField()
    private let background = BackgroundView()
    private let icon = NSImageView()
    private lazy var clearButton = Build.imageButton(
        symbol: "xmark.circle.fill", pointSize: 11, tooltip: "Clear search", target: self,
        action: #selector(clear))

    var onQueryChange: ((String) -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        background.fillColor = Theme.chipBackground
        background.cornerRadius = 7

        icon.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 12, weight: .regular)
        icon.contentTintColor = .secondaryLabelColor
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.setContentHuggingPriority(.required, for: .horizontal)

        field.placeholderString = "Search"
        field.font = .systemFont(ofSize: 13)
        field.isBezeled = false
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        // The bar draws its own magnifier and clear button.
        if let cell = field.cell as? NSSearchFieldCell {
            cell.searchButtonCell = nil
            cell.cancelButtonCell = nil
        }
        field.usesSingleLineMode = true
        field.cell?.isScrollable = true
        field.delegate = self
        field.translatesAutoresizingMaskIntoConstraints = false
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        clearButton.contentTintColor = .tertiaryLabelColor
        clearButton.isHidden = true

        addSubview(background)
        background.addSubview(icon)
        background.addSubview(field)
        background.addSubview(clearButton)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 36),
            background.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            background.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            background.centerYAnchor.constraint(equalTo: centerYAnchor),
            background.heightAnchor.constraint(equalToConstant: 28),

            icon.leadingAnchor.constraint(equalTo: background.leadingAnchor, constant: 8),
            icon.centerYAnchor.constraint(equalTo: background.centerYAnchor),

            field.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 6),
            field.trailingAnchor.constraint(equalTo: clearButton.leadingAnchor, constant: -4),
            field.centerYAnchor.constraint(equalTo: background.centerYAnchor),

            clearButton.trailingAnchor.constraint(equalTo: background.trailingAnchor, constant: -6),
            clearButton.centerYAnchor.constraint(equalTo: background.centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    // Clicking the magnifier or padding focuses the field.
    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(field)
    }

    func controlTextDidChange(_ obj: Notification) { queryChanged() }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy commandSelector: Selector) -> Bool {
        guard commandSelector == #selector(NSResponder.cancelOperation(_:)), !field.stringValue.isEmpty
        else { return false }
        clear()
        return true
    }

    @objc private func clear() {
        field.stringValue = ""
        queryChanged()
    }

    private func queryChanged() {
        clearButton.isHidden = field.stringValue.isEmpty
        onQueryChange?(field.stringValue)
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
