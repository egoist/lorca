import XCTest
@testable import Lorca

final class RunningTaskTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_000)

    private func task(_ state: CommandRun.State, inGroup: Bool = false) -> RunningTask {
        RunningTask(
            id: "call", title: "Install dependencies", botName: "Scout", showsBotName: inGroup, command: "bun install",
            firstLine: "bun install", output: "", state: state, startedAt: start)
    }

    func testTheRunningTimeCountsUpFromTheStart() {
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(5)), "0:05")
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(12 * 60 + 3)), "12:03")
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(3600 + 2 * 60 + 3)), "1:02:03")
        // Another Device's clock may run behind the Runner's.
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(-4)), "0:00")
    }

    func testTheRunningTimeCountsFromWhenItsTerminalStartedIt() throws {
        // The row goes up before Auto-review asks; its terminal starts it after the answer.
        let row = { (run: String) in
            #"{"id": "call", "chat_id": "chat", "author": {"kind": "bot", "bot_id": "scout"}, "created_at": 820, "state": {"kind": "complete"}, "body": {"kind": "tool", "name": "bash", "summary": "Running", "detail": "", "is_running": true, "run": \#(run)}}"#
        }
        let started = try Wire.decoder.decode(
            Wire.Message.self,
            from: Data(row(#"{"session_id": "bash-1", "started_at": 1000.5, "command": "bun install", "state": "running"}"#).utf8)
        ).toModel()
        XCTAssertEqual(started.commandRun?.startedAt, Date(timeIntervalSince1970: 1000.5))
        let asking = try Wire.decoder.decode(Wire.Message.self, from: Data(row(#"{"command": "bun install", "state": "asking"}"#).utf8)).toModel()
        XCTAssertNil(asking.commandRun?.startedAt)
    }

    func testTheStatusSaysWhoRunsItAndHowItStands() {
        let now = start.addingTimeInterval(65)
        XCTAssertEqual(task(.running).status(at: now), "Running · 1:05")
        XCTAssertEqual(task(.waiting, inGroup: true).status(at: now), "Scout · Waiting for input · 1:05")
        // One that ended while the list was open says how, with no running time.
        XCTAssertEqual(task(.exited).status(at: now), "Finished")
        XCTAssertEqual(task(.failed).status(at: now), "Failed")
        XCTAssertEqual(task(.stopped, inGroup: true).status(at: now), "Scout · Stopped")
    }

    func testOnlyACommandInItsTerminalIsARunningTask() {
        var run = CommandRun(command: "bun install", state: .running)
        // Before a terminal runs it: Auto-review has yet to allow it.
        XCTAssertFalse(run.takesInput)
        run.sessionID = "bash-1"
        XCTAssertTrue(run.takesInput)
        run.state = .waiting
        XCTAssertTrue(run.takesInput)
        run.state = .exited
        XCTAssertFalse(run.takesInput)
        XCTAssertTrue(run.hasEnded)
        run.state = .denied
        XCTAssertFalse(run.hasEnded)
    }
}
