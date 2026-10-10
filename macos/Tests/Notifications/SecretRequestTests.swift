import AppKit
import XCTest
@testable import Lorca

/// A bot's secret request on its card, and a Runner's secrets in Settings, against the demo
/// roster. `LORCA_UI_CAPTURE_DIR` also saves each as a 2x PNG in the light and dark appearances.
@MainActor
final class SecretRequestTests: XCTestCase {
    func testASecretRequestDecodesFromTheCLIsShape() throws {
        let json = #"""
        {"id":"msg-1","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},"state":{"kind":"complete"},"created_at":1,
         "body":{"kind":"permission","plugin_id":"playwright","plugin_name":"Browser","tool":"secret","summary":"GitHub password, One-time code",
                 "decision":"pending","reason":"To open the pull request.",
                 "secret":{"use":"browser","site":"github.com","fields":[{"name":"password","label":"GitHub password"},{"name":"otp","label":"One-time code"}]}}}
        """#
        let message = try Wire.decoder.decode(Wire.Message.self, from: Data(json.utf8)).toModel()
        guard case let .permission(request) = message.body else { return XCTFail("not a card") }
        XCTAssertTrue(request.isSecret)
        XCTAssertEqual(request.secret, SecretAsk(use: .browser, site: "github.com", fields: [.init(name: "password", label: "GitHub password"), .init(name: "otp", label: "One-time code")]))
        XCTAssertEqual(request.verbPhrase, "needs a secret for github.com")
        XCTAssertEqual(ChatNotification(message)?.body, "Asks for GitHub password, One-time code")
        var answered = request
        answered.decision = .allowed
        XCTAssertEqual(SecretCellView.caption(for: answered), "Saved · GitHub password, One-time code")
        answered.decision = .denied
        XCTAssertEqual(answered.decisionText, "Not now")
        var command = request
        command.secret?.use = .command
        XCTAssertEqual(command.verbPhrase, "needs a secret for its commands")
    }

    func testARunnersSecretsDecodeWithoutValues() throws {
        let json = #"{"secrets":[{"id":"secret-1","bot_id":"bot-1","name":"NPM_TOKEN","label":"npm token","use":"command","updated_at":1700000000}]}"#
        let secret = try XCTUnwrap(Wire.decoder.decode(Wire.SecretList.self, from: Data(json.utf8)).secrets.first?.toModel())
        XCTAssertEqual(secret, SavedSecret(id: "secret-1", botID: "bot-1", name: "NPM_TOKEN", label: "npm token", use: .command, site: nil, updatedAt: Date(timeIntervalSince1970: 1_700_000_000)))
    }

    /// The card asks for each value, saves only once each has one, and reads Saved after.
    func testTheCardTakesEachValueAndSaves() async throws {
        try prepare()
        let chat = ChatViewController()
        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 620, height: 560), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = chat
        window.setContentSize(NSSize(width: 620, height: 560))
        defer { window.close() }
        _ = AppStore.shared.append(Message(author: .you, body: .text("Publish the CLI package to npm once the review is done."), createdAt: Date().addingTimeInterval(-60)), to: "chat-patch")
        _ = AppStore.shared.append(Message(
            author: .bot("bot-patch"),
            body: .permission(PermissionRequest(
                pluginID: "computer", pluginName: "Studio", tool: "secret", summary: "npm token", decision: .pending,
                reason: "To publish the package to npm for you.",
                secret: SecretAsk(use: .command, site: nil, fields: [.init(name: "NPM_TOKEN", label: "npm token")]))),
            createdAt: Date()), to: "chat-patch")
        chat.show(chatID: "chat-patch")
        try await settle(chat.view)
        let card = try XCTUnwrap(descendants(chat.view).compactMap { $0 as? SecretCellView }.first)
        let fields = descendants(card).compactMap { $0 as? NSSecureTextField }
        XCTAssertEqual(fields.map(\.placeholderString), ["npm token"])
        let save = try XCTUnwrap(descendants(card).compactMap { $0 as? NSButton }.first { $0.title == "Save" })
        XCTAssertFalse(save.isEnabled, "nothing typed yet")
        XCTAssertTrue(labels(in: card).contains("Developer needs a secret for its commands"))
        XCTAssertTrue(labels(in: card).contains("Saved on Studio. Developer never sees it."))
        try captureView(chat.view, window: window, name: "secret-card")

        fields[0].stringValue = "npm_0123456789"
        NotificationCenter.default.post(name: NSControl.textDidChangeNotification, object: fields[0])
        XCTAssertTrue(save.isEnabled)
        save.performClick(nil)
        try await wait {
            guard case let .permission(request) = AppStore.shared.chat("chat-patch")?.messages.last?.body else { return false }
            return request.decision == .allowed
        }
        try await settle(chat.view)
        let saved = try XCTUnwrap(descendants(chat.view).compactMap { $0 as? SecretCellView }.first)
        XCTAssertTrue(labels(in: saved).contains("Saved · npm token"))
        XCTAssertTrue(descendants(saved).compactMap { $0 as? NSSecureTextField }.isEmpty, "an answered card holds no field")
        try captureView(chat.view, window: window, name: "secret-card-saved")
    }

    func testSettingsListsARunnersSecrets() async throws {
        try prepare()
        let pane = SecretsSettingsViewController()
        let window = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 640, height: 420), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentViewController = pane
        window.setContentSize(NSSize(width: 640, height: 420))
        defer { window.close() }
        pane.show(deviceID: "dev-studio")
        pane.viewWillAppear()
        try await wait { self.descendants(pane.view).contains { $0 is BotRow } }
        pane.view.layoutSubtreeIfNeeded()
        let rows = descendants(pane.view).compactMap { $0 as? BotRow }
        XCTAssertEqual(rows.count, 2)
        XCTAssertTrue(labels(in: pane.view).isSuperset(of: ["Secrets on Studio", "GitHub password", "Developer · github.com", "Semantic Scholar API key", "Researcher · Commands"]))
        XCTAssertEqual(rows[0].menu?.items.map(\.title), ["Replace…", "", "Delete…"])
        try captureView(pane.view, window: window, name: "secrets-pane")

        let sheet = SecretViewController(secret: MockData.secrets[0].secret, botName: "Developer", runner: try XCTUnwrap(AppStore.shared.device("dev-studio")))
        let parent = host(sheet)
        defer { parent.close() }
        XCTAssertFalse(sheet.confirmButton.isEnabled, "Replace waits for a value")
        try capture(sheet, window: parent, name: "secret-sheet")
    }

    // MARK: - Helpers

    private func prepare() throws {
        guard AppStore.shared.isMock else { throw XCTSkip("The views run against the demo roster: set LORCA_MOCK=1") }
        _ = NSApplication.shared
        AppStore.shared.start()
    }

    private func settle(_ view: NSView) async throws {
        for _ in 0..<5 {
            try await Task.sleep(nanoseconds: 50_000_000)
            view.layoutSubtreeIfNeeded()
            view.displayIfNeeded()
        }
    }

    private func wait(_ condition: @escaping () -> Bool) async throws {
        for _ in 0..<100 where !condition() {
            try await Task.sleep(nanoseconds: 20_000_000)
        }
        XCTAssertTrue(condition())
    }

    private func descendants(_ view: NSView) -> [NSView] {
        view.subviews + view.subviews.flatMap(descendants)
    }

    private func labels(in view: NSView) -> Set<String> {
        Set(descendants(view).compactMap { ($0 as? NSTextField)?.stringValue }.filter { !$0.isEmpty })
    }

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
        let parent = NSWindow(contentRect: NSRect(x: -4000, y: -4000, width: 700, height: 500), styleMask: [.titled], backing: .buffered, defer: false)
        parent.isReleasedWhenClosed = false
        let root = NSViewController()
        root.view = NSView(frame: NSRect(x: 0, y: 0, width: 700, height: 500))
        parent.contentViewController = root
        parent.orderFront(nil)
        root.presentAsSheet(controller)
        return parent
    }

    private func capture(_ controller: SheetViewController, window parent: NSWindow, name: String) throws {
        guard ProcessInfo.processInfo.environment["LORCA_UI_CAPTURE_DIR"] != nil else { return }
        let sheet = try XCTUnwrap(controller.view.window)
        for appearance in [NSAppearance.Name.aqua, .darkAqua] {
            sheet.appearance = NSAppearance(named: appearance)
        }
        try captureView(try XCTUnwrap(sheet.contentView?.superview), window: sheet, name: name)
    }
}
