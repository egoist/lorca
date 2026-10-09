import XCTest
@testable import Lorca

final class DraftCardTests: XCTestCase {
    /// A draft card as the CLI writes it: the review item it stands for, and the message's parts.
    func testADraftMessageDecodesWithItsParts() throws {
        let json = """
            {"id":"review-status-review-1","chat_id":"c1","author":{"kind":"bot","bot_id":"writer"},"state":{"kind":"complete"},"created_at":1,
             "body":{"kind":"draft","review_id":"review-1","version":2,"state":"pending","plugin_id":"gmail-work","account":"Gmail · Work",
                     "draft":{"kind":"email","to":["ana@example.com"],"cc":["bo@example.com"],"subject":"Lunch","body":"Noon?",
                              "attachments":[{"name":"menu.pdf","size":3}],"reply":"m-1"},"direct":false}}
            """
        let message = try Wire.decoder.decode(Wire.Message.self, from: Data(json.utf8)).toModel()
        guard case let .draft(card) = message.body else { return XCTFail("not a draft card") }
        XCTAssertEqual(card.reviewID, "review-1")
        XCTAssertEqual(card.version, 2)
        XCTAssertTrue(card.isPending)
        XCTAssertFalse(card.direct)
        XCTAssertEqual(card.fields.to, ["ana@example.com"])
        XCTAssertEqual(card.fields.attachments, [.init(name: "menu.pdf", size: 3)])
        XCTAssertEqual(card.title(botName: "Writer"), L("%@ drafted a reply", "Writer"))
        XCTAssertNil(card.stateText, "a draft that waits shows no state")
        XCTAssertEqual(card.fields.parameters["subject"] as? String, "Lunch")
    }

    func testAccessKeepsDraftsOnUnlessTurnedOff() throws {
        XCTAssertTrue(try Wire.decoder.decode(BotPermissions.self, from: Data("{}".utf8)).drafts)
        let off = try Wire.decoder.decode(BotPermissions.self, from: Data("{\"drafts\":false}".utf8))
        XCTAssertFalse(off.drafts)
        XCTAssertEqual(off.json["drafts"] as? Bool, false)
        XCTAssertEqual(off.summary, L("Full access"), "sending directly limits nothing")
    }
}
