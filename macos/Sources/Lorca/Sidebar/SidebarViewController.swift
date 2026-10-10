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

    /// The top-level rows: the chats (only the pinned ones once there are sections), then the
    /// groups, which hold the rest.
    private var nodes: [SidebarNode] = []
    /// A chat keeps its node while it is listed, so a row that moves stays the same item to the
    /// outline view, with its row view, cell, and selection. A group keeps its node the same way.
    private var chatNodes: [Chat.ID: SidebarNode] = [:]
    private var groupNodes: [SidebarGroup: SidebarNode] = [:]
    /// What the rows show, as last applied.
    private var layout: SidebarLayout?
    /// The root can restore selection before the first snapshot has created the outline rows.
    private var selection: Selection?
    private var isApplyingSelection = false
    private var isNotifyingSelection = false
    /// Set while the groups fold or unfold to follow the store, so the outline view's notices
    /// are not taken for the user's.
    private var isApplyingFolds = false
    /// Redraws the rows when the soonest mute runs out.
    private var muteExpiry: Timer?

    /// ⌘1–⌘9 open the first nine rows.
    private static let shortcutCount = 9
    private var flagsMonitor: Any?
    private var pendingShortcutHints: DispatchWorkItem?
    private var showsShortcutHints = false {
        didSet { if showsShortcutHints != oldValue { refreshShortcutHints() } }
    }

    var onSelect: ((Selection?) -> Void)?
    var onOpenDevice: ((Device.ID) -> Void)?
    var onOpenMarketplace: (() -> Void)?
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
        footer.onMarketplace = { [weak self] in self?.onOpenMarketplace?() }

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
        // Chats drag into sections, and sections drag into another order.
        outlineView.registerForDraggedTypes([.lorcaChat, .lorcaSection])
        outlineView.setDraggingSourceOperationMask(.move, forLocal: true)
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        rebuild()
        store.observe(self) { [weak self] event in
            switch event {
            case .chatsChanged, .snapshotReplaced, .chatChanged:
                self?.rebuild()
            case .connectionChanged, .rosterChanged:
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
        if !store.chats.isEmpty || !store.sections.isEmpty { installList() }

        let layout = SidebarLayout(
            chats: store.chats, sections: store.sections, showsHidden: Preferences.showsHiddenChats,
            collapsesOthers: Preferences.collapsesOtherChats)
        if listInstalled, layout != self.layout {
            // The outline view is still handling the click that selected a row; the rows move
            // once it is done.
            if isNotifyingSelection {
                DispatchQueue.main.async { [weak self] in self?.rebuild() }
                return
            }
            apply(layout)
        }
        refreshCells()
        scheduleMuteExpiry()
        footer.update()
    }

    /// Moves, inserts, and removes rows until they follow `layout`, and folds the groups it
    /// folds. Only the first rows come in with a reload: a reload builds every row view and cell
    /// again, and a row view built while the outline view handles the click that selected it
    /// draws its selection unemphasized (gray) until the next selection change.
    private func apply(_ layout: SidebarLayout) {
        let wasApplyingSelection = isApplyingSelection
        isApplyingSelection = true
        defer { isApplyingSelection = wasApplyingSelection }

        let top = layout.entries.map { entry in
            switch entry {
            case let .chat(id): node(for: id)
            case let .group(group): node(for: group.kind)
            }
        }
        let groups = layout.groups.map { (node(for: $0.kind), $0.chatIDs.map(node(for:))) }
        let listed = Set((top + groups.flatMap(\.1)).map(ObjectIdentifier.init))

        if nodes.isEmpty {
            nodes = top
            for (group, children) in groups { group.children = children }
            outlineView.reloadData()
        } else {
            var dropped: [SidebarNode] = []
            // Rows jump to their places, as a reload put them; the outline view would slide them.
            NSAnimationContext.runAnimationGroup { context in
                context.duration = 0
                outlineView.beginUpdates()
                // Rows listed nowhere go: those in a group first, then those at the top level,
                // a group with whatever rows it still holds.
                for parent in nodes where parent.group != nil {
                    remove(parent.children.indices.filter { !listed.contains(ObjectIdentifier(parent.children[$0])) }, from: parent)
                }
                dropped = remove(nodes.indices.filter { !listed.contains(ObjectIdentifier(nodes[$0])) }, from: nil)
                place(top, in: nil)
                for (group, children) in groups { place(children, in: group) }
                outlineView.endUpdates()
            }
            for group in dropped { group.children = [] }
        }
        chatNodes = chatNodes.filter { listed.contains(ObjectIdentifier($0.value)) }
        groupNodes = groupNodes.filter { listed.contains(ObjectIdentifier($0.value)) }
        self.layout = layout
        applyFolds(layout)
        setSelection(selection)
    }

    @discardableResult
    private func remove(_ indices: [Int], from parent: SidebarNode?) -> [SidebarNode] {
        guard !indices.isEmpty else { return [] }
        var removed: [SidebarNode] = []
        for index in indices.reversed() {
            if let parent {
                removed.append(parent.children.remove(at: index))
            } else {
                removed.append(nodes.remove(at: index))
            }
        }
        outlineView.removeItems(at: IndexSet(indices), inParent: parent, withAnimation: [])
        return removed
    }

    /// Top down, each place takes its row: the one already there, one moved up from further
    /// down or over from another group (a chat with new activity takes one move), or a new row.
    private func place(_ wanted: [SidebarNode], in parent: SidebarNode?) {
        for (index, node) in wanted.enumerated() {
            let siblings = parent?.children ?? nodes
            if index < siblings.count, siblings[index] === node { continue }
            if let (from, fromIndex) = location(of: node) {
                if let from { from.children.remove(at: fromIndex) } else { nodes.remove(at: fromIndex) }
                if let parent { parent.children.insert(node, at: index) } else { nodes.insert(node, at: index) }
                outlineView.moveItem(at: fromIndex, inParent: from, to: index, inParent: parent)
            } else {
                if let parent { parent.children.insert(node, at: index) } else { nodes.insert(node, at: index) }
                outlineView.insertItems(at: IndexSet(integer: index), inParent: parent, withAnimation: [])
            }
        }
    }

    /// Where a row is now: its group, or none at the top level, and its place there.
    private func location(of node: SidebarNode) -> (SidebarNode?, Int)? {
        if let index = nodes.firstIndex(where: { $0 === node }) { return (nil, index) }
        for parent in nodes where parent.group != nil {
            if let index = parent.children.firstIndex(where: { $0 === node }) { return (parent, index) }
        }
        return nil
    }

    private func parentGroup(of node: SidebarNode) -> SidebarNode? {
        location(of: node)?.0
    }

    /// The groups fold as the store says, at once.
    private func applyFolds(_ layout: SidebarLayout) {
        isApplyingFolds = true
        defer { isApplyingFolds = false }
        NSAnimationContext.runAnimationGroup { context in
            context.duration = 0
            for group in layout.groups {
                guard let node = groupNodes[group.kind] else { continue }
                if group.isCollapsed, outlineView.isItemExpanded(node) {
                    outlineView.collapseItem(node)
                } else if !group.isCollapsed, !outlineView.isItemExpanded(node) {
                    outlineView.expandItem(node)
                }
            }
        }
    }

    /// A group the user folded or unfolded: a section folds on every Device, the others here.
    private func remember(_ group: SidebarGroup, collapsed: Bool) {
        switch group {
        case let .section(id): store.setSectionCollapsed(id, collapsed)
        case .others: Preferences.collapsesOtherChats = collapsed
        case .hidden: Preferences.showsHiddenChats = !collapsed
        }
    }

    private func node(for id: Chat.ID) -> SidebarNode {
        if let node = chatNodes[id] { return node }
        let node = SidebarNode(.chat(id))
        chatNodes[id] = node
        return node
    }

    private func node(for group: SidebarGroup) -> SidebarNode {
        if let node = groupNodes[group] { return node }
        let node = SidebarNode(.group(group))
        groupNodes[group] = node
        return node
    }

    private func title(of group: SidebarGroup) -> String {
        switch group {
        case let .section(id): store.section(id)?.name ?? ""
        case .others: L("Chats", context: "no section")
        case .hidden: L("Hidden")
        }
    }

    /// Updates the rows the outline view holds cells for, on screen or kept ready beside it.
    /// The other rows get theirs when they scroll into view.
    private func refreshCells() {
        guard listInstalled else { return }
        outlineView.enumerateAvailableRowViews { rowView, row in
            guard let node = outlineView.item(atRow: row) as? SidebarNode else { return }
            if let group = node.group {
                (rowView.view(atColumn: 0) as? SidebarHeaderCell)?.configure(title(of: group))
                return
            }
            guard let cell = rowView.view(atColumn: 0) as? SidebarChatCell, let id = node.chatID,
                let chat = store.chat(id)
            else { return }
            cell.configure(SidebarChatCell.Content(chat: chat, store: store))
            cell.shortcutNumber = shortcutNumber(forRow: row)
        }
    }

    /// A muted row loses its mark when the mute runs out.
    private func scheduleMuteExpiry() {
        muteExpiry?.invalidate()
        muteExpiry = nil
        let now = Date()
        guard let soonest = store.chats.compactMap({ $0.mute?.until }).filter({ $0 > now }).min() else { return }
        muteExpiry = Timer.scheduledTimer(withTimeInterval: soonest.timeIntervalSince(now) + 0.5, repeats: false) { [weak self] _ in
            MainActor.assumeIsolated { self?.rebuild() }
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
        let previous = self.selection
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
        // A chat selected some other way (the palette, a notification) shows its row.
        if selection != previous { unfold(around: node) }
        let row = outlineView.row(forItem: node)
        guard row >= 0 else {
            outlineView.deselectAll(nil)
            return
        }
        guard row != outlineView.selectedRow else { return }
        outlineView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
    }

    /// A chat opened from the palette, a notification, or search shows its row, selected, even
    /// when it was already the chat on screen: its group unfolds.
    func reveal(_ id: Chat.ID) {
        guard listInstalled, let node = chatNodes[id] else { return }
        unfold(around: node)
        setSelection(.chat(id))
        scrollSelectionToVisible()
    }

    private func unfold(around node: SidebarNode) {
        guard let group = parentGroup(of: node), let kind = group.group, !outlineView.isItemExpanded(group) else { return }
        isApplyingFolds = true
        outlineView.expandItem(group)
        isApplyingFolds = false
        remember(kind, collapsed: false)
    }

    // MARK: - Shortcuts

    /// The chats as their rows show, top down.
    private var chatRows: [Int] {
        guard listInstalled else { return [] }
        return (0..<outlineView.numberOfRows).filter { (outlineView.item(atRow: $0) as? SidebarNode)?.chatID != nil }
    }

    /// The chat ⌘`number` opens: the chat row at that place in the list as it shows.
    func chatSelection(forShortcut number: Int) -> Selection? {
        guard (1...Self.shortcutCount).contains(number) else { return nil }
        let rows = chatRows
        guard number <= rows.count else { return nil }
        return (outlineView.item(atRow: rows[number - 1]) as? SidebarNode)?.selection
    }

    func scrollSelectionToVisible() {
        guard listInstalled, outlineView.selectedRow >= 0 else { return }
        outlineView.scrollRowToVisible(outlineView.selectedRow)
    }

    private func shortcutNumber(forRow row: Int) -> Int? {
        guard showsShortcutHints, let place = chatRows.prefix(Self.shortcutCount).firstIndex(of: row) else { return nil }
        return place + 1
    }

    private func refreshShortcutHints() {
        guard listInstalled else { return }
        for row in chatRows.prefix(Self.shortcutCount + 1) {
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

    // MARK: - Sections

    @objc func renameSection(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String, let section = store.section(id), let window = view.window else { return }
        SectionPrompt.name(title: L("Rename Section"), button: L("Rename"), initial: section.name, in: window) { [weak self] name in
            self?.store.renameSection(id, to: name)
        }
    }

    @objc func moveSectionUp(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String, let place = store.sections.firstIndex(where: { $0.id == id }) else { return }
        store.moveSection(id, to: place - 1)
    }

    @objc func moveSectionDown(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String, let place = store.sections.firstIndex(where: { $0.id == id }) else { return }
        store.moveSection(id, to: place + 1)
    }

    @objc func deleteSection(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String, let section = store.section(id), let window = view.window else { return }
        SectionPrompt.confirmDelete(section, in: window) { [weak self] in
            self?.store.deleteSection(id)
        }
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
        (item as? SidebarNode)?.group != nil
    }

    // MARK: Dragging

    func outlineView(_ outlineView: NSOutlineView, pasteboardWriterForItem item: Any) -> NSPasteboardWriting? {
        guard let node = item as? SidebarNode else { return nil }
        let writer = NSPasteboardItem()
        switch node.kind {
        case let .chat(id) where !store.sections.isEmpty: writer.setString(id, forType: .lorcaChat)
        case let .group(.section(id)): writer.setString(id, forType: .lorcaSection)
        default: return nil
        }
        return writer
    }

    func outlineView(
        _ outlineView: NSOutlineView, validateDrop info: NSDraggingInfo, proposedItem item: Any?, proposedChildIndex index: Int
    ) -> NSDragOperation {
        let pasteboard = info.draggingPasteboard
        if let id = pasteboard.string(forType: .lorcaChat), let chat = store.chat(id) {
            // Over a group, or a row in one: the chat goes into that group, which lights up whole.
            let target = (item as? SidebarNode).flatMap { $0.group != nil ? $0 : parentGroup(of: $0) }
            guard let target, let group = target.group, group != .hidden else { return [] }
            if !chat.isPinned, !chat.isHidden, group.sectionID == chat.sectionID.flatMap(store.section)?.id { return [] }
            outlineView.setDropItem(target, dropChildIndex: NSOutlineViewDropOnItemIndex)
            return .move
        }
        if let id = pasteboard.string(forType: .lorcaSection), let dragged = groupNodes[.section(id)] {
            // Between the sections only: over a group, the section goes before it.
            var place = index
            if let node = item as? SidebarNode {
                guard let group = node.group != nil ? node : parentGroup(of: node) else { return [] }
                place = nodes.firstIndex { $0 === group } ?? index
            }
            let sections = nodes.indices.filter { nodes[$0].group?.sectionID != nil }
            guard let first = sections.first, let last = sections.last else { return [] }
            place = max(first, min(last + 1, place))
            let from = nodes.firstIndex { $0 === dragged } ?? place
            if place == from || place == from + 1 { return [] }
            outlineView.setDropItem(nil, dropChildIndex: place)
            return .move
        }
        return []
    }

    func outlineView(
        _ outlineView: NSOutlineView, acceptDrop info: NSDraggingInfo, item: Any?, childIndex index: Int
    ) -> Bool {
        let pasteboard = info.draggingPasteboard
        if let id = pasteboard.string(forType: .lorcaChat), let group = (item as? SidebarNode)?.group, group != .hidden {
            guard store.chat(id) != nil else { return false }
            store.moveChat(id, toSection: group.sectionID)
            return true
        }
        if let id = pasteboard.string(forType: .lorcaSection), item == nil {
            let before = nodes[..<min(index, nodes.count)].filter { $0.group?.sectionID != nil && $0.group?.sectionID != id }
            store.moveSection(id, to: before.count)
            return true
        }
        return false
    }
}

// MARK: - Delegate

extension SidebarViewController: NSOutlineViewDelegate {
    func outlineView(_ outlineView: NSOutlineView, heightOfRowByItem item: Any) -> CGFloat {
        (item as? SidebarNode)?.group != nil ? 28 : 54
    }

    func outlineView(_ outlineView: NSOutlineView, isGroupItem item: Any) -> Bool {
        (item as? SidebarNode)?.group != nil
    }

    func outlineView(_ outlineView: NSOutlineView, shouldSelectItem item: Any) -> Bool {
        (item as? SidebarNode)?.chatID != nil
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
            cell.shortcutNumber = shortcutNumber(forRow: outlineView.row(forItem: node))
            return cell

        case let .group(group):
            let cell =
                outlineView.makeView(withIdentifier: SidebarHeaderCell.identifier, owner: self)
                as? SidebarHeaderCell ?? {
                    let new = SidebarHeaderCell()
                    new.identifier = SidebarHeaderCell.identifier
                    return new
                }()
            cell.configure(title(of: group))
            return cell

        case .header, .pane, .setting:
            return nil
        }
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !isApplyingSelection else { return }
        let picked = currentSelection()
        // Folding a group away deselects the row of the chat on screen, which stays open; its row
        // is selected again when the group unfolds.
        if picked == nil, case let .chat(id) = selection, let node = chatNodes[id], outlineView.row(forItem: node) < 0 { return }
        isNotifyingSelection = true
        defer { isNotifyingSelection = false }
        selection = picked
        onSelect?(selection)
    }

    // A group folded or unfolded by the user.

    func outlineViewItemDidCollapse(_ notification: Notification) {
        guard !isApplyingFolds else { return }
        if let group = (notification.userInfo?["NSObject"] as? SidebarNode)?.group { remember(group, collapsed: true) }
    }

    func outlineViewItemDidExpand(_ notification: Notification) {
        guard !isApplyingFolds else { return }
        if let group = (notification.userInfo?["NSObject"] as? SidebarNode)?.group { remember(group, collapsed: false) }
        setSelection(selection)
    }
}

// MARK: - Context menu

extension SidebarViewController: NSMenuDelegate, NSMenuItemValidation {
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()

        let row = outlineView.clickedRow
        guard row >= 0, let node = outlineView.item(atRow: row) as? SidebarNode else { return }
        if let group = node.group {
            addItems(for: group, to: menu)
            return
        }
        guard let selection = node.selection else { return }

        // Acting on the clicked row means selecting it first; the menu commands read the selection.
        setSelection(selection)
        onSelect?(selection)

        guard case let .chat(id) = node.kind, let chat = store.chat(id) else { return }
        menu.addItem(item(chat.isPinned ? L("Unpin") : L("Pin"), #selector(RootSplitViewController.togglePinChat(_:))))
        menu.addItem(ChatMenus.muteItem(for: chat))
        menu.addItem(ChatMenus.sectionItem(for: chat, store: store))
        menu.addItem(item(chat.isHidden ? L("Show in Sidebar") : L("Hide"), #selector(RootSplitViewController.toggleHideChat(_:))))
        menu.addItem(.separator())
        if chat.isGroup {
            menu.addItem(item(L("Rename…"), #selector(RootSplitViewController.renameChat(_:))))
            // Only groups take new members; a DM is fixed to its one bot.
            menu.addItem(item(L("Add Bot…"), #selector(RootSplitViewController.addBotToChat(_:))))
        } else {
            menu.addItem(item(L("Share as Template…"), #selector(RootSplitViewController.shareBotTemplate(_:))))
        }
        menu.addItem(.separator())
        menu.addItem(
            item(chat.isBotDM ? L("Delete Bot") : L("Delete"),
                 #selector(RootSplitViewController.deleteChat(_:))))
    }

    /// A section's header renames, moves, and deletes it; the Chats header starts a section.
    private func addItems(for group: SidebarGroup, to menu: NSMenu) {
        switch group {
        case let .section(id):
            menu.addItem(sectionItem(L("Rename Section…"), #selector(renameSection(_:)), id))
            menu.addItem(sectionItem(L("Move Up"), #selector(moveSectionUp(_:)), id))
            menu.addItem(sectionItem(L("Move Down"), #selector(moveSectionDown(_:)), id))
            menu.addItem(.separator())
            menu.addItem(sectionItem(L("Delete Section…"), #selector(deleteSection(_:)), id))
        case .others:
            menu.addItem(item(L("New Section…"), #selector(RootSplitViewController.newSection(_:))))
        case .hidden:
            break
        }
    }

    func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        guard let id = menuItem.representedObject as? String, let place = store.sections.firstIndex(where: { $0.id == id }) else {
            return true
        }
        switch menuItem.action {
        case #selector(moveSectionUp(_:)): return place > 0
        case #selector(moveSectionDown(_:)): return place < store.sections.count - 1
        default: return true
        }
    }

    private func item(_ title: String, _ action: Selector) -> NSMenuItem {
        let menuItem = NSMenuItem(title: title, action: action, keyEquivalent: "")
        menuItem.target = nil
        return menuItem
    }

    private func sectionItem(_ title: String, _ action: Selector, _ id: SidebarSection.ID) -> NSMenuItem {
        let menuItem = NSMenuItem(title: title, action: action, keyEquivalent: "")
        menuItem.target = self
        menuItem.representedObject = id
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
/// the CLI is not answering and orange while the CLI cannot connect to the relay, with the error
/// in its tooltip.
final class SidebarFooterView: NSView {
    private lazy var settings = HoverButton(
        symbol: "gearshape", tooltip: L("Settings (⌘,)"), target: self, action: #selector(openSettings))
    private lazy var device = HoverButton(
        symbol: "laptopcomputer", tooltip: "", target: self, action: #selector(openDevice))
    /// The marketplace, at the footer's other end.
    private lazy var marketplace = HoverButton(
        symbol: "circle.grid.2x2", tooltip: L("Marketplace (⇧⌘M)"), target: self, action: #selector(openMarketplace))

    var onSettings: (() -> Void)?
    var onDevice: (() -> Void)?
    var onMarketplace: (() -> Void)?
    /// What the device button shows. The sidebar updates the footer on every store event, and
    /// one that changes none of it leaves the button alone.
    private var shown: (symbol: String, name: String, status: String)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        let buttons = Build.stack([settings, device], orientation: .horizontal, spacing: 4)
        addSubview(buttons)
        marketplace.translatesAutoresizingMaskIntoConstraints = false
        addSubview(marketplace)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 38),
            buttons.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 10),
            buttons.centerYAnchor.constraint(equalTo: centerYAnchor),
            marketplace.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            marketplace.centerYAnchor.constraint(equalTo: buttons.centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func update() {
        let store = AppStore.shared
        let connected = store.isConnected
        let name = store.thisDevice?.name ?? L("This computer")
        let relayError = connected ? store.relayError : nil
        let status =
            if !connected {
                L("CLI not running · start it with: lorca serve")
                    .replacingOccurrences(of: "lorca serve", with: AppInfo.cliCommand)
            } else if let relayError {
                L("Can’t connect to the relay: %@", relayError)
            } else {
                L("CLI on 127.0.0.1:%@", String(Preferences.cliPort))
            }
        // Device symbols fill their screen in monochrome, which sits heavier than the gear's
        // outline; a palette with a clear second layer leaves the outline alone. The iMac's chin
        // stays solid either way, so a desktop shows as a plain display here.
        let symbol = store.thisDevice?.symbolName ?? "laptopcomputer"
        if let shown, shown == (symbol, name, status) { return }
        shown = (symbol, name, status)
        device.image = NSImage(
            systemSymbolName: symbol == "desktopcomputer" ? "display" : symbol, accessibilityDescription: name)
        let tint: NSColor = !connected ? .systemRed : relayError != nil ? .systemOrange : .secondaryLabelColor
        device.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 15, weight: .regular)
            .applying(.init(paletteColors: [tint, .clear]))
        device.toolTip = "\(name) · \(status)"
        device.setAccessibilityLabel(name)
    }

    @objc private func openSettings() { onSettings?() }
    @objc private func openDevice() { onDevice?() }
    @objc private func openMarketplace() { onMarketplace?() }
}
