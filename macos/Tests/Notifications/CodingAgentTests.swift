import XCTest
@testable import Lorca

/// A coding agent's card: what the CLI sends, the words it shows, and, with
/// `LORCA_UI_CAPTURE_DIR`, how it looks in a chat in each state and appearance.
@MainActor
final class CodingAgentTests: XCTestCase {
    private func agent(_ state: AgentRun.State, question: AgentRun.Question? = nil) -> AgentRun {
        AgentRun(
            id: "agent-3f9a2c1d", kind: "claude", host: nil, task: "Fix the broken docs link on the download page, with a test",
            folder: "~/.lorca/worktrees/site-1a2b3c/fix-docs-link", branch: "fix-docs-link", state: state, question: question,
            output: "● Edit(src/pages/download.astro)\n  ⎿ Updated 1 line\n● Bash(bun test links)\n  ⎿ 14 pass\n    0 fail",
            device: "Workbench")
    }

    func testTheCardDecodesFromTheCLI() throws {
        let json = """
            {"id":"m1","chat_id":"c1","author":{"kind":"bot","bot_id":"b1"},"state":{"kind":"complete"},"created_at":1,
             "body":{"kind":"tool","name":"coding_agent","summary":"Started","detail":"","is_running":false,
              "agent":{"id":"agent-1","kind":"codex","host":"herdr","task":"Add dark mode","folder":"~/x/dark","branch":"dark",
               "state":"asking","question":{"kind":"choices","text":"Trust this folder?","choices":["No, exit","Yes, I trust this folder"]},
               "started_at":10}}}
            """
        let message = try Wire.decoder.decode(Wire.Message.self, from: Data(json.utf8)).toModel()
        guard case let .tool(tool) = message.body, let agent = tool.agent else { return XCTFail("no card") }
        XCTAssertTrue(tool.isShown)
        XCTAssertEqual(agent.name, "Codex")
        XCTAssertEqual(agent.hostName, "Herdr")
        XCTAssertEqual(agent.question?.kind, .choices)
        XCTAssertEqual(agent.question?.choices.count, 2)
        XCTAssertEqual(agent.question?.isPermission, false)
        XCTAssertTrue(agent.isRunning)
    }

    func testTheCardSaysHowItStandsInAWordOrTwo() {
        XCTAssertEqual(AgentCellView.title(for: agent(.working), botName: "Developer"), "Claude Code")
        XCTAssertEqual(AgentCellView.detail(for: agent(.working)), "Working · fix-docs-link")
        XCTAssertEqual(AgentCellView.detail(for: agent(.idle)), "Done · fix-docs-link")
        var stalled = agent(.working)
        stalled.stalled = true
        XCTAssertEqual(AgentCellView.detail(for: stalled), "Quiet · fix-docs-link")
        var failed = agent(.failed)
        failed.outcome = "Claude Code exited with code 1"
        XCTAssertEqual(AgentCellView.detail(for: failed), "Failed: Claude Code exited with code 1 · fix-docs-link")
        let start = agent(.asking, question: .init(kind: .start, text: "", command: nil, choices: [], reason: nil, rule: nil))
        XCTAssertEqual(AgentCellView.title(for: start, botName: "Developer"), "Developer wants to start Claude Code on Workbench")
        XCTAssertNil(AgentCellView.detail(for: start), "the question says it")
        XCTAssertFalse(start.isRunning, "nothing to stop before it starts")
        let command = agent(.asking, question: .init(kind: .command, text: "", command: "git push origin fix-docs-link", choices: [], reason: nil, rule: nil))
        XCTAssertEqual(AgentCellView.title(for: command, botName: "Developer"), "Claude Code wants to run a command")
        XCTAssertTrue(command.isRunning)
    }

    func testTheCardKeepsItsPlaceAfterItEnds() {
        var tool = ToolInvocation(name: "coding_agent", summary: "", detail: "", isRunning: false, agent: agent(.stopped))
        XCTAssertTrue(tool.isShown)
        tool.agent = nil
        XCTAssertFalse(tool.isShown)
    }

    /// The developer's chat with its agent's card in each state, as a chat window shows it.
    func testCapturesTheCardInAChat() async throws {
        guard ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"] != nil else { throw XCTSkip("Set LORCA_UI_CAPTURE_DIR to capture") }
        guard AppStore.shared.isMock else { throw XCTSkip("The capture runs against the demo roster: set LORCA_MOCK=1") }
        _ = NSApplication.shared
        let store = AppStore.shared
        store.start()
        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 620, height: 560), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let chat = ChatViewController()
        window.contentViewController = chat
        window.setContentSize(NSSize(width: 620, height: 560))
        window.orderFront(nil)
        chat.show(chatID: "chat-patch")
        let rowID = try XCTUnwrap(store.chat("chat-patch")?.messages.last { if case .tool = $0.body { true } else { false } }?.id)
        let permission = AgentRun.Question(kind: .command, text: "", command: "git push origin fix-docs-link", choices: [], reason: "Pushing sends the branch to GitHub, where others can see it.", rule: "push branches to GitHub")
        let choices = AgentRun.Question(kind: .choices, text: "Bash command\n  rm -rf node_modules && bun install\nDo you want to proceed?", command: nil, choices: ["Yes", "Yes, and don't ask again for rm commands", "No, and tell Claude what to do differently"], reason: nil, rule: nil)
        let start = AgentRun.Question(kind: .start, text: "", command: nil, choices: [], reason: "Claude Code edits files and runs commands on Workbench.", rule: "start coding agents in the site repository")
        var failed = agent(.failed)
        failed.outcome = "Claude Code exited with code 1"
        var inHerdr = agent(.working)
        inHerdr.host = "herdr"
        let states: [(String, AgentRun)] = [
            ("working", agent(.working)),
            ("herdr", inHerdr),
            ("start", agent(.asking, question: start)),
            ("command", agent(.asking, question: permission)),
            ("choices", agent(.asking, question: choices)),
            ("done", agent(.idle)),
            ("failed", failed),
        ]
        for (name, card) in states {
            store.update(rowID, in: "chat-patch") { message in
                guard case var .tool(tool) = message.body else { return }
                tool.agent = card
                message.body = .tool(tool)
            }
            try await Task.sleep(nanoseconds: 300_000_000)
            try capture(window, name: "agent-card-\(name)")
        }
    }

    /// The transcript sheet, for an agent in Herdr on this Runner.
    func testCapturesTheTranscript() async throws {
        guard ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"] != nil else { throw XCTSkip("Set LORCA_UI_CAPTURE_DIR to capture") }
        guard AppStore.shared.isMock else { throw XCTSkip("The capture runs against the demo roster: set LORCA_MOCK=1") }
        _ = NSApplication.shared
        let store = AppStore.shared
        store.start()
        let rowID = try XCTUnwrap(store.chat("chat-patch")?.messages.last { if case .tool = $0.body { true } else { false } }?.id)
        var card = agent(.working)
        card.host = "herdr"
        store.update(rowID, in: "chat-patch") { message in
            guard case var .tool(tool) = message.body else { return }
            tool.agent = card
            message.body = .tool(tool)
        }
        let parent = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 900, height: 760), styleMask: [.titled], backing: .buffered, defer: false)
        parent.isReleasedWhenClosed = false
        let root = NSViewController()
        root.view = NSView(frame: NSRect(x: 0, y: 0, width: 900, height: 760))
        parent.contentViewController = root
        parent.orderFront(nil)
        let sheet = AgentTranscriptViewController(chatID: "chat-patch", messageID: rowID, agent: card)
        root.presentAsSheet(sheet)
        try await Task.sleep(nanoseconds: 600_000_000)
        let window = try XCTUnwrap(sheet.view.window)
        try capture(window, name: "agent-transcript")
    }

    private func capture(_ window: NSWindow, name: String) throws {
        guard let folder = ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"], let view = window.contentView else { return }
        for (suffix, appearance) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
            window.appearance = NSAppearance(named: appearance)
            view.layoutSubtreeIfNeeded()
            view.displayIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            try FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
            try png.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name)-\(suffix).png"))
        }
    }
}
