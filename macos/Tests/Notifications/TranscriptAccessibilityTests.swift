import AppKit
import XCTest
@testable import Lorca

/// The transcript answers accessibility clients from the chat: reading every row builds no
/// cells beyond the ones on screen, and a row keeps its element while rows go in around it.
@MainActor
final class TranscriptAccessibilityTests: XCTestCase {
    private final class Source: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        var ids: [String]
        var configured: [Int] = []
        var scrolledTo: [Int] = []

        init(ids: [String]) { self.ids = ids }

        func numberOfRows(in tableView: NSTableView) -> Int { ids.count }
        func tableView(_ tableView: NSTableView, heightOfRow row: Int) -> CGFloat { 40 }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            configured.append(row)
            let identifier = NSUserInterfaceItemIdentifier("cell")
            let cell = tableView.makeView(withIdentifier: identifier, owner: nil) as? TranscriptCellView ?? TranscriptCellView()
            cell.identifier = identifier
            return cell
        }
    }

    private var window: NSWindow!
    private var table: TranscriptTableView!
    private var source: Source!

    override func setUp() async throws {
        source = Source(ids: (0..<300).map { "m\($0)" })
        table = TranscriptTableView()
        table.addTableColumn(NSTableColumn(identifier: NSUserInterfaceItemIdentifier("message")))
        table.headerView = nil
        table.dataSource = source
        table.delegate = source
        table.rowIdentity = { [unowned self] row in source.ids[row] }
        table.rowSpokenText = { [unowned self] row in "Chef: \(source.ids[row])" }
        table.scrollRowIntoView = { [unowned self] row in source.scrolledTo.append(row) }
        let scrollView = NSScrollView(frame: NSRect(x: 0, y: 0, width: 400, height: 400))
        scrollView.documentView = table
        window = NSWindow(contentRect: scrollView.frame, styleMask: .borderless, backing: .buffered, defer: false)
        window.setFrameOrigin(NSPoint(x: -4000, y: -4000))
        window.contentView = scrollView
        table.reloadData()
        table.scrollRowToVisible(source.ids.count - 1)
        window.displayIfNeeded()
    }

    override func tearDown() async throws {
        window.orderOut(nil)
        window = nil
    }

    /// What a client reads walking the tree: every element's children and words.
    private func walk(_ element: Any) {
        guard let element = element as? NSAccessibilityElementProtocol & NSObject else { return }
        _ = (element as? NSAccessibilityProtocol)?.accessibilityLabel()
        _ = element.accessibilityFrame()
        let children = (element as? NSAccessibilityProtocol)?.accessibilityChildren() ?? []
        for child in children { walk(child) }
    }

    func testReadingEveryRowBuildsNoCellsOffScreen() throws {
        let onScreen = Set(source.configured)
        XCTAssertLessThan(onScreen.count, 40)
        source.configured = []

        walk(table!)
        let rows = try XCTUnwrap(table.accessibilityRows())
        XCTAssertEqual(rows.count, 300)
        for (index, row) in rows.enumerated() {
            let cell = try XCTUnwrap((row as? NSAccessibilityProtocol)?.accessibilityChildren()?.first as? NSAccessibilityProtocol)
            XCTAssertEqual(cell.accessibilityLabel(), "Chef: m\(index)")
            XCTAssertEqual(cell.accessibilityRole(), .cell)
        }
        XCTAssertEqual(source.configured, [])
    }

    func testTheCellOnScreenIsTheRowsCell() throws {
        let last = try XCTUnwrap(table.accessibilityRowElement(at: 299))
        let cell = try XCTUnwrap(last.cell as? TranscriptCellView)
        XCTAssertTrue(cell === table.view(atColumn: 0, row: 299, makeIfNecessary: false))
        XCTAssertTrue(cell.accessibilityParent() as? TranscriptRowElement === last)
        XCTAssertEqual(cell.accessibilityLabel(), "Chef: m299")
        XCTAssertEqual(cell.accessibilityRowIndexRange(), NSRange(location: 299, length: 1))
    }

    func testARowKeepsItsElementWhenAnOlderPageArrives() throws {
        let first = try XCTUnwrap(table.accessibilityRowElement(at: 0))
        source.ids.insert(contentsOf: (0..<50).map { "older\($0)" }, at: 0)
        table.insertRows(at: IndexSet(integersIn: 0..<50))

        XCTAssertTrue(table.accessibilityRowElement(at: 50) === first)
        XCTAssertEqual(first.accessibilityIndex(), 50)
        XCTAssertEqual(table.accessibilityIndex(ofChild: first), 50)
        XCTAssertEqual((first.cell as? NSAccessibilityProtocol)?.accessibilityLabel(), "Chef: m0")
    }

    /// The first row is a day separator, which an older page can push out: a screen reader on
    /// it lands on the page's last row, where it was. The working row at the end gives way to
    /// a status row the same way.
    func testARowThatGoesAwayHandsItsElementToTheRowInItsPlace() throws {
        let top = try XCTUnwrap(table.accessibilityRowElement(at: 0))
        let next = try XCTUnwrap(table.accessibilityRowElement(at: 1))
        source.ids.replaceSubrange(0..<1, with: (0..<5).map { "older\($0)" })
        table.beginUpdates()
        table.removeRows(at: IndexSet(integer: 0))
        table.insertRows(at: IndexSet(integersIn: 0..<5))
        table.endUpdates()

        XCTAssertTrue(table.accessibilityRowElement(at: 4) === top)
        XCTAssertEqual((top.cell as? NSAccessibilityProtocol)?.accessibilityLabel(), "Chef: older4")
        XCTAssertTrue(table.accessibilityRowElement(at: 5) === next)

        let last = try XCTUnwrap(table.accessibilityRowElement(at: 303))
        source.ids[303] = "status"
        table.beginUpdates()
        table.removeRows(at: IndexSet(integer: 303))
        table.insertRows(at: IndexSet(integer: 303))
        table.endUpdates()

        XCTAssertTrue(table.accessibilityRowElement(at: 303) === last)
        XCTAssertEqual((last.cell as? NSAccessibilityProtocol)?.accessibilityLabel(), "Chef: status")
        XCTAssertEqual(Set(try XCTUnwrap(table.accessibilityRows()).map { ObjectIdentifier($0 as AnyObject) }).count, 304)
    }

    func testClientsPagingThroughRowsGetTheSameElements() throws {
        let rows = try XCTUnwrap(table.accessibilityRows()) as [Any]
        XCTAssertEqual(table.accessibilityArrayAttributeCount(.rows), 300)
        let page = table.accessibilityArrayAttributeValues(.rows, index: 298, maxCount: .max)
        XCTAssertEqual(page.count, 2)
        XCTAssertTrue(page[0] as AnyObject === rows[298] as AnyObject)
        XCTAssertEqual(table.accessibilityArrayAttributeValues(.children, index: 400, maxCount: 5).count, 0)
    }

    /// A screen reader moving keyboard focus with its cursor selects rows through these elements.
    func testSelectingARowSelectsItAndScrollsItIntoView() throws {
        let row = try XCTUnwrap(table.accessibilityRowElement(at: 12))
        table.setAccessibilitySelectedRows([row])
        XCTAssertEqual(Array(table.selectedRowIndexes), [12])
        XCTAssertTrue(row.isAccessibilitySelected())
        XCTAssertTrue(table.accessibilitySelectedRows()?.first as AnyObject === row)

        let other = try XCTUnwrap(table.accessibilityRowElement(at: 40))
        other.setAccessibilitySelected(true)
        XCTAssertEqual(Array(table.selectedRowIndexes), [40])
        XCTAssertEqual(source.scrolledTo, [12, 40])
    }

    func testMovingToARowScrollsItIntoView() throws {
        let row = try XCTUnwrap(table.accessibilityRowElement(at: 3))
        // What a client's AXScrollToVisible calls, without the deprecated Swift name.
        row.perform(NSSelectorFromString("accessibilityPerformAction:"), with: "AXScrollToVisible")
        (row.cell as? NSAccessibilityElement)?.setAccessibilityFocused(true)
        XCTAssertEqual(source.scrolledTo, [3, 3])
    }
}
