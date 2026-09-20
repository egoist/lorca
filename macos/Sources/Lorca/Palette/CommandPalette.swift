import AppKit

/// ⌘K: a search field over a list of the menu bar's commands, the chats, and the settings, on a
/// panel that floats over the main window as its child. The field keeps the keyboard; the arrows
/// move the list's selection, Return runs it, and Escape or a click elsewhere closes the panel.
@MainActor
final class CommandPalette: NSObject {
    private enum Row {
        case header(String)
        case item(PaletteItem)
    }

    private enum Metric {
        static let width: CGFloat = 620
        static let fieldHeight: CGFloat = 52
        static let rowHeight: CGFloat = 36
        static let headerHeight: CGFloat = 26
        static let maxListHeight: CGFloat = 360
        static let emptyHeight: CGFloat = 64
    }

    private weak var parent: NSWindow?
    private let root: RootSplitViewController

    private let panel = PalettePanel(
        contentRect: NSRect(x: 0, y: 0, width: Metric.width, height: Metric.fieldHeight),
        styleMask: [.borderless, .fullSizeContentView], backing: .buffered, defer: true)
    private let field = NSTextField()
    private let separator = NSBox()
    private let scrollView = NSScrollView()
    private let tableView = PaletteTableView()
    private let emptyLabel = Build.label(L("No Results"), font: .systemFont(ofSize: 13), color: .secondaryLabelColor)

    private var sections: [PaletteSection] = []
    private var rows: [Row] = []
    private var isClosing = false

    var isVisible: Bool { panel.isVisible }

    init(parent: NSWindow, root: RootSplitViewController) {
        self.parent = parent
        self.root = root
        super.init()
        buildPanel()
    }

    // MARK: - Showing

    func toggle() {
        if isVisible { close() } else { show() }
    }

    func show() {
        guard let parent, parent.attachedSheet == nil else {
            NSSound.beep()
            return
        }
        // The menu validates against the main window's responder chain, so the commands are
        // read before the panel takes the keyboard.
        sections = PaletteIndex.sections(root: root, store: AppStore.shared)
        field.stringValue = ""
        reload()

        parent.addChildWindow(panel, ordered: .above)
        panel.makeKeyAndOrderFront(nil)
        panel.makeFirstResponder(field)
    }

    func close() {
        guard isVisible, !isClosing else { return }
        isClosing = true
        defer { isClosing = false }
        panel.parent?.removeChildWindow(panel)
        panel.orderOut(nil)
        parent?.makeKey()
    }

    private func run(_ item: PaletteItem) {
        close()
        // The main window is key again by the time the command looks for its target.
        DispatchQueue.main.async { item.run() }
    }

    // MARK: - Views

    private func buildPanel() {
        panel.isFloatingPanel = false
        panel.hidesOnDeactivate = false
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.isMovable = false
        panel.animationBehavior = .utilityWindow
        panel.delegate = self
        panel.setAccessibilityLabel(L("Command Palette"))

        let icon = NSImageView()
        icon.image = NSImage(systemSymbolName: "magnifyingglass", accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 18, weight: .regular)
        icon.contentTintColor = .secondaryLabelColor

        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 19)
        field.placeholderString = L("Search actions, chats, and settings")
        field.cell?.usesSingleLineMode = true
        field.cell?.isScrollable = true
        field.delegate = self

        separator.boxType = .separator

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("main"))
        column.resizingMask = .autoresizingMask
        tableView.addTableColumn(column)
        tableView.headerView = nil
        tableView.style = .inset
        tableView.backgroundColor = .clear
        tableView.intercellSpacing = .zero
        tableView.allowsEmptySelection = true
        tableView.allowsMultipleSelection = false
        tableView.refusesFirstResponder = true
        tableView.dataSource = self
        tableView.delegate = self
        tableView.target = self
        tableView.action = #selector(clickedRow)
        tableView.onHover = { [weak self] row in self?.select(row, scroll: false) }

        scrollView.documentView = tableView
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.scrollerStyle = .overlay
        scrollView.automaticallyAdjustsContentInsets = false

        let content = NSView()
        for view in [icon, field, separator, scrollView, emptyLabel] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            content.addSubview(view)
        }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: content.leadingAnchor, constant: 18),
            icon.centerYAnchor.constraint(equalTo: content.topAnchor, constant: Metric.fieldHeight / 2),
            icon.widthAnchor.constraint(equalToConstant: 22),

            field.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 8),
            field.trailingAnchor.constraint(equalTo: content.trailingAnchor, constant: -18),
            field.centerYAnchor.constraint(equalTo: icon.centerYAnchor),

            separator.topAnchor.constraint(equalTo: content.topAnchor, constant: Metric.fieldHeight),
            separator.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            separator.trailingAnchor.constraint(equalTo: content.trailingAnchor),

            scrollView.topAnchor.constraint(equalTo: separator.bottomAnchor),
            scrollView.leadingAnchor.constraint(equalTo: content.leadingAnchor),
            scrollView.trailingAnchor.constraint(equalTo: content.trailingAnchor),
            scrollView.bottomAnchor.constraint(equalTo: content.bottomAnchor),

            emptyLabel.centerXAnchor.constraint(equalTo: content.centerXAnchor),
            emptyLabel.centerYAnchor.constraint(
                equalTo: separator.bottomAnchor, constant: Metric.emptyHeight / 2),
        ])

        panel.contentView = Self.backdrop(holding: content)
    }

    /// Liquid Glass where AppKit has it, else the menu material, with the panel's corners.
    private static func backdrop(holding content: NSView) -> NSView {
        let radius: CGFloat = 18
        if #available(macOS 26.0, *) {
            let glass = NSGlassEffectView()
            glass.cornerRadius = radius
            glass.contentView = content
            // The window cuts its shadow from what it draws, and glass alone counts as its whole
            // rectangle: a clipping layer gives the panel, and so the shadow, the glass's corners.
            let clip = NSView()
            clip.wantsLayer = true
            clip.layer?.cornerRadius = radius
            clip.layer?.cornerCurve = .continuous
            clip.layer?.masksToBounds = true
            glass.translatesAutoresizingMaskIntoConstraints = false
            clip.addSubview(glass)
            glass.pin(to: clip)
            return clip
        }
        let effect = NSVisualEffectView()
        effect.material = .menu
        effect.blendingMode = .behindWindow
        effect.state = .active
        effect.wantsLayer = true
        effect.layer?.cornerRadius = radius
        effect.layer?.cornerCurve = .continuous
        effect.layer?.masksToBounds = true
        content.translatesAutoresizingMaskIntoConstraints = false
        effect.addSubview(content)
        content.pin(to: effect)
        return effect
    }

    // MARK: - List

    private func reload() {
        let matched = PaletteIndex.filter(sections, query: field.stringValue)
        rows = matched.flatMap { section in
            [.header(section.title)] + section.items.map { Row.item($0) }
        }
        tableView.reloadData()
        tableView.restingPointer = NSEvent.mouseLocation
        emptyLabel.isHidden = !rows.isEmpty
        scrollView.isHidden = rows.isEmpty
        layoutPanel()
        select(rows.firstIndex { if case .item = $0 { true } else { false } } ?? -1, scroll: false)
        tableView.scroll(.zero)
    }

    /// The panel is as tall as its rows, up to a limit, and keeps its top edge where it is: a
    /// fifth of the way down the main window, centered on it.
    private func layoutPanel() {
        guard let parent else { return }
        // An inset table pads its rows above and below, so the list is measured, not summed.
        let list = rows.isEmpty ? 0 : tableView.rect(ofRow: rows.count - 1).maxY + tableView.rect(ofRow: 0).minY
        let body = rows.isEmpty ? Metric.emptyHeight : min(list, Metric.maxListHeight)
        let height = Metric.fieldHeight + 1 + body
        let width = min(Metric.width, parent.frame.width - 40)
        let top = parent.frame.maxY - max(90, (parent.frame.height * 0.2).rounded())
        let frame = NSRect(x: (parent.frame.midX - width / 2).rounded(), y: top - height, width: width, height: height)
        panel.setFrame(frame, display: true)
        panel.invalidateShadow()
    }

    private func select(_ row: Int, scroll: Bool = true) {
        guard rows.indices.contains(row), case .item = rows[row] else {
            tableView.deselectAll(nil)
            return
        }
        guard tableView.selectedRow != row else { return }
        tableView.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
        if scroll { scrollToVisible(row) }
    }

    /// A section's first item brings its header along, so the list's top never hides one.
    private func scrollToVisible(_ row: Int) {
        if row > 0, case .header = rows[row - 1] { tableView.scrollRowToVisible(row - 1) }
        tableView.scrollRowToVisible(row)
    }

    /// The next item up or down, past the headers, wrapping at the ends.
    private func moveSelection(by delta: Int) {
        let items = rows.indices.filter { if case .item = rows[$0] { true } else { false } }
        guard !items.isEmpty else { return }
        let current = items.firstIndex(of: tableView.selectedRow)
        let next = current.map { ($0 + delta + items.count) % items.count } ?? (delta > 0 ? 0 : items.count - 1)
        select(items[next])
    }

    private func runSelection() {
        guard rows.indices.contains(tableView.selectedRow), case let .item(item) = rows[tableView.selectedRow]
        else { return }
        run(item)
    }

    @objc private func clickedRow() {
        guard rows.indices.contains(tableView.clickedRow), case let .item(item) = rows[tableView.clickedRow]
        else { return }
        run(item)
    }
}

// MARK: - Field

extension CommandPalette: NSTextFieldDelegate {
    func controlTextDidChange(_ notification: Notification) {
        reload()
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveDown(_:)): moveSelection(by: 1)
        case #selector(NSResponder.moveUp(_:)): moveSelection(by: -1)
        case #selector(NSResponder.insertNewline(_:)): runSelection()
        case #selector(NSResponder.cancelOperation(_:)): close()
        default: return false
        }
        return true
    }
}

extension CommandPalette: NSWindowDelegate {
    func windowDidResignKey(_ notification: Notification) {
        close()
    }
}

// MARK: - Table

extension CommandPalette: NSTableViewDataSource, NSTableViewDelegate {
    func numberOfRows(in tableView: NSTableView) -> Int { rows.count }

    func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat {
        if case .header = rows[row] { Metric.headerHeight } else { Metric.rowHeight }
    }

    func tableView(_ tableView: NSTableView, shouldSelectRow row: Int) -> Bool {
        if case .item = rows[row] { true } else { false }
    }

    func tableView(_ tableView: NSTableView, rowViewForRow row: Int) -> NSTableRowView? {
        PaletteRowView()
    }

    func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
        switch rows[row] {
        case let .header(title):
            let cell = tableView.makeView(withIdentifier: PaletteHeaderCell.identifier, owner: nil) as? PaletteHeaderCell
                ?? PaletteHeaderCell()
            cell.title.stringValue = title
            return cell
        case let .item(item):
            let cell = tableView.makeView(withIdentifier: PaletteItemCell.identifier, owner: nil) as? PaletteItemCell
                ?? PaletteItemCell()
            cell.configure(item)
            return cell
        }
    }
}

// MARK: - Panel and rows

/// A borderless panel takes the keyboard only when it says it can.
private final class PalettePanel: NSPanel {
    override var canBecomeKey: Bool { true }
    override var canBecomeMain: Bool { false }
}

/// The pointer selects the row under it, as in a menu.
private final class PaletteTableView: NSTableView {
    var onHover: ((Int) -> Void)?
    private var tracking: NSTrackingArea?

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: .zero, options: [.mouseMoved, .activeInKeyWindow, .inVisibleRect], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    /// Where the pointer was when the list last changed under it. Rows that appear or scroll
    /// beneath a resting pointer leave the selection alone; only a real move selects.
    var restingPointer = NSEvent.mouseLocation

    override func mouseMoved(with event: NSEvent) {
        let pointer = NSEvent.mouseLocation
        guard abs(pointer.x - restingPointer.x) > 1 || abs(pointer.y - restingPointer.y) > 1 else { return }
        restingPointer = NSPoint(x: CGFloat.infinity, y: CGFloat.infinity)
        let row = row(at: convert(event.locationInWindow, from: nil))
        if row >= 0 { onHover?(row) }
    }
}

/// The field holds the keyboard, so the list would draw its selection as an inactive one.
private final class PaletteRowView: NSTableRowView {
    override var isEmphasized: Bool {
        get { true }
        set {}
    }
}

private final class PaletteHeaderCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("PaletteHeaderCell")

    let title = Build.label("", font: .systemFont(ofSize: 11, weight: .semibold), color: .tertiaryLabelColor)

    init() {
        super.init(frame: .zero)
        identifier = Self.identifier
        title.translatesAutoresizingMaskIntoConstraints = false
        addSubview(title)
        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            title.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -4),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

private final class PaletteItemCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("PaletteItemCell")

    private static let slot: CGFloat = 22

    private let symbol = NSImageView()
    private let avatars = AvatarClusterView(slot: PaletteItemCell.slot)
    private let title = Build.label("", font: .systemFont(ofSize: 13))
    private let subtitle = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
    private let shortcut = Build.label("", font: .systemFont(ofSize: 13), color: .secondaryLabelColor)

    init() {
        super.init(frame: .zero)
        identifier = Self.identifier

        symbol.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .regular)
        symbol.imageAlignment = .alignCenter
        title.lineBreakMode = .byTruncatingTail
        subtitle.lineBreakMode = .byTruncatingTail
        title.setContentCompressionResistancePriority(.defaultLow + 1, for: .horizontal)
        subtitle.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        subtitle.setContentHuggingPriority(.defaultLow, for: .horizontal)
        shortcut.setContentCompressionResistancePriority(.required, for: .horizontal)
        shortcut.setContentHuggingPriority(.required, for: .horizontal)

        for view in [symbol, avatars, title, subtitle, shortcut] as [NSView] {
            view.translatesAutoresizingMaskIntoConstraints = false
            addSubview(view)
        }
        NSLayoutConstraint.activate([
            symbol.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 8),
            symbol.centerYAnchor.constraint(equalTo: centerYAnchor),
            symbol.widthAnchor.constraint(equalToConstant: Self.slot),
            symbol.heightAnchor.constraint(equalToConstant: Self.slot),
            avatars.centerXAnchor.constraint(equalTo: symbol.centerXAnchor),
            avatars.centerYAnchor.constraint(equalTo: centerYAnchor),

            title.leadingAnchor.constraint(equalTo: symbol.trailingAnchor, constant: 8),
            title.centerYAnchor.constraint(equalTo: centerYAnchor),
            subtitle.leadingAnchor.constraint(equalTo: title.trailingAnchor, constant: 8),
            subtitle.firstBaselineAnchor.constraint(equalTo: title.firstBaselineAnchor),
            subtitle.trailingAnchor.constraint(lessThanOrEqualTo: shortcut.leadingAnchor, constant: -12),
            shortcut.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            shortcut.firstBaselineAnchor.constraint(equalTo: title.firstBaselineAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(_ item: PaletteItem) {
        switch item.icon {
        case let .symbol(name):
            symbol.image = NSImage(systemSymbolName: name, accessibilityDescription: nil)
            avatars.isHidden = true
            symbol.isHidden = false
        case let .chat(bots):
            avatars.configure(with: bots)
            avatars.isHidden = false
            symbol.isHidden = true
        }
        title.stringValue = item.title
        subtitle.stringValue = item.subtitle
        shortcut.stringValue = item.shortcut
        syncTint()
    }

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { syncTint() }
    }

    /// The labels follow the row's selection on their own; a tinted symbol has to be told.
    private func syncTint() {
        symbol.contentTintColor = backgroundStyle == .emphasized ? .alternateSelectedControlTextColor : .secondaryLabelColor
    }
}
