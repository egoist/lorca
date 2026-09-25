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
