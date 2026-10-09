import XCTest
@testable import Lorca

final class ReviewItemTests: XCTestCase {
    func testToolArgumentKeysAndLargeIntegerIdsStayExact() throws {
        let data = Data(#"{"kind":"plugin","plugin_id":"mail","server_name":"main","tool":"send","arguments":{"account_id":9223372036854775807,"draft_body":{"subject_line":"Reviewed text"}}}"#.utf8)
        let payload = try Wire.decoder.decode(ReviewItem.Payload.self, from: data)
        let params = try payload.parameters(editedText: payload.editorText)
        let arguments = try XCTUnwrap(params["arguments"] as? [String: Any])
        XCTAssertEqual((arguments["account_id"] as? NSNumber)?.int64Value, Int64.max)
        XCTAssertNil(arguments["accountId"])
        let body = try XCTUnwrap(arguments["draft_body"] as? [String: Any])
        XCTAssertEqual(body["subject_line"] as? String, "Reviewed text")
        XCTAssertEqual(params["server_name"] as? String, "main")
        XCTAssertEqual(params["tool"] as? String, "send")
    }

    func testDraftsAndCommandsEditAsTextAndCallsAsAnObject() throws {
        let draft = try Wire.decoder.decode(ReviewItem.Payload.self, from: Data(#"{"kind":"draft","text":"before"}"#.utf8))
        let params = try draft.parameters(editedText: "after\nwith another line")
        XCTAssertEqual(params["text"] as? String, "after\nwith another line")
        let shell = try Wire.decoder.decode(ReviewItem.Payload.self, from: Data(#"{"kind":"shell","arguments":{"command":"echo reviewed","description":"test","timeout_ms":120000}}"#.utf8))
        XCTAssertEqual(shell.editorText, "echo reviewed")
        let edited = try XCTUnwrap(shell.parameters(editedText: "echo edited")["arguments"] as? [String: Any])
        XCTAssertEqual(edited["command"] as? String, "echo edited")
        XCTAssertEqual(edited["description"] as? String, "test")
        XCTAssertEqual((edited["timeout_ms"] as? NSNumber)?.intValue, 120000)
        let call = try Wire.decoder.decode(ReviewItem.Payload.self, from: Data(#"{"kind":"plugin","plugin_id":"mail","server_name":"main","tool":"send","arguments":{}}"#.utf8))
        XCTAssertThrowsError(try call.parameters(editedText: "[1,2,3]"))
        XCTAssertThrowsError(try call.parameters(editedText: "broken json"))
    }
}
