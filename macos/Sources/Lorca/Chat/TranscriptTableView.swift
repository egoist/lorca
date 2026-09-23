import AppKit

/// The transcript's table. It builds cells only for the rows on screen, and accessibility
/// clients see it the same way: one `TranscriptRowElement` per row, whose words, frame, and
/// index come from the chat. AppKit's own row elements would build a cell for every row a
/// client reads, since their cell element takes its children from
/// `view(atColumn:row:makeIfNecessary: true)`: a client walking the window (VoiceOver, the
/// Accessibility Inspector) had the table configure a cell for every message, once for each
/// attribute it read.
final class TranscriptTableView: NSTableView {
    /// The user stopped dragging the window's edge, so the rows measured at an old width catch up.
    var onLiveResizeEnd: (() -> Void)?
    /// What stays with a row while rows go in and out around it: a message's id.
    var rowIdentity: ((Int) -> AnyHashable)?
    /// What a screen reader says for a row.
    var rowSpokenText: ((Int) -> String)?
    /// Brings a row a screen reader moved to into view.
    var scrollRowIntoView: ((Int) -> Void)?

    private var rowElements: [TranscriptRowElement] = []
    private var rowElementsAreStale = true

    override func viewDidEndLiveResize() {
        super.viewDidEndLiveResize()
        onLiveResizeEnd?()
    }

    override func reloadData() {
        rowElementsAreStale = true
        super.reloadData()
    }

    override func insertRows(at indexes: IndexSet, withAnimation animationOptions: NSTableView.AnimationOptions = []) {
        rowElementsAreStale = true
        super.insertRows(at: indexes, withAnimation: animationOptions)
    }

    override func removeRows(at indexes: IndexSet, withAnimation animationOptions: NSTableView.AnimationOptions = []) {
        rowElementsAreStale = true
        super.removeRows(at: indexes, withAnimation: animationOptions)
    }

    /// An element for every row, in order, made when a client first asks. A row keeps its
    /// element while rows go in and out around it, so a screen reader stays on its message
    /// when a reply or an older page arrives. A row that goes away hands its element to a new
    /// row in its place, just above the next row that stayed: the day separator an older page
    /// pushes out of the top goes to the page's last message, and the working row to the
    /// status row that replaces it.
    var accessibilityRowElements: [TranscriptRowElement] {
        guard rowElementsAreStale || rowElements.count != numberOfRows else { return rowElements }
        rowElementsAreStale = false
        let identities = (0..<numberOfRows).map { rowIdentity?($0) ?? AnyHashable($0) }
        var byIdentity = Dictionary(rowElements.map { ($0.identity, $0) }, uniquingKeysWith: { first, _ in first })
        var elements = identities.map { byIdentity.removeValue(forKey: $0) }
        let stayed = Dictionary(
            elements.enumerated().compactMap { row, element in element.map { (ObjectIdentifier($0), row) } },
            uniquingKeysWith: { first, _ in first })
        var place = elements.count
        for element in rowElements.reversed() {
            if let row = stayed[ObjectIdentifier(element)] {
                place = row
            } else if place > 0, elements[place - 1] == nil {
                place -= 1
                element.identity = identities[place]
                elements[place] = element
            }
        }
        rowElements = elements.enumerated().map { row, element in
            let element = element ?? TranscriptRowElement(table: self, identity: identities[row])
            element.row = row
            return element
        }
        return rowElements
    }

    func accessibilityRowElement(at row: Int) -> TranscriptRowElement? {
        let elements = accessibilityRowElements
        return elements.indices.contains(row) ? elements[row] : nil
    }

    // MARK: - Accessibility

    private var visibleRowRange: Range<Int> {
        let visible = rows(in: visibleRect)
        return (visible.location..<NSMaxRange(visible)).clamped(to: 0..<numberOfRows)
    }

    override func accessibilityChildren() -> [Any]? {
        accessibilityRowElements + (accessibilityColumns() ?? [])
    }

    override func accessibilityChildrenInNavigationOrder() -> [any NSAccessibilityElementProtocol]? {
        accessibilityRowElements
    }

    override func accessibilityRows() -> [any NSAccessibilityRow]? {
        accessibilityRowElements
    }

    override func accessibilityVisibleRows() -> [any NSAccessibilityRow]? {
        Array(accessibilityRowElements[visibleRowRange])
    }

    override func accessibilitySelectedRows() -> [any NSAccessibilityRow]? {
        selectedRowIndexes.compactMap(accessibilityRowElement(at:))
    }

    /// A screen reader whose cursor moves keyboard focus selects the row it moves to, which
    /// AppKit's own setter cannot find among these elements.
    override func setAccessibilitySelectedRows(_ selectedRows: [any NSAccessibilityRow]) {
        let rows = selectedRows.compactMap { element in
            (element as? TranscriptRowElement).flatMap { accessibilityIndex(ofChild: $0) == NSNotFound ? nil : $0.row }
        }
        selectRowIndexes(IndexSet(rows), byExtendingSelection: false)
        if let row = rows.first { scrollRowIntoView?(row) }
    }

    override func accessibilitySelectedCells() -> [Any]? {
        selectedRowIndexes.compactMap { accessibilityRowElement(at: $0)?.cell }
    }

    override func accessibilityCell(forColumn column: Int, row: Int) -> Any? {
        column == 0 ? accessibilityRowElement(at: row)?.cell : nil
    }

    override func accessibilityIndex(ofChild child: Any) -> Int {
        guard let element = child as? TranscriptRowElement else { return super.accessibilityIndex(ofChild: child) }
        return accessibilityRowElement(at: element.row) === element ? element.row : NSNotFound
    }

    /// Clients read long arrays a page at a time, which AppKit answers with its own row
    /// elements unless told otherwise.
    private func accessibilityArray(_ attribute: NSAccessibility.Attribute) -> [Any]? {
        switch attribute {
        case .children: accessibilityChildren()
        case childrenInNavigationOrder: accessibilityChildrenInNavigationOrder()
        case .rows: accessibilityRows()
        case .visibleRows: accessibilityVisibleRows()
        case .selectedRows: accessibilitySelectedRows()
        case .selectedCells: accessibilitySelectedCells()
        default: nil
        }
    }

    override func accessibilityArrayAttributeCount(_ attribute: NSAccessibility.Attribute) -> Int {
        accessibilityArray(attribute)?.count ?? super.accessibilityArrayAttributeCount(attribute)
    }

    override func accessibilityArrayAttributeValues(
        _ attribute: NSAccessibility.Attribute, index: Int, maxCount: Int
    ) -> [Any] {
        guard let values = accessibilityArray(attribute) else {
            return super.accessibilityArrayAttributeValues(attribute, index: index, maxCount: maxCount)
        }
        guard index >= 0, index < values.count else { return [] }
        return Array(values[index..<(index + min(maxCount, values.count - index))])
    }
}

/// AppKit declares it without a Swift name.
private let childrenInNavigationOrder = NSAccessibility.Attribute(rawValue: "AXChildrenInNavigationOrder")

/// AXScrollToVisible, the action a screen reader performs on an element it moves to. AppKit
/// names it only from macOS 26.
private let scrollToVisibleAction = NSAccessibility.Action(rawValue: "AXScrollToVisible")

/// A transcript row as accessibility clients see it: an AXRow whose one cell is the row's real
/// cell while the table has one and a stand-in with the same words and frame while it has not.
final class TranscriptRowElement: NSAccessibilityElement, NSAccessibilityRow {
    fileprivate(set) var identity: AnyHashable
    /// Its place in the table, brought up to date whenever a client asks after the rows change.
    fileprivate(set) var row = 0
    private unowned let table: TranscriptTableView
    private lazy var standIn = TranscriptCellElement(row: self)

    fileprivate init(table: TranscriptTableView, identity: AnyHashable) {
        self.table = table
        self.identity = identity
        super.init()
    }

    var spokenText: String { table.rowSpokenText?(row) ?? "" }

    /// The row's cell: the one on screen, else the stand-in, so a client never makes the table
    /// build a cell.
    var cell: Any { table.view(atColumn: 0, row: row, makeIfNecessary: false) ?? standIn }

    var screenFrame: NSRect { NSAccessibility.screenRect(fromView: table, rect: table.rect(ofRow: row)) }

    func scrollIntoView() { table.scrollRowIntoView?(row) }

    override func accessibilityRole() -> NSAccessibility.Role? { .row }
    override func accessibilitySubrole() -> NSAccessibility.Subrole? { .tableRow }
    override func accessibilityParent() -> Any? { table }
    override func accessibilityIndex() -> Int { row }
    override func accessibilityFrame() -> NSRect { screenFrame }
    override func accessibilityChildren() -> [Any]? { [cell] }
    override func isAccessibilitySelected() -> Bool { table.isRowSelected(row) }
    override func accessibilityActionNames() -> [NSAccessibility.Action] { [scrollToVisibleAction] }

    override func setAccessibilitySelected(_ selected: Bool) {
        if selected {
            table.selectRowIndexes(IndexSet(integer: row), byExtendingSelection: false)
            scrollIntoView()
        } else {
            table.deselectRow(row)
        }
    }

    override func accessibilityPerformAction(_ action: NSAccessibility.Action) {
        if action == scrollToVisibleAction { scrollIntoView() }
    }

    /// AppKit finds a child's index for a client (AXIndexForChildUIElement) only for elements
    /// that answer this.
    override func accessibilityIsIgnored() -> Bool { false }

    /// Swift sees `accessibilityIdentifier` optional on the class and not on the row protocol;
    /// this one is both.
    override func accessibilityIdentifier() -> String { super.accessibilityIdentifier() ?? "" }
}

/// A row's cell while the table has not built the real one: the row's words over the row's frame.
private final class TranscriptCellElement: NSAccessibilityElement {
    private unowned let rowElement: TranscriptRowElement

    init(row: TranscriptRowElement) {
        rowElement = row
        super.init()
    }

    override func accessibilityRole() -> NSAccessibility.Role? { .cell }
    override func accessibilityParent() -> Any? { rowElement }
    override func accessibilityLabel() -> String? { rowElement.spokenText }
    override func accessibilityFrame() -> NSRect { rowElement.screenFrame }
    override func accessibilityRowIndexRange() -> NSRange { NSRange(location: rowElement.row, length: 1) }
    override func accessibilityColumnIndexRange() -> NSRange { NSRange(location: 0, length: 1) }
    override func accessibilityActionNames() -> [NSAccessibility.Action] { [scrollToVisibleAction] }

    override func accessibilityPerformAction(_ action: NSAccessibility.Action) {
        if action == scrollToVisibleAction { rowElement.scrollIntoView() }
    }

    /// A screen reader whose cursor moves keyboard focus focuses the cell it moves to.
    override func setAccessibilityFocused(_ focused: Bool) {
        if focused { rowElement.scrollIntoView() }
    }

    override func accessibilityIsIgnored() -> Bool { false }
}

/// A transcript cell as accessibility clients see it: the cell of its row's element, saying
/// what the row says. Cells move between rows as the user scrolls, so each ask looks the row up.
class TranscriptCellView: NSTableCellView {
    private var rowElement: TranscriptRowElement? {
        var ancestor = superview
        while let view = ancestor, !(view is TranscriptTableView) { ancestor = view.superview }
        guard let table = ancestor as? TranscriptTableView else { return nil }
        return table.accessibilityRowElement(at: table.row(for: self))
    }

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .cell }
    override func accessibilityParent() -> Any? { rowElement ?? super.accessibilityParent() }
    override func accessibilityLabel() -> String? { rowElement?.spokenText }
    override func accessibilityRowIndexRange() -> NSRange { NSRange(location: rowElement?.row ?? 0, length: 1) }
    override func accessibilityColumnIndexRange() -> NSRange { NSRange(location: 0, length: 1) }
    override func accessibilityActionNames() -> [NSAccessibility.Action] { [scrollToVisibleAction] }

    override func accessibilityPerformAction(_ action: NSAccessibility.Action) {
        if action == scrollToVisibleAction { rowElement?.scrollIntoView() }
    }
}
