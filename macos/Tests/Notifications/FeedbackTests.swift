import XCTest
@testable import Lorca

/// What `feedback.list` answers, as the CLI writes it, read into the app's model.
final class FeedbackTests: XCTestCase {
    private let list = """
        {"feedback":[{"id":"feedback-2","kind":"edited","origin":{"chat_id":"chat-1","message_id":"m-2"},"note":"","example":"Lead with decisions.","target":{"kind":"routine_prompt","id":"rt-1"},"created_at":1760000100.0},
                     {"id":"feedback-1","kind":"ignored_alert","origin":{"chat_id":"chat-1","message_id":"m-1","routine_id":"rt-1"},"note":"","example":"","target":null,"created_at":1760000000.0}],
         "feedback_count":2,
         "proposals":[{"id":"proposal-1","target":{"kind":"routine_prompt","id":"rt-1"},"before":{"content":"Summarize.","hash":"h","revision":0},"after":"Summarize. Decisions first.","evidence":["feedback-2"],"origins":[],"explanation":"You moved decisions first.","diff":"--- current\\n+++ proposed\\n@@ -1,1 +1,1 @@\\n-Summarize.\\n+Summarize. Decisions first.\\n","diff_hash":"d","state":"pending","created_at":1760000200.0}],
         "revisions":[{"id":"revision-1","target":{"kind":"playbook","id":"playbook-1","scope":{"kind":"bot","id":"bot-1"}},"created_at":1760000300.0,"rollback_of":null,"diff":"+Check the tests.\\n","can_rollback":true,"current_hash":"c"}],
         "settings":{"review_every_secs":604800},
         "targets":[{"target":{"kind":"routine_prompt","id":"rt-1"},"name":"Morning brief"},{"target":{"kind":"playbook","id":"playbook-1","scope":{"kind":"bot","id":"bot-1"}},"name":"launch-brief"}]}
        """

    @MainActor
    func testTheListReadsIntoTheModel() throws {
        let feedback = try Wire.decoder.decode(Wire.Feedback.self, from: Data(list.utf8)).toModel()
        XCTAssertEqual(feedback.notes.map(\.kind), [.edited, .ignoredAlert])
        XCTAssertEqual(feedback.notes[0].text, "Lead with decisions.", "a note without words shows the message")
        XCTAssertEqual(feedback.suggestions.first?.diffHash, "d")
        XCTAssertEqual(feedback.changes.first?.canUndo, true)
        XCTAssertEqual(feedback.reviewEvery, 604_800)
        XCTAssertEqual(feedback.name(of: feedback.changes[0].target), "launch-brief")
        XCTAssertEqual((feedback.changes[0].target.params["scope"] as? [String: String])?["id"], "bot-1")
        XCTAssertFalse(feedback.isEmpty)
        XCTAssertTrue(BotFeedback.empty.isEmpty)
    }

    func testADiffShowsOnlyItsChangedLines() {
        let diff = "--- current\n+++ proposed\n@@ -1,2 +1,2 @@\n-Old line\n+New line\n+\n"
        XCTAssertEqual(DiffLine.lines(of: diff), [.removed("Old line"), .added("New line"), .added("")])
    }
}
