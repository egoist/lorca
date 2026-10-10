import XCTest
@testable import Lorca

@MainActor
final class BrowserProfileTests: XCTestCase {
    /// The Runner's `browser.sessions` answer, as the Profiles section reads it.
    func testProfilesDecodeFromTheRunnersAnswer() throws {
        let json = #"{"sessions":[{"id":"browser-1","bot_id":"bot-1","runner_id":"runner-2","name":"Work","state":"taking_over","selected":true,"revision":9,"created_at":1},{"id":"browser-2","name":"Shop","state":"human","revision":3,"recording":true}]}"#
        struct List: Decodable { let sessions: [BrowserProfile] }
        let profiles = try Wire.decoder.decode(List.self, from: Data(json.utf8)).sessions
        XCTAssertEqual(profiles.first, BrowserProfile(id: "browser-1", name: "Work", state: .takingOver, revision: 9))
        XCTAssertEqual(profiles.last, BrowserProfile(id: "browser-2", name: "Shop", state: .human, revision: 3, recording: true))
    }

    /// On the Runner, Record opens the browser for the user and the row reads Recording; Stop
    /// Recording sends it to the chat and closes the sheet over it.
    func testRecordAndStopRecordingFromTheProfilesMenu() async throws {
        try prepare()
        let store = AppStore.shared
        let bot = try XCTUnwrap(store.bot("bot-quill"))
        let runner = try browserRunner("dev-workbench")
        try await store.browserProfileAction("browser.create", botID: bot.id, params: ["name": "Work"])
        try await store.browserProfileAction("browser.create", botID: bot.id, params: ["name": "Personal"])
        let controller = PluginViewController(pluginID: BrowserProfile.pluginID, runner: runner, bot: bot, chatID: "chat-launch")
        let window = host(controller)
        defer { window.close() }
        try await wait { self.row("Work", in: controller.view) != nil }
        let work = try XCTUnwrap(row("Work", in: controller.view))
        XCTAssertEqual(titles(work.menu), ["Open Browser", "Record", "-", "Delete…"])

        try choose("Record", in: work)
        try await wait { self.labels(in: controller.view).contains("Recording") }
        XCTAssertEqual(titles(try XCTUnwrap(row("Work", in: controller.view)).menu), ["Stop Recording", "Take Screenshot", "Close Browser", "-", "Delete…"])
        XCTAssertEqual(titles(try XCTUnwrap(row("Personal", in: controller.view)).menu), ["Open Browser", "Record", "-", "Delete…"])
        try capture(controller, window: window, name: "mac-recording")

        try choose("Stop Recording", in: try XCTUnwrap(row("Work", in: controller.view)))
        try await wait { controller.presentingViewController == nil }
    }

    /// From another Device a closed browser records only on its Runner, and one open there records
    /// from here.
    func testAnotherDeviceRecordsOnlyABrowserOpenOnTheRunner() async throws {
        try prepare()
        let store = AppStore.shared
        let bot = try XCTUnwrap(store.bot("bot-scout"))
        let runner = try browserRunner("dev-studio")
        try await store.browserProfileAction("browser.create", botID: bot.id, params: ["name": "Research"])
        try await store.browserProfileAction("browser.create", botID: bot.id, params: ["name": "Personal"])
        let listed = try await store.browserProfiles(of: bot.id)
        let research = try XCTUnwrap(listed.first { $0.name == "Research" })
        try await store.browserProfileAction("browser.takeover", botID: bot.id, params: ["session_id": research.id])
        let controller = PluginViewController(pluginID: BrowserProfile.pluginID, runner: runner, bot: bot, chatID: "chat-scout")
        let window = host(controller)
        defer {
            controller.dismiss(nil)
            window.close()
        }
        try await wait { self.row("Personal", in: controller.view) != nil }
        let personal = try XCTUnwrap(row("Personal", in: controller.view))
        XCTAssertEqual(titles(personal.menu), ["Open on Studio", "Record on Studio", "-", "Delete…"])
        XCTAssertEqual(personal.menu?.items.prefix(2).map(\.isEnabled), [false, false])
        XCTAssertEqual(titles(try XCTUnwrap(row("Research", in: controller.view)).menu), ["Return to Bot", "Record", "Take Screenshot", "Close Browser", "-", "Delete…"])
    }

    private func prepare() throws {
        guard AppStore.shared.isMock else { throw XCTSkip("The sheets run against the demo roster: set LORCA_MOCK=1") }
        _ = NSApplication.shared
        AppStore.shared.start()
    }

    /// The demo Runner with Browser installed.
    private func browserRunner(_ id: Device.ID) throws -> Device {
        var runner = try XCTUnwrap(AppStore.shared.devices.first { $0.id == id })
        runner.plugins.append(InstalledPlugin(id: BrowserProfile.pluginID, name: "Browser", description: "Opens web pages in a browser on the Runner to read them, fill forms, and take screenshots.", version: "1", icon: "globe", state: .ready, detail: "Ready"))
        return runner
    }

    private func descendants(_ view: NSView) -> [NSView] {
        view.subviews + view.subviews.flatMap(descendants)
    }

    private func labels(in view: NSView) -> [String] {
        descendants(view).compactMap { ($0 as? NSTextField)?.stringValue }
    }

    private func row(_ title: String, in view: NSView) -> StatusRow? {
        descendants(view).compactMap { $0 as? StatusRow }.first { labels(in: $0).contains(title) }
    }

    private func titles(_ menu: NSMenu?) -> [String] {
        (menu?.items ?? []).map { $0.isSeparatorItem ? "-" : $0.title }
    }

    private func choose(_ title: String, in row: StatusRow) throws {
        let menu = try XCTUnwrap(row.menu)
        let index = try XCTUnwrap(menu.items.firstIndex { $0.title == title })
        menu.performActionForItem(at: index)
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
        for _ in 0..<300 {
            if condition() { return }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTFail("the sheet never got there")
    }
}
