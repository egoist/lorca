import AppKit

/// The sidebar while Settings is open: Back, the panes, and the paired Devices, each Device
/// opening its own page.
final class SettingsSidebarViewController: NSViewController {
    private let store = AppStore.shared

    private let outlineView = NSOutlineView()
    private let scrollView = NSScrollView()
    private let backBar = SidebarBackBar()

    private static let devicesTitle = "Devices"

    private var nodes: [SidebarNode] = []
    private var selection: Selection?
    private var isApplyingSelection = false
    private var isNotifyingSelection = false

    var onSelect: ((Selection) -> Void)?
    var onBack: (() -> Void)?

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

        backBar.onClick = { [weak self] in self?.onBack?() }

        container.addSubview(backBar)
        container.addSubview(scrollView)

        NSLayoutConstraint.activate([
            // Pane content under the titlebar gets no clicks, so Back starts at the safe area.
            backBar.topAnchor.constraint(equalTo: container.safeAreaLayoutGuide.topAnchor),
            backBar.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            backBar.trailingAnchor.constraint(equalTo: container.trailingAnchor),

            scrollView.topAnchor.constraint(equalTo: backBar.bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
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
        let menu = NSMenu()
        menu.delegate = self
        outlineView.menu = menu
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

    // MARK: - Data

    private func rebuild() {
        let settingsHeader = SidebarNode(.header("Settings"))
        settingsHeader.children = SettingsPane.allCases.map { SidebarNode(.pane($0)) }

        let devicesHeader = SidebarNode(.header(Self.devicesTitle))
        devicesHeader.children = store.devices.map { SidebarNode(.device($0.id)) }

        let fresh = [settingsHeader, devicesHeader]

        if shape(of: fresh) == shape(of: nodes) {
            // Same rows (a Device came online, a bot moved): update the cells in place, which
            // keeps the row views and the selection's emphasis.
            refreshVisibleCells()
        } else if isNotifyingSelection {
            DispatchQueue.main.async { [weak self] in self?.rebuild() }
        } else {
            nodes = fresh
            outlineView.reloadData()
            for node in nodes { outlineView.expandItem(node) }
            setSelection(selection)
        }
    }

    private func shape(of nodes: [SidebarNode]) -> [[SidebarNode.Kind]] {
        nodes.map { [$0.kind] + $0.children.map(\.kind) }
    }

    private func refreshVisibleCells() {
        for row in 0..<outlineView.numberOfRows {
            guard let node = outlineView.item(atRow: row) as? SidebarNode,
                case let .device(id) = node.kind, let device = store.device(id),
                let cell = outlineView.view(atColumn: 0, row: row, makeIfNecessary: false) as? SidebarDeviceCell
            else { continue }
            cell.configure(device: device, store: store)
        }
    }

    // MARK: - Selection

    func setSelection(_ newSelection: Selection?) {
        selection = newSelection
        guard isViewLoaded else { return }
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

    // The Devices header holds the Pair button where a group row's Show/Hide control would go.
    func outlineView(_ outlineView: NSOutlineView, shouldShowOutlineCellForItem item: Any) -> Bool {
        false
    }

    func outlineView(_ outlineView: NSOutlineView, shouldSelectItem item: Any) -> Bool {
        !((item as? SidebarNode)?.isHeader ?? false)
    }

    func outlineView(_ outlineView: NSOutlineView, heightOfRowByItem item: Any) -> CGFloat {
        (item as? SidebarNode)?.isHeader == true ? 28 : 32
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
            if title == Self.devicesTitle {
                cell.configure(title, actionTooltip: "Pair a Device…") {
                    NSApp.sendAction(#selector(AppDelegate.pairDevice(_:)), to: nil, from: nil)
                }
            } else {
                cell.configure(title)
            }
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

        case let .device(id):
            guard let device = store.device(id) else { return nil }
            let cell =
                outlineView.makeView(withIdentifier: SidebarDeviceCell.identifier, owner: self)
                as? SidebarDeviceCell ?? {
                    let new = SidebarDeviceCell()
                    new.identifier = SidebarDeviceCell.identifier
                    return new
                }()
            cell.configure(device: device, store: store)
            return cell

        case .chat:
            return nil
        }
    }

    func outlineViewSelectionDidChange(_ notification: Notification) {
        guard !isApplyingSelection else { return }
        let row = outlineView.selectedRow
        guard row >= 0, let picked = (outlineView.item(atRow: row) as? SidebarNode)?.selection else {
            // A click on empty space clears the row; Settings always shows one of its pages.
            setSelection(selection)
            return
        }
        selection = picked
        isNotifyingSelection = true
        defer { isNotifyingSelection = false }
        onSelect?(picked)
    }
}

// MARK: - Context menu

extension SettingsSidebarViewController: NSMenuDelegate {
    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()

        let row = outlineView.clickedRow
        guard row >= 0, let node = outlineView.item(atRow: row) as? SidebarNode,
            case let .device(id) = node.kind
        else { return }

        // Unpair reads the selection, so the clicked Device is selected first.
        setSelection(.device(id))
        onSelect?(.device(id))

        menu.addItem(item("Pair a Device…", #selector(AppDelegate.pairDevice(_:))))
        if store.device(id)?.isThisDevice == false {
            menu.addItem(.separator())
            menu.addItem(item("Unpair…", #selector(RootSplitViewController.unpairDevice(_:))))
        }
    }

    private func item(_ title: String, _ action: Selector) -> NSMenuItem {
        let menuItem = NSMenuItem(title: title, action: action, keyEquivalent: "")
        menuItem.target = nil
        return menuItem
    }
}

// MARK: - Back

/// The strip where the main sidebar has its search field: a chevron and "Back", returning to chats.
final class SidebarBackBar: NSView {
    private let chevron = NSImageView()
    private let label = Build.label("Back", font: .systemFont(ofSize: 13), color: .secondaryLabelColor)
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }
    private var isPressed = false { didSet { needsDisplay = true } }

    var onClick: (() -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true

        chevron.image = NSImage(systemSymbolName: "chevron.left", accessibilityDescription: nil)
        chevron.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 12, weight: .medium)
        chevron.contentTintColor = .secondaryLabelColor
        chevron.translatesAutoresizingMaskIntoConstraints = false

        addSubview(chevron)
        addSubview(label)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(equalToConstant: 36),
            chevron.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 18),
            chevron.centerYAnchor.constraint(equalTo: centerYAnchor),
            label.leadingAnchor.constraint(equalTo: chevron.trailingAnchor, constant: 6),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
            label.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -10),
        ])

        setAccessibilityElement(true)
        setAccessibilityRole(.button)
        setAccessibilityLabel("Back to Chats")
        toolTip = "Back to Chats (esc)"
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var allowsVibrancy: Bool { true }

    override func accessibilityPerformPress() -> Bool {
        onClick?()
        return true
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

    // The click counts on release inside the bar, like a button.
    override func mouseDown(with event: NSEvent) { isPressed = true }

    override func mouseUp(with event: NSEvent) {
        isPressed = false
        if bounds.contains(convert(event.locationInWindow, from: nil)) { onClick?() }
    }

    override func draw(_ dirtyRect: NSRect) {
        guard isHovered || isPressed else { return }
        NSColor.labelColor.withAlphaComponent(isPressed ? 0.1 : 0.06).setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 10, dy: 4), xRadius: 7, yRadius: 7).fill()
    }
}
