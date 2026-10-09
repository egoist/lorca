import AppKit
import XCTest
@testable import Lorca

@MainActor
final class OutputTests: XCTestCase {
    private func output(_ id: String, version: Int, minutesAgo: Double, evidence: Output.Evidence? = nil) -> Message {
        Message(
            id: "msg-\(id)-\(version)", author: .bot("bot-1"), body: .text(""), createdAt: Date().addingTimeInterval(-minutesAgo * 60),
            output: Output(id: id, name: "\(id).txt", mime: "text/plain", botId: "bot-1", version: version, evidence: evidence))
    }

    func testAnOutputMessageDecodesWithItsVersionAndCheck() throws {
        let raw = #"""
        {"id":"msg-output-v2","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},
         "body":{"kind":"text","text":"After screenshot · Passed: Checked layout","attachments":[{"id":"att-result2","name":"Screenshot.png","mime":"image/png","size":1024,"width":640,"height":480}]},
         "state":{"kind":"complete"},"created_at":1728000000,
         "output":{"id":"out-result","name":"Screenshot.png","mime":"image/png","bot_id":"bot-1","chat_id":"chat-1","task_id":"task-123","version":2,"previous_message_id":"msg-output-v1",
           "evidence":{"kind":"after_screenshot","summary":"Checked layout","status":"passed","command":"swift test","exit_code":0}}}
        """#
        let message = try Wire.decoder.decode(Wire.Message.self, from: Data(raw.utf8)).toModel()
        let output = try XCTUnwrap(message.output)
        XCTAssertEqual(output.id, "out-result")
        XCTAssertEqual(output.version, 2)
        XCTAssertEqual(output.botId, "bot-1")
        XCTAssertEqual(output.symbolName, "photo")
        XCTAssertEqual(output.evidence?.title, "After screenshot")
        XCTAssertEqual(output.evidence?.statusText, "Passed")
        XCTAssertEqual(output.evidence?.command, "swift test")
        XCTAssertEqual(message.attachments.first?.width, 640)
    }

    func testMessagesWithoutAnOutputDecodeAsBefore() throws {
        let raw = #"{"id":"msg-old","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},"body":{"kind":"text","text":"hello"},"state":{"kind":"complete"},"created_at":1728000000}"#
        let message = try Wire.decoder.decode(Wire.Message.self, from: Data(raw.utf8)).toModel()
        XCTAssertNil(message.output)
        XCTAssertEqual(message.text, "hello")
    }

    func testVersionsGroupUnderTheirOutputNewestFirst() {
        let failed = Output.Evidence(kind: "test_result", summary: "2 fail", status: "failed")
        let series = OutputSeries.group([
            output("out-report", version: 1, minutesAgo: 30, evidence: failed),
            output("out-chart", version: 1, minutesAgo: 20),
            output("out-report", version: 2, minutesAgo: 5),
            Message(author: .bot("bot-1"), body: .text("not an output")),
        ])
        XCTAssertEqual(series.map(\.id), ["out-report", "out-chart"], "the latest published first")
        XCTAssertEqual(series[0].versions.map { $0.output?.version }, [2, 1])
        XCTAssertNil(series[0].output.evidence, "the row shows the latest version")
        XCTAssertTrue(series[0].versions[1].output?.evidence?.failed == true)
    }

    func testAFileThatCouldNotBeFetchedRetriesOnAClick() {
        let tile = AttachmentTile()
        let attachment = Attachment(id: "att-gone", name: "After.png", mime: "image/png", size: 1200, width: 80, height: 60)
        var retries = 0
        tile.configure(attachment, url: nil, onUserBubble: false, error: "the relay no longer has After.png", onRetry: { retries += 1 })
        XCTAssertEqual(tile.toolTip, "After.png: the relay no longer has After.png")
        XCTAssertTrue(tile.accessibilityPerformPress())
        XCTAssertEqual(retries, 1)
        tile.configure(attachment, url: nil, onUserBubble: false, onRetry: { retries += 1 })
        XCTAssertTrue(tile.accessibilityPerformPress())
        XCTAssertEqual(retries, 1, "a file on its way does not retry")
    }

    func testOnlyHTTPSDocumentLinksOpen() {
        var output = Output(id: "out-link", name: "Report", mime: "text/html", botId: "bot-1", version: 1, url: "https://docs.example.com/report")
        XCTAssertEqual(output.documentURL?.host, "docs.example.com")
        XCTAssertEqual(output.symbolName, "link")
        for raw in ["file:///etc/passwd", "javascript:alert(1)", "https://user:secret@example.com", "http://example.com"] {
            output.url = raw
            XCTAssertNil(output.documentURL, raw)
        }
    }
}
