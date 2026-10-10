import XCTest
@testable import Lorca

/// A one-time routine, a watch on one pull request, and a routine around calendar events, as the
/// CLI sends them: the schedule in words, the row's symbol, and the routine sheet's rows.
@MainActor
final class RoutineTriggerTests: XCTestCase {
    private func routine(_ json: String) throws -> Routine {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let base = #""id":"rt-1","bot_id":"b1","name":"Brief","prompt":"Read the inbox","created_at":0,"is_enabled":true,"state":"on""#
        return try decoder.decode(Wire.Routine.self, from: Data("{\(base),\(json)}".utf8)).toModel()
    }

    func testEachKindSaysWhatPlacesItsRuns() throws {
        let watch = try routine(#""schedule":"every 10m","schedule_text":"Watches acme/project#42","next_run_at":2000000000,"pull_request":{"repo":"acme/project","number":42,"title":"Add login","url":"https://github.com/acme/project/pull/42"}"#)
        XCTAssertEqual(watch.scheduleText, L("Watches %@", "acme/project#42"))
        XCTAssertEqual(watch.pullRequest?.url?.absoluteString, "https://github.com/acme/project/pull/42")
        XCTAssertTrue(watch.looksFirst)
        XCTAssertEqual(watch.symbol, "arrow.triangle.pull")
        XCTAssertEqual(watch.detail, L("%@ · Next check %@", watch.scheduleText, Format.upcoming(Date(timeIntervalSince1970: 2_000_000_000))))
        var failing = watch
        failing.state = "failed"
        failing.health.status = "failed"
        XCTAssertEqual(failing.problem, .readFailed(calendar: false))

        let events = try routine(#""schedule":"15m before events","schedule_text":"15 minutes before events matching “Customer”","timezone":"Pacific/Kiritimati","calendar":{"account":"Google Calendar · Work","matching":"Customer","minutes":15,"after":false,"next_event":{"title":"Customer call: Acme","start":1,"end":2}}"#)
        XCTAssertEqual(events.scheduleText, L("%@ before events matching “%@”", L("%d minutes", 15), "Customer"))
        XCTAssertEqual(events.scheduleSummary, events.scheduleText, "events keep their own times, whatever the zone")
        XCTAssertEqual(events.calendar?.nextEventTitle, "Customer call: Acme")
        XCTAssertEqual(events.symbol, "calendar")
        XCTAssertEqual(Format.aroundEvents(minutes: 0, after: true, matching: nil), L("When each event ends"))
        XCTAssertEqual(Format.aroundEvents(minutes: 120, after: false, matching: nil), L("%@ before each event", L("%d hours", 2)))

        // 2030-10-12 09:00 in Singapore.
        let once = try routine(#""schedule":"once 2030-10-12 09:00","schedule_text":"Once on 2030-10-12 at 9:00 AM","timezone":"Asia/Singapore","once_at":1917997200"#)
        // The clock reads 9:00 on the routine's own clock, in the system's spacing.
        XCTAssertTrue(once.scheduleText.hasPrefix("Once on Oct 12, 2030 at 9:00"), once.scheduleText)
        XCTAssertEqual(once.symbol, "alarm")
        XCTAssertFalse(once.looksFirst)
    }

    func testSheetsShowEachKind() async throws {
        guard AppStore.shared.isMock else { throw XCTSkip("The sheets run against the demo roster: set LORCA_MOCK=1") }
        _ = NSApplication.shared
        AppStore.shared.start()
        for (id, botID) in [("rt-login-pr", "bot-patch"), ("rt-tag-release", "bot-patch"), ("rt-call-prep", "bot-scout")] {
            let bot = try XCTUnwrap(AppStore.shared.bot(botID))
            let controller = RoutineViewController(routineID: id, bot: bot)
            let window = host(controller)
            let labels = descendants(controller.view).compactMap { $0 as? NSTextField }.filter { !$0.isHiddenOrHasHiddenAncestor }.map(\.stringValue)
            switch id {
            case "rt-login-pr":
                XCTAssertTrue(labels.contains(L("Pull request")) && labels.contains("Add passkey sign-in") && labels.contains(L("Next check")), "\(labels)")
            case "rt-tag-release":
                XCTAssertFalse(labels.contains(L("Missed runs")), "a one-time routine runs once its Runner is back")
            default:
                XCTAssertTrue(labels.contains("Google Calendar · Work") && labels.contains { $0.hasSuffix("· Customer call: Acme") }, "\(labels)")
            }
            try capture(controller, window: window, name: "routine-\(id)")
            controller.dismiss(nil)
            window.close()
        }
        // The rows of the Developer's routines, as its inspector lists them.
        let section = SectionView(title: L("Routines"))
        section.setRows(AppStore.shared.routines(for: "bot-patch").map { routine in
            let row = SwitchRow()
            row.configure(routine: routine)
            return row
        })
        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 300, height: 200), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        let root = NSView(frame: NSRect(x: 0, y: 0, width: 300, height: 200))
        window.contentView = root
        root.addSubview(section)
        NSLayoutConstraint.activate([
            section.leadingAnchor.constraint(equalTo: root.leadingAnchor, constant: 12),
            section.trailingAnchor.constraint(equalTo: root.trailingAnchor, constant: -12),
            section.topAnchor.constraint(equalTo: root.topAnchor, constant: 12),
        ])
        try captureView(root, window: window, name: "inspector-routines")
        window.close()
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

    /// A view as it shows, in each appearance, when `LORCA_UI_CAPTURE_DIR` names a folder.
    private func captureView(_ view: NSView, window: NSWindow, name: String) throws {
        guard let folder = ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"] else { return }
        for (suffix, appearance) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
            window.appearance = NSAppearance(named: appearance)
            view.wantsLayer = true
            NSAppearance(named: appearance)!.performAsCurrentDrawingAppearance {
                view.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
            }
            view.layoutSubtreeIfNeeded()
            view.displayIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
            try FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
            try png.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name)-\(suffix).png"))
        }
    }

    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }
}
