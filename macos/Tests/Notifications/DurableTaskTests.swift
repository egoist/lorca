import AppKit
import XCTest
@testable import Lorca

final class DurableTaskTests: XCTestCase {
    func testWireDecodesOwnershipStateAndEvidenceReferences() throws {
        let data = Data(#"""
        {"id":"task-00000000-0000-0000-0000-000000000001","revision":7,
         "authority_runner_id":"authority","owner_bot_id":"bot","runner_id":"runner",
         "goal":"Deliver report","acceptance_criteria":["Sources verified"],"dependencies":["task-dependency"],
         "next_action":"Review evidence","chat_ids":["group"],"links":[],"state":"awaiting_review",
         "reason":null,"result":"Report ready","active_run":null,"created_at":1,"updated_at":2,
         "evidence":[{"kind":"output","label":"Report v2","chat_id":"group","message_id":"message-v2","output_id":"out-report","version":2}]}
        """#.utf8)
        let task = try Wire.decoder.decode(DurableTask.self, from: data)
        XCTAssertEqual(task.authorityRunnerId, "authority")
        XCTAssertEqual(task.ownerBotId, "bot")
        XCTAssertEqual(task.runnerId, "runner")
        XCTAssertEqual(task.state, .awaitingReview)
        XCTAssertEqual(task.evidence[0].outputId, "out-report")
        XCTAssertEqual(task.evidence[0].version, 2)
        XCTAssertFalse(task.canStart)
    }

    func testOnlyQueuedOrBlockedTasksWithoutARunStart() throws {
        let data = Data(#"""
        {"id":"task-1","revision":1,"authority_runner_id":"r","owner_bot_id":"b","runner_id":"r","goal":"g",
         "acceptance_criteria":["c"],"dependencies":[],"next_action":"n","chat_ids":["c"],"links":[],"state":"blocked",
         "reason":"Waiting on a key","result":null,"evidence":[],"active_run":null,"created_at":1,"updated_at":1}
        """#.utf8)
        var task = try Wire.decoder.decode(DurableTask.self, from: data)
        XCTAssertTrue(task.canStart)
        task.activeRun = .init(id: "task-run-1", botId: "b", runnerId: "r", chatId: "c", startedAt: 1)
        XCTAssertFalse(task.canStart)
        task.activeRun = nil
        task.state = .completed
        XCTAssertFalse(task.canStart)
        XCTAssertTrue(task.state.isFinished)
    }
}
