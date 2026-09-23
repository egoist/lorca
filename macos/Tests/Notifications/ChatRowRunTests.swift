import XCTest
@testable import Lorca

final class ChatRowRunTests: XCTestCase {
    private let today = ChatRow.day(Date(timeIntervalSince1970: 86_400))
    private let yesterday = ChatRow.day(Date(timeIntervalSince1970: 0))

    private func message(_ id: String, groupStart: Bool = true) -> ChatRow {
        .message(id: id, groupStart: groupStart)
    }

    func testAReplyGoesInAheadOfTheWorkingRow() {
        let old = [today, message("a"), .working(["bot"])]
        let new = [today, message("a"), message("b"), .working(["bot"])]
        let run = ChatRow.changedRun(from: old, to: new)
        XCTAssertEqual(run.removed, 2..<2)
        XCTAssertEqual(run.inserted, 2..<3)
    }

    func testTheWorkingRowGivesWayToTheStatusRow() {
        let old = [today, message("a"), .working(["bot"])]
        let new = [today, message("a"), .status("Chef stopped without replying")]
        let run = ChatRow.changedRun(from: old, to: new)
        XCTAssertEqual(run.removed, 2..<3)
        XCTAssertEqual(run.inserted, 2..<3)
    }

    /// The older page's last message is close to the old first one: that one loses its
    /// separator and its group start, and both go out with the page coming in.
    func testAnOlderPageReplacesTheStartOfTheTranscript() {
        let old = [today, message("c"), message("d", groupStart: false)]
        let new = [yesterday, message("a"), message("b", groupStart: false), message("c", groupStart: false),
                   message("d", groupStart: false)]
        let run = ChatRow.changedRun(from: old, to: new)
        XCTAssertEqual(run.removed, 0..<2)
        XCTAssertEqual(run.inserted, 0..<4)
    }

    func testARemovedMessageLeavesTheRowsAroundIt() {
        let old = [today, message("a"), message("b"), message("c")]
        let new = [today, message("a"), message("c")]
        let run = ChatRow.changedRun(from: old, to: new)
        XCTAssertEqual(run.removed, 2..<3)
        XCTAssertEqual(run.inserted, 2..<2)
    }

    func testTheSameRowsChangeNothing() {
        let rows = [today, message("a"), message("b", groupStart: false)]
        let run = ChatRow.changedRun(from: rows, to: rows)
        XCTAssertTrue(run.removed.isEmpty)
        XCTAssertTrue(run.inserted.isEmpty)
    }

    func testTheFirstMessageFillsAnEmptyChat() {
        let run = ChatRow.changedRun(from: [], to: [today, message("a")])
        XCTAssertEqual(run.removed, 0..<0)
        XCTAssertEqual(run.inserted, 0..<2)
    }
}
