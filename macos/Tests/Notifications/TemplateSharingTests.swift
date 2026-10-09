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

    func testShareSendsThePickedContentInTheBotsOrder() async throws {
        try prepare()
        let bot = try XCTUnwrap(AppStore.shared.bot("bot-nova"))
        var preview: [String: Any]?
        let controller = TemplateShareViewController(bot: bot) { [unowned self] method, params in
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
        // The profile is always in the file, so it has no check; nothing else starts picked.
        XCTAssertEqual(rows[0].accessibilityRole(), .staticText)
        XCTAssertEqual(rows.map(\.isSelected), Array(repeating: false, count: 9))
        XCTAssertTrue(controller.confirmButton.isEnabled)
        XCTAssertEqual(controller.confirmButton.title, "Share Link")
        XCTAssertTrue(descendants(controller.view).contains { ($0 as? NSButton)?.title == "Save as File…" })
        XCTAssertFalse(descendants(controller.view).contains { ($0 as? NSButton)?.title == "Select All" }, "a short list has no Select All")
        // Taller than its room, the list ends between rows, never through one.
        let list = try XCTUnwrap(descendants(controller.view).compactMap { $0 as? TemplateItemList }.first)
        controller.view.layoutSubtreeIfNeeded()
        XCTAssertLessThanOrEqual(list.frame.height, 380)
        XCTAssertGreaterThan(list.frame.height, 300, "the list shows what fits")
        for row in rows {
            let frame = row.convert(row.bounds, to: list)
            XCTAssertFalse(frame.minY < -0.5 && frame.maxY > 0.5, "the list's edge cuts through \(row.toolTip ?? "")")
        }
        try capture(controller, window: window, name: "share")

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
        try capture(controller, window: window, name: "share-error")
    }

    func testShareLinkShowsTheLinkCopied() async throws {
        try prepare()
        let bot = try XCTUnwrap(AppStore.shared.bot("bot-nova"))
        let url = "https://lorca.app/t/yWUtxVvEA7X0edjxRUKd2A#-xMo1zlzI63iQ_tY2ATkxlkle7thq-OfylCF-jFDqYw"
        var shared: [String: Any]?
        let controller = TemplateShareViewController(bot: bot) { [unowned self] method, params in
            switch method {
            case "templates.contents": return self.contents
            case "templates.export.preview": return ["digest": "digest-1"]
            case "templates.share":
                shared = params
                return ["link": ["id": "yWUtxVvEA7X0edjxRUKd2A", "url": url]]
            default: XCTFail("unexpected \(method)"); return [:]
            }
        }
        let window = host(controller)
        defer {
            controller.dismiss(nil)
            window.close()
        }
        try await wait { self.rows(in: controller.view).count == 9 }
        _ = rows(in: controller.view)[1].accessibilityPerformPress()
        controller.confirmButton.performClick(nil)
        try await wait { controller.confirmButton.title == "Done" }
        XCTAssertEqual(shared?["expected_digest"] as? String, "digest-1")
        XCTAssertEqual(shared?["reviewed"] as? Bool, true)
        XCTAssertNil(shared?["link_id"], "a bot shared the first time gets a new link")
        XCTAssertEqual((shared?["selection"] as? [String: Any])?["routine_ids"] as? [String], ["rt-brief"])
        XCTAssertEqual(LinkBox.pasteboard.string(forType: .string), url, "the link is copied already")
        XCTAssertTrue(labels(in: controller.view).contains(url))
        XCTAssertFalse(descendants(controller.view).contains { ($0 as? NSButton)?.title == "Cancel" })
        try capture(controller, window: window, name: "shared")
    }

    func testUpdateKeepsTheLinkAndStartsFromWhatItHolds() async throws {
        try prepare()
        let bot = try XCTUnwrap(AppStore.shared.bot("bot-quill"))
        let link = try XCTUnwrap(AppStore.shared.sharedLink(for: bot.id))
        var shared: [String: Any]?
        let contents: [String: Any] = [
            "profile": ["id": "profile", "content": ["name": "Writer", "symbol_name": "pencil.and.scribble", "accent": "pink",
                "description": "Writes docs, copy, and release notes in plain language: short sentences, no filler, and no exclamation marks."]],
            "memories": [["id": "memory-voice", "content": "- Plain words, short sentences, no exclamation marks."],
                         ["id": "memory-launch", "content": "- The launch post goes out Friday morning."]],
            "routines": [], "requirements": [["service_id": "notion", "name": "Notion"]]]
        let controller = TemplateShareViewController(bot: bot) { method, params in
            switch method {
            case "templates.contents": return contents
            case "templates.export.preview": return ["digest": "digest-2"]
            default:
                shared = params
                return ["link": ["id": link.id, "url": link.url]]
            }
        }
        let window = host(controller)
        defer {
            controller.dismiss(nil)
            window.close()
        }
        try await wait { self.rows(in: controller.view).count == 4 }
        XCTAssertEqual(controller.confirmButton.title, "Update Link")
        XCTAssertTrue(labels(in: controller.view).contains(link.url))
        XCTAssertEqual(rows(in: controller.view).map(\.isSelected), [false, false, true, false], "the memory the link holds starts picked")
        try capture(controller, window: window, name: "update")
        controller.confirmButton.performClick(nil)
        try await wait { controller.confirmButton.title == "Done" }
        XCTAssertEqual(shared?["link_id"] as? String, link.id)
    }

    func testSharedLinksListTheAccountsLinks() throws {
        try prepare()
        let pane = SharedLinksSettingsViewController()
        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 640, height: 420), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = pane
        window.setContentSize(NSSize(width: 640, height: 420))
        defer { window.close() }
        pane.view.layoutSubtreeIfNeeded()
        let row = try XCTUnwrap(descendants(pane.view).compactMap { $0 as? BotRow }.first)
        XCTAssertTrue(labels(in: pane.view).contains("Writer"))
        XCTAssertEqual(row.menu?.items.map(\.title), ["Copy Link", "", "Revoke Link…"])
        try captureView(pane.view, window: window, name: "shared-links")
    }

    func testLongListsOfferSelectAll() async throws {
        try prepare()
        let bot = try XCTUnwrap(AppStore.shared.bot("bot-nova"))
        var contents = contents
        var memories = contents["memories"] as! [[String: Any]]
        memories += [["id": "memory-notes", "content": "- Release notes go out on Monday."], ["id": "memory-design", "content": "- The design review is on Tuesdays."]]
        contents["memories"] = memories
        var preview: [String: Any]?
        let controller = TemplateShareViewController(bot: bot) { method, params in
            if method == "templates.contents" { return contents }
            preview = params["selection"] as? [String: Any]
            throw CLIClient.RequestError(message: "stop")
        }
        let window = host(controller)
        defer {
            controller.dismiss(nil)
            window.close()
        }
        try await wait { self.rows(in: controller.view).count == 11 }
        let buttons = descendants(controller.view).compactMap { $0 as? NSButton }.filter { $0.title == "Select All" }
        XCTAssertEqual(buttons.count, 1, "only the list of six memories is long")
        buttons[0].performClick(nil)
        XCTAssertEqual(buttons[0].title, "Deselect All")
        controller.confirmButton.performClick(nil)
        try await wait { preview != nil }
        XCTAssertEqual((preview?["memory_ids"] as? [String])?.count, 6)
    }

    func testImportUsesTheRunnersConnectionAndSaysWhatIsMissing() async throws {
        try prepare()
        var linearInstalled = false
        var requests: [[String: Any]] = []
        let url = URL(fileURLWithPath: "/tmp/Project Manager.lorca-template")
        let controller = TemplateImportViewController(source: .file(url), reply: { [unowned self] method, params in
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
        XCTAssertEqual(requests.first?["path"] as? String, url.path)
        XCTAssertFalse(controller.confirmButton.isEnabled)
        XCTAssertEqual(fields(in: controller.view).map(\.stringValue), [(url.path as NSString).abbreviatingWithTildeInPath, "Project Manager"])
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

    func testImportOpensAPastedLink() async throws {
        try prepare()
        var requests: [[String: Any]] = []
        let controller = TemplateImportViewController(reply: { [unowned self] method, params in
            requests.append(params)
            return self.importPreview(selected: "github", linearInstalled: true)
        }, onCreate: { _ in XCTFail("a test never adds a bot") })
        let window = host(controller)
        defer {
            controller.dismiss(nil)
            window.close()
        }
        // Nothing to set up until a template opens.
        XCTAssertFalse(labels(in: controller.view).contains("Name"))
        XCTAssertTrue(requests.isEmpty)
        try capture(controller, window: window, name: "import-empty")
        let from = try XCTUnwrap(fields(in: controller.view).first)
        let link = "https://lorca.app/t/yWUtxVvEA7X0edjxRUKd2A#-xMo1zlzI63iQ_tY2ATkxlkle7thq-OfylCF-jFDqYw"
        from.stringValue = link
        (controller as NSTextFieldDelegate).controlTextDidChange?(Notification(name: NSControl.textDidChangeNotification, object: from))
        try await wait { controller.confirmButton.isEnabled }
        XCTAssertEqual(requests.first?["link"] as? String, link)
        XCTAssertNil(requests.first?["path"])
        XCTAssertTrue(labels(in: controller.view).contains("Name"))
        try capture(controller, window: window, name: "import-link")
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
        // Copy Link goes to a pasteboard of the test's own, never the user's.
        LinkBox.pasteboard = NSPasteboard(name: NSPasteboard.Name("app.lorca.tests"))
    }

    /// A pane as it shows, in each appearance, when `LORCA_UI_CAPTURE_DIR` names a folder.
    private func captureView(_ view: NSView, window: NSWindow, name: String) throws {
        guard let folder = ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"] else { return }
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
        descendants(view).compactMap { $0 as? NSTextField }.filter { !$0.isHiddenOrHasHiddenAncestor }.map(\.stringValue)
    }
}
