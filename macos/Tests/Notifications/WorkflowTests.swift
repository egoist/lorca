import XCTest
@testable import Lorca

final class WorkflowTests: XCTestCase {
    func testOlderMarketplaceRepliesDecodeWithNoPacks() throws {
        let wire = try Wire.decoder.decode(Wire.Marketplace.self, from: Data(#"{"plugins":[],"bots":[]}"#.utf8))
        XCTAssertNil(wire.packs)
    }

    func testProgressDecodesAndPicksTheAccountInUse() throws {
        let source = #"""
        {
          "setup": {
            "id":"workflow-1","runner_id":"runner-1","updated_at":1,
            "pack":{"id":"inbox-triage","name":"Inbox triage","outcome":"Triage the inbox","description":"Review a sample","symbol_name":"envelope","questions":[{"id":"inbox-scope","label":"Messages","placeholder":"Unread"}],"connections":[{"service_id":"gmail","name":"Gmail"},{"service_id":"github","name":"GitHub"},{"service_id":"slack","name":"Slack"}]},
            "answers":{"inbox-scope":"Unread today"},"bot_ids":{"triager":"bot-1"},"connection_ids":{"gmail":"gmail-personal"},"phase":"reviewed",
            "sample":{"job_id":"job-1","chat_id":"chat-1","bot_id":"bot-1","started_at":1,"state":"reviewed","message_ids":["reply-1"]}
          },
          "connections":[
            {"service_id":"gmail","name":"Gmail","selected_id":"gmail-personal","available":true,"choices":[
              {"id":"gmail-work","name":"Gmail","service_id":"gmail","account_name":"Work","state":"ready","detail":"Ready"},
              {"id":"gmail-personal","name":"Gmail","service_id":"gmail","account_name":"Personal","state":"needs_auth","detail":"Sign in"}]},
            {"service_id":"github","name":"GitHub","selected_id":null,"available":true,"choices":[{"id":"github","name":"GitHub","state":"ready","detail":"Ready"}]},
            {"service_id":"slack","name":"Slack","selected_id":null,"available":false,"choices":[
              {"id":"slack-a","name":"Slack","service_id":"slack","account_name":"A","state":"ready","detail":"Ready"},
              {"id":"slack-b","name":"Slack","service_id":"slack","account_name":"B","state":"ready","detail":"Ready"}]}],
          "specialists":[{"id":"triager","name":"Inbox Triager","selected_id":"bot-1","choices":[{"id":"bot-1","name":"Inbox Triager"}]}],
          "routines":[{"id":"routine-1","name":"Inbox check","schedule_text":"Weekdays at 9:00 AM","is_enabled":false}],
          "sample_messages":[{"id":"reply-1","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},"body":{"kind":"text","text":"One urgent message."},"state":{"kind":"complete"},"created_at":1}],
          "is_running":false
        }
        """#
        let progress = try Wire.decoder.decode(WorkflowProgress.self, from: Data(source.utf8))
        XCTAssertEqual(progress.setup.pack.symbolName, "envelope")
        XCTAssertEqual(progress.setup.answers["inbox-scope"], "Unread today")
        XCTAssertEqual(progress.setup.sample?.jobId, "job-1")
        XCTAssertEqual(progress.sampleMessages.first?.body.text, "One urgent message.")
        XCTAssertFalse(progress.routines[0].isEnabled)
        // The chosen account, with its own state; the Runner's only one; none of several unchosen.
        XCTAssertEqual(progress.connections[0].account?.accountName, "Personal")
        XCTAssertEqual(progress.connections[0].account?.plugin.state, .needsAuth)
        XCTAssertEqual(progress.connections[1].account?.plugin.id, "github")
        XCTAssertNil(progress.connections[2].account)
    }
}
