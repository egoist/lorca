import AppKit
import XCTest
@testable import Lorca

/// The template sheets against answers in the CLI's shape, with the demo roster: what they send
/// back, and what they let the user do. `LORCA_UI_CAPTURE_DIR` also saves each state as a 2x PNG
/// in the light and dark appearances.
@MainActor
final class TemplateSharingTests: XCTestCase {
    private let profile: [String: Any] = [
        "name": "Project Manager", "symbol_name": "list.bullet.clipboard.fill", "accent": "indigo",
        "description": "Plans the work and delegates it to the team. Breaks work down, hands it off with message_bot, and summarizes what came back.",
    ]
    private let brief: [String: Any] = [
        "name": "Morning brief", "schedule": "0 9 * * 1-5",
        "prompt": "Read the recent messages in every chat you are in and post a short brief: what changed and what needs a decision.",
    ]
    private let checklist: [String: Any] = [
        "name": "Launch checklist", "schedule": "every 2h",
        "prompt": "Review the launch checklist and report new blockers, or PASS when nothing changed.",
    ]

    private var contents: [String: Any] {
        ["profile": ["id": "profile", "content": profile],
         "memories": [
            ["id": "memory-launch", "content": "- The launch is on Friday; the go/no-go call is Thursday at 4 PM."],
            ["id": "memory-summary", "content": "- Send the weekly summary to team@example.com.", "flags": ["email"]],
            ["id": "memory-staging", "content": "- Staging deploys use API_KEY=«redacted credential».", "flags": ["credential"]],
            ["id": "topic-release", "content": "# Release process\nCut the release branch on Thursday. Tag it after the checklist passes and the notes are approved."],
         ],
         "routines": [["id": "rt-brief", "content": brief], ["id": "rt-checklist", "content": checklist]],
         "requirements": [["service_id": "github", "name": "GitHub"], ["service_id": "linear", "name": "Linear"]]]
    }

    private func importPreview(selected: String?, linearInstalled: Bool) -> [String: Any] {
        var requirements: [[String: Any]] = [["service_id": "github", "name": "GitHub", "selected": selected as Any,
            "candidates": [["id": "github", "name": "GitHub", "state": "ready", "detail": "Ready"]]]]
        if !linearInstalled { requirements.append(["service_id": "linear", "name": "Linear", "candidates": []]) }
        let template: [String: Any] = ["format": "lorca.bot-template", "version": 1, "profile": profile,
            "memories": ["- The launch is on Friday; the go/no-go call is Thursday at 4 PM."],
            "routines": [brief.merging(["schedule_text": "Weekdays at 9:00 AM"]) { $1 }],
            "requirements": requirements.map { ["service_id": $0["service_id"]!] }]
        return ["digest": "digest-1", "can_import": selected != nil && linearInstalled, "issues": [], "template": template, "requirements": requirements]
    }

    func testExportSendsThePickedContentInTheBotsOrder() async throws {
        try prepare()
        let bot = try XCTUnwrap(AppStore.shared.bot("bot-nova"))
        var preview: [String: Any]?
        let controller = TemplateExportViewController(bot: bot) { [unowned self] method, params in
            switch method {
            case "templates.contents": return self.contents
            case "templates.export.preview":
                preview = params["selection"] as? [String: Any]
                throw CLIClient.RequestError(message: "What you picked uses GitHub. Check it under Plugins too.")
            default: XCTFail("unexpected \(method)"); return [:]
            }
        }
        let window = host(controller)
        defer {
            controller.dismiss(nil)
            window.close()
        }
        try await wait { self.rows(in: controller.view).count == 9 }
        let rows = rows(in: controller.view)
        // The profile starts picked; nothing else does.
        XCTAssertEqual(rows.map(\.isSelected), [true] + Array(repeating: false, count: 8))
        XCTAssertTrue(controller.confirmButton.isEnabled)
        try capture(controller, window: window, name: "export")

        // Every memory at once, and none.
        let all = try XCTUnwrap(descendants(controller.view).compactMap { $0 as? NSButton }.first { $0.title == "Select All" })
        all.performClick(nil)
        XCTAssertEqual(rows[5...].map(\.isSelected), [true, true, true, true])
        XCTAssertEqual(all.title, "Deselect All")
        all.performClick(nil)
        XCTAssertEqual(rows[5...].map(\.isSelected), [false, false, false, false])

        // Picked out of order, listed in the bot's: routines, plugins, then memories.
        _ = rows[8].accessibilityPerformPress()
        _ = rows[5].accessibilityPerformPress()
        _ = rows[1].accessibilityPerformPress()
        controller.confirmButton.performClick(nil)
        try await wait { preview != nil }
        XCTAssertEqual(preview?["profile"] as? Bool, true)
        XCTAssertEqual(preview?["memory_ids"] as? [String], ["memory-launch", "topic-release"])
        XCTAssertEqual(preview?["routine_ids"] as? [String], ["rt-brief"])
        XCTAssertEqual(preview?["requirement_ids"] as? [String], [])
        try await wait { self.labels(in: controller.view).contains("What you picked uses GitHub. Check it under Plugins too.") }
        XCTAssertTrue(controller.confirmButton.isEnabled, "the user fixes the selection and tries again")
        try capture(controller, window: window, name: "export-error")
    }

    func testImportUsesTheRunnersConnectionAndSaysWhatIsMissing() async throws {
        try prepare()
        var linearInstalled = false
        var requests: [[String: Any]] = []
        let url = URL(fileURLWithPath: "/tmp/Project Manager.lorca-template")
        let controller = TemplateImportViewController(url: url, reply: { [unowned self] method, params in
            XCTAssertEqual(method, "templates.import.preview")
            requests.append(params)
            return self.importPreview(selected: "github", linearInstalled: linearInstalled)
        }, onCreate: { _ in XCTFail("a test never adds a bot") })
        let window = host(controller)
        defer {
            controller.dismiss(nil)
            window.close()
        }
        try await wait { self.labels(in: controller.view).contains("Add Linear to Workbench from the Marketplace first.") }
        XCTAssertEqual(requests.first?["runner_id"] as? String, "dev-workbench")
        XCTAssertFalse(controller.confirmButton.isEnabled)
        XCTAssertEqual(fields(in: controller.view).first?.stringValue, "Project Manager")
        try capture(controller, window: window, name: "import-missing")

        linearInstalled = true
        controller.loadViewIfNeeded()
        let runner = try XCTUnwrap(popups(in: controller.view).first)
        _ = runner.sendAction(runner.action, to: runner.target)
        try await wait { controller.confirmButton.isEnabled }
        XCTAssertTrue(labels(in: controller.view).contains("Routines start paused."))
        controller.view.layoutSubtreeIfNeeded()
        let stack = controller.contentStack
        XCTAssertEqual(stack.frame.height, stack.fittingSize.height, accuracy: 1, "the sheet shrinks with its rows")
        XCTAssertEqual((requests.last?["mappings"] as? [String: String]) ?? [:], [:], "a new Runner starts over")
        try capture(controller, window: window, name: "import")
    }

    func testContentsReadTheCLIsShape() {
        let parsed = TemplateContents(json: contents) { $0 == "rt-brief" ? "Weekdays at 9:00 AM" : nil }
        XCTAssertEqual(parsed.profile.title, "Project Manager")
        XCTAssertEqual(parsed.memories.map(\.title).first, "The launch is on Friday; the go/no-go call is Thursday at 4 PM.")
        XCTAssertEqual(parsed.memories[3].title, "Release process")
        XCTAssertEqual(parsed.memories[1].flagSummary?.personal, true)
        XCTAssertEqual(parsed.memories[2].flagSummary?.personal, false)
        XCTAssertTrue(parsed.routines[0].detail.hasPrefix("Weekdays at 9:00 AM · "))
        XCTAssertEqual(parsed.plugins.map(\.name), ["GitHub", "Linear"])
        let preview = TemplateImportPreview(json: importPreview(selected: nil, linearInstalled: true))
        XCTAssertFalse(preview.plugins[0].isReady)
        XCTAssertTrue(TemplateImportPreview(json: importPreview(selected: "github", linearInstalled: true)).plugins[0].isReady)
    }

    private func prepare() throws {
        guard AppStore.shared.isMock else { throw XCTSkip("The sheets run against the demo roster: set LORCA_MOCK=1") }
        _ = NSApplication.shared
        AppStore.shared.start()
    }

    /// Presents the sheet as the app does, on a window out of sight.
    private func host(_ controller: SheetViewController) -> NSWindow {
        let parent = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 900, height: 900), styleMask: [.titled], backing: .buffered, defer: false)
        parent.isReleasedWhenClosed = false
        let root = NSViewController()
        root.view = NSView(frame: NSRect(x: 0, y: 0, width: 900, height: 900))
        parent.contentViewController = root
        parent.orderFront(nil)
        root.presentAsSheet(controller)
        return parent
    }

    /// The sheet as it shows, in each appearance, when `LORCA_UI_CAPTURE_DIR` names a folder.
    private func capture(_ controller: SheetViewController, window parent: NSWindow, name: String) throws {
        guard let folder = ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"] else { return }
        let sheet = try XCTUnwrap(controller.view.window)
        for (suffix, appearance) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
            parent.appearance = NSAppearance(named: appearance)
            sheet.appearance = parent.appearance
            let view = try XCTUnwrap(sheet.contentView?.superview)
            view.layoutSubtreeIfNeeded()
            sheet.displayIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            try FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
            try png.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name)-\(suffix).png"))
        }
    }

    private func wait(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTFail("the sheet never got there")
    }

    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
    private func rows(in view: NSView) -> [TemplateItemRow] { descendants(view).compactMap { $0 as? TemplateItemRow } }
    private func popups(in view: NSView) -> [NSPopUpButton] { descendants(view).compactMap { $0 as? NSPopUpButton } }
    private func fields(in view: NSView) -> [NSTextField] { descendants(view).compactMap { $0 as? NSTextField }.filter(\.isEditable) }
    private func labels(in view: NSView) -> [String] {
        descendants(view).compactMap { $0 as? NSTextField }.filter { !$0.isHidden }.map(\.stringValue)
    }
}
