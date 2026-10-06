import XCTest
@testable import Lorca

final class RunningTaskTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 1_000)

    private func task(_ state: CommandRun.State, inGroup: Bool = false) -> RunningTask {
        RunningTask(
            id: "call", title: "Install dependencies", botName: "Scout", showsBotName: inGroup, command: "bun install",
            firstLine: "bun install", output: "", state: state, runsInForeground: false, startedAt: start)
    }

    func testTheRunningTimeCountsUpFromTheStart() {
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(5)), "0:05")
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(12 * 60 + 3)), "12:03")
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(3600 + 2 * 60 + 3)), "1:02:03")
        // Another Device's clock may run behind the Runner's.
        XCTAssertEqual(RunningTask.elapsed(since: start, now: start.addingTimeInterval(-4)), "0:00")
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

    func testRunInBackgroundIsOfferedWhileTheBotsCallWaitsOnACommandInTheForeground() {
        func row(isRunning: Bool, state: CommandRun.State = .running, sessionID: String? = "bash-1", background: Bool = false) -> Message {
            let run = CommandRun(sessionID: sessionID, command: "npm run dev", state: state, background: background)
            let tool = ToolInvocation(name: "bash", summary: "Running", detail: "", isRunning: isRunning, run: run)
            return Message(author: .bot("bot"), body: .tool(tool), createdAt: start)
        }
        XCTAssertTrue(row(isRunning: true).runsInForeground)
        // Sent there, or started there and still in its first two seconds.
        XCTAssertFalse(row(isRunning: false, background: true).runsInForeground)
        XCTAssertFalse(row(isRunning: true, background: true).runsInForeground)
        // Its call returned, or no terminal runs it yet.
        XCTAssertFalse(row(isRunning: false).runsInForeground)
        XCTAssertFalse(row(isRunning: true, state: .checking, sessionID: nil).runsInForeground)
    }
}
