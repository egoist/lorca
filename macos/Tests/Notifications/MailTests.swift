import XCTest
@testable import Lorca

@MainActor
final class MailTests: XCTestCase {
    func testStatusReadsTheCLIsShape() throws {
        let json = """
            {"available":true,"domain":"bots.lorca.app","address":{"name":"k7f3m9q2","email":"k7f3m9q2@bots.lorca.app","state":"suspended"},"lead_bot_id":"bot-1","bots":[{"bot_id":"bot-2","email":"k7f3m9q2+scout@bots.lorca.app"}]}
            """
        let status = try Wire.decoder.decode(MailStatus.self, from: Data(json.utf8))
        XCTAssertEqual(status.address?.state, .suspended)
        XCTAssertEqual(status.leadBotId, "bot-1")
        XCTAssertEqual(status.email(of: "bot-2"), "k7f3m9q2+scout@bots.lorca.app")
        let off = try Wire.decoder.decode(MailStatus.self, from: Data(#"{"available":false,"domain":null,"address":null,"lead_bot_id":null}"#.utf8))
        XCTAssertFalse(off.available)
        XCTAssertEqual(off.bots, [], "a status without bots has none")
        let reply = try Wire.decoder.decode(Wire.MailReply.self, from: Data(#"{"problem":"taken"}"#.utf8))
        XCTAssertEqual(reply.problem, .taken)
        XCTAssertNil(reply.mail)
    }

    func testNamesFollowTheRelaysRules() {
        for good in ["egoist", "k7f3m9q2", "a.b-c", "abc"] { XCTAssertTrue(MailStatus.isValidName(good), good) }
        for bad in ["ab", "Egoist", "a..b", ".abc", "abc-", "a+b", "名字abc", String(repeating: "a", count: 33)] {
            XCTAssertFalse(MailStatus.isValidName(bad), bad)
        }
        XCTAssertEqual(MailStatus.normalized("  Egoist "), "egoist")
    }

    func testSettingsListsEmailOnlyWhileTheRelayOffersIt() throws {
        try prepare()
        let store = AppStore.shared
        store.applyMail(MailStatus(available: false, domain: nil, address: nil, leadBotId: nil, bots: []))
        XCTAssertFalse(SettingsPane.listed(in: store).contains(.email))
        store.applyMail(MockData.mail())
        XCTAssertTrue(SettingsPane.listed(in: store).contains(.email))
    }

    func testThePaneOffersAnAddressThenItsActions() throws {
        try prepare()
        let store = AppStore.shared
        let mock = MockData.mail()
        store.applyMail(MailStatus(available: true, domain: mock.domain, address: nil, leadBotId: mock.leadBotId, bots: []))
        let pane = EmailSettingsViewController()
        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 640, height: 360), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = pane
        window.setContentSize(NSSize(width: 640, height: 360))
        defer { window.close() }
        pane.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(buttons(in: pane.view), ["Get an Address…"])
        try captureView(pane.view, window: window, name: "email-none")

        store.applyMail(mock)
        pane.view.layoutSubtreeIfNeeded()
        XCTAssertEqual(buttons(in: pane.view), ["Copy", "Change…", "Give Up Address…"])
        XCTAssertTrue(labels(in: pane.view).contains("k7f3m9q2@bots.lorca.app"))
        let note = try XCTUnwrap(labels(in: pane.view).first { $0.hasPrefix("Each bot writes") })
        XCTAssertTrue(note.contains("k7f3m9q2+developer@bots.lorca.app") && note.hasSuffix("Project Manager."), note)
        let row = try XCTUnwrap(descendants(pane.view).compactMap { $0 as? ActionRow }.first)
        XCTAssertEqual(row.menu?.items.map(\.title), ["Copy Address", "Change Address…", "", "Give Up Address…"])
        try captureView(pane.view, window: window, name: "email-address")

        var suspended = mock
        suspended.address?.state = .suspended
        store.applyMail(suspended)
        pane.view.layoutSubtreeIfNeeded()
        XCTAssertTrue(labels(in: pane.view).contains("Suspended"))
        XCTAssertTrue(labels(in: pane.view).contains { $0.hasPrefix("Mail to this address bounces for now") })
        try captureView(pane.view, window: window, name: "email-suspended")
        store.applyMail(mock)
    }

    func testTheSheetSaysWhyANameWasNotTaken() async throws {
        try prepare()
        let store = AppStore.shared
        store.applyMail(MockData.mail())
        let controller = EmailAddressViewController(current: store.mail?.address, domain: "bots.lorca.app")
        let window = host(controller)
        defer {
            controller.dismiss(nil)
            window.close()
        }
        try await wait { controller.view.window != nil }
        XCTAssertEqual(controller.confirmButton.title, "Change Address")
        XCTAssertTrue(controller.confirmButton.isEnabled, "a random name needs nothing typed")
        try capture(controller, window: window, name: "email-change")

        let field = try XCTUnwrap(descendants(controller.view).compactMap { $0 as? NSTextField }.first { $0.isEditable })
        field.stringValue = "Admin"
        controller.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: field))
        XCTAssertEqual(field.stringValue, "admin", "what is typed is lowercased")
        controller.confirmButton.performClick(nil)
        try await wait { self.labels(in: controller.view).contains("That name is reserved.") }
        try capture(controller, window: window, name: "email-reserved")

        field.stringValue = "egoist"
        controller.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: field))
        XCTAssertFalse(labels(in: controller.view).contains("That name is reserved."))
        controller.confirmButton.performClick(nil)
        try await wait { store.mail?.address?.email == "egoist@bots.lorca.app" }
        store.applyMail(MockData.mail())
    }

    func testTheFirstSheetGetsAnAddress() async throws {
        try prepare()
        let controller = EmailAddressViewController(current: nil, domain: "bots.lorca.app")
        let window = host(controller)
        defer {
            controller.dismiss(nil)
            window.close()
        }
        try await wait { controller.view.window != nil }
        XCTAssertEqual(controller.confirmButton.title, "Get Address")
        XCTAssertFalse(labels(in: controller.view).contains { $0.contains("stops working") }, "a first address gives nothing up")
        try capture(controller, window: window, name: "email-get")
    }

    func testTheAccessSheetListsEmailLikeAPlugin() async throws {
        try prepare()
        let bot = try XCTUnwrap(AppStore.shared.bots.first)
        let controller = BotAccessViewController(botID: bot.id)
        let window = host(controller)
        defer {
            controller.dismiss(nil)
            window.close()
        }
        try await wait { self.labels(in: controller.view).contains("Email") }
        try capture(controller, window: window, name: "email-access")
    }

    // MARK: - Helpers

    private func prepare() throws {
        guard AppStore.shared.isMock else { throw XCTSkip("The pane runs against the demo roster: set LORCA_MOCK=1") }
        _ = NSApplication.shared
        AppStore.shared.start()
        LinkBox.pasteboard = NSPasteboard(name: NSPasteboard.Name("app.lorca.tests"))
    }

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

    /// A pane as it shows, in each appearance, when `LORCA_UI_CAPTURE_DIR` names a folder.
    private func captureView(_ view: NSView, window: NSWindow, name: String) throws {
        guard let folder = ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"] else { return }
        for (suffix, appearance) in [("light", NSAppearance.Name.aqua), ("dark", .darkAqua)] {
            window.appearance = NSAppearance(named: appearance)
            view.layoutSubtreeIfNeeded()
            view.displayIfNeeded()
            let bitmap = try XCTUnwrap(view.bitmapImageRepForCachingDisplay(in: view.bounds))
            view.cacheDisplay(in: view.bounds, to: bitmap)
            try write(bitmap, folder: folder, name: "\(name)-\(suffix)")
        }
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
            try write(bitmap, folder: folder, name: "\(name)-\(suffix)")
        }
    }

    private func write(_ bitmap: NSBitmapImageRep, folder: String, name: String) throws {
        let png = try XCTUnwrap(bitmap.representation(using: .png, properties: [:]))
        try FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
        try png.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name).png"))
    }

    private func wait(_ condition: () -> Bool) async throws {
        for _ in 0..<200 {
            if condition() { return }
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTFail("the view never got there")
    }

    private func descendants(_ view: NSView) -> [NSView] { [view] + view.subviews.flatMap(descendants) }

    private func labels(in view: NSView) -> [String] {
        descendants(view).compactMap { ($0 as? NSTextField).flatMap { $0.isEditable ? nil : $0.stringValue } }
    }

    /// The visible buttons' titles, in the order the rows hold them.
    private func buttons(in view: NSView) -> [String] {
        descendants(view).compactMap { $0 as? NSButton }.filter { !$0.isHidden && !$0.title.isEmpty }.map(\.title)
    }
}
