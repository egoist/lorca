import AppKit

/// The sidebar while Settings is open: a search field, the panes, and Back in the footer, where
/// the chats' sidebar has the way in. A query narrows the
/// list to the panes and settings that match; a setting opens its pane with the row in view.
final class SettingsSidebarViewController: NSViewController {
    private let store = AppStore.shared

    private let outlineView = NSOutlineView()
    private let scrollView = NSScrollView()
    private let searchBar = SidebarSearchBar()
    /// The search field, in the place the chats' one has. Where the sidebar's chrome floats, the
    /// root hangs it on the split view item; otherwise it is laid out in this view.
    var header: NSView { searchBar }
    /// Back to the chats, in the place of the chats' footer. Hung on the split view item where
    /// the sidebar's chrome floats, like the header.
    private(set) lazy var footer: NSView = {
        let back = HoverButton(
            symbol: "chevron.left", pointSize: 12, title: "Back", tooltip: "Back to Chats (esc)", target: self,
            action: #selector(back))
        back.setAccessibilityLabel("Back to Chats")
        back.translatesAutoresizingMaskIntoConstraints = false
        let bar = NSView()
        bar.translatesAutoresizingMaskIntoConstraints = false
        bar.addSubview(back)
        NSLayoutConstraint.activate([
            bar.heightAnchor.constraint(equalToConstant: 38),
            back.leadingAnchor.constraint(equalTo: bar.leadingAnchor, constant: 10),
            back.centerYAnchor.constraint(equalTo: bar.centerYAnchor),
        ])
        return bar
    }()

    var onBack: (() -> Void)?

    @objc private func back() {
        onBack?()
    }
    private let noResults = Build.label(
        "", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0, alignment: .center)

    private var nodes: [SidebarNode] = []
    private var selection: Selection?
    /// The Device the Device panes show, whose bots, providers, and plugins a query searches.
    private var deviceID: Device.ID?
    private var searchQuery = ""
    private var isApplyingSelection = false
    private var isNotifyingSelection = false

    var onSelect: ((Selection) -> Void)?
    /// A search result was picked: its pane is selected, and this brings the row into view.
    var onReveal: ((SettingsEntry) -> Void)?

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
            self?.searchQuery = query.trimmingCharacters(in: .whitespaces)
            self?.rebuild()
        }
        searchBar.onMoveDown = { [weak self] in
            guard let self else { return }
            if outlineView.selectedRow < 0 { pickFirstResult() }
            focusList()
        }
        searchBar.onSubmit = { [weak self] in
            self?.pickFirstResult() ?? false
        }
        noResults.isHidden = true

        container.addSubview(scrollView)
        container.addSubview(noResults)

        if SidebarChrome.floats {
            // The list runs the pane's full height under the header, inset by the safe area.
            scrollView.automaticallyAdjustsContentInsets = true
            scrollView.contentInsets = NSEdgeInsets()
            // The gap between the search bar and the first pane, as the chats' sidebar has it.
            container.additionalSafeAreaInsets.top = 5
            scrollView.pin(to: container)
        } else {
            let divider = HairlineView()
            container.addSubview(header)
            container.addSubview(divider)
            container.addSubview(footer)
            NSLayoutConstraint.activate([
                header.topAnchor.constraint(equalTo: container.safeAreaLayoutGuide.topAnchor),
                header.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                header.trailingAnchor.constraint(equalTo: container.trailingAnchor),

                scrollView.topAnchor.constraint(equalTo: header.bottomAnchor),
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
        }

        NSLayoutConstraint.activate([
            noResults.topAnchor.constraint(
                equalTo: SidebarChrome.floats ? container.safeAreaLayoutGuide.topAnchor : header.bottomAnchor,
                constant: 24),
            noResults.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 16),
            noResults.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -16),
        ])

        view = container
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
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        rebuild()
        store.observe(self) { [weak self] event in
            switch event {
            case .rosterChanged, .snapshotReplaced: self?.rebuild()
            default: break
            }
        }
    }

    func focusList() {
        view.window?.makeFirstResponder(outlineView)
    }

    func focusSearch() {
        searchBar.window?.makeFirstResponder(searchBar.field)
    }

    /// Leaving Settings drops the query, so the next visit lists every page.
    func resetSearch() {
        guard isViewLoaded else { return }
        searchBar.clear()
    }

    // MARK: - Data

    private func rebuild() {
        var fresh: [SidebarNode] = []
        let device = deviceID.flatMap { store.device($0) }

        let results: [SettingsSearch.PaneResult] =
            if searchQuery.isEmpty {
                SettingsPane.allCases.map { .init(pane: $0, entries: []) }
            } else {
                SettingsSearch.panes(matching: searchQuery, device: device, store: store)
            }
        for result in results {
            fresh.append(SidebarNode(.pane(result.pane)))
            fresh += result.entries.map { SidebarNode(.setting($0)) }
        }

        noResults.stringValue = "No Results for \u{201C}\(searchQuery)\u{201D}"
        noResults.isHidden = !fresh.isEmpty

        if shape(of: fresh) == shape(of: nodes) {
            // Same rows: the row views and the selection's emphasis stay as they are.
        } else if isNotifyingSelection {
            DispatchQueue.main.async { [weak self] in self?.rebuild() }
        } else {
            nodes = fresh
            outlineView.reloadData()
            setSelection(selection)
        }
    }

    private func shape(of nodes: [SidebarNode]) -> [[SidebarNode.Kind]] {
        nodes.map { [$0.kind] + $0.children.map(\.kind) }
    }

    func setDevice(_ id: Device.ID?) {
        guard deviceID != id else { return }
        deviceID = id
        guard isViewLoaded, !searchQuery.isEmpty else { return }
        rebuild()
    }

    // MARK: - Selection

    func setSelection(_ newSelection: Selection?) {
        selection = newSelection
        guard isViewLoaded else { return }
        // A picked search result stands for its pane; the pane's own row sits above it.
        let selectedRow = outlineView.selectedRow
        if let newSelection, selectedRow >= 0,
            (outlineView.item(atRow: selectedRow) as? SidebarNode)?.selection == newSelection
        {
            return
        }
        for row in 0..<outlineView.numberOfRows {
            guard let node = outlineView.item(atRow: row) as? SidebarNode, let newSelection,
                node.selection == newSelection
            else { continue }
            isApplyingSelection = true
            outlineView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
            isApplyingSelection = false
            return
        }
    }

    /// Opens the best search result: the first matching setting, or else the first page listed.
    @discardableResult
    private func pickFirstResult() -> Bool {
        guard !searchQuery.isEmpty else { return false }
        let rows = (0..<outlineView.numberOfRows).filter {
            (outlineView.item(atRow: $0) as? SidebarNode)?.isHeader == false
        }
        let setting = rows.first {
            if case .setting = (outlineView.item(atRow: $0) as? SidebarNode)?.kind { return true }
            return false
        }
        guard let row = setting ?? rows.first else { return false }
        if outlineView.selectedRow == row {
            // Already selected, so no selection change will bring the row into view again.
            if case let .setting(entry) = (outlineView.item(atRow: row) as? SidebarNode)?.kind { onReveal?(entry) }
        } else {
            outlineView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        }
        outlineView.scrollRowToVisible(row)
        return true
    }
}

// MARK: - Data source

extension SettingsSidebarViewController: NSOutlineViewDataSource {
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

extension SettingsSidebarViewController: NSOutlineViewDelegate {
    func outlineView(_ outlineView: NSOutlineView, isGroupItem item: Any) -> Bool {
        (item as? SidebarNode)?.isHeader ?? false
    }

    func outlineView(_ outlineView: NSOutlineView, shouldShowOutlineCellForItem item: Any) -> Bool {
        false
    }

    func outlineView(_ outlineView: NSOutlineView, shouldSelectItem item: Any) -> Bool {
        !((item as? SidebarNode)?.isHeader ?? false)
    }

    func outlineView(_ outlineView: NSOutlineView, heightOfRowByItem item: Any) -> CGFloat {
        switch (item as? SidebarNode)?.kind {
        case .header: 28
        case .setting: 26
        default: 32
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

        case let .pane(pane):
            let cell =
                outlineView.makeView(withIdentifier: SidebarPaneCell.identifier, owner: self)
                as? SidebarPaneCell ?? {
                    let new = SidebarPaneCell()
                    new.identifier = SidebarPaneCell.identifier
                    return new
                }()
            cell.configure(pane: pane)
            return cell

        case let .setting(entry):
            let cell =
                outlineView.makeView(withIdentifier: SidebarSettingCell.identifier, owner: self)
                as? SidebarSettingCell ?? {
                    let new = SidebarSettingCell()
                    new.identifier = SidebarSettingCell.identifier
                    return new
                }()
            cell.configure(entry: entry)
            return cell

        case .chat:
            return nil
        }
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !isApplyingSelection else { return }
        let row = outlineView.selectedRow
        guard row >= 0, let node = outlineView.item(atRow: row) as? SidebarNode, let picked = node.selection
        else {
            // A click on empty space clears the row; Settings always shows one of its pages.
            setSelection(selection)
            return
        }
        selection = picked
        isNotifyingSelection = true
        defer { isNotifyingSelection = false }
        onSelect?(picked)
        if case let .setting(entry) = node.kind { onReveal?(entry) }
    }
}
