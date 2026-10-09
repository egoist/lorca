import XCTest
@testable import Lorca

final class BotPermissionTests: XCTestCase {
    func testPoliciesKeepInstanceIDsAndEmptyAllowlistsThroughTheWire() throws {
        let json = """
            {"connections":{"gmail-0123456789abcdef0123456789abcdef":{"capabilities":["read","draft"],"tools":["list_messages","create_draft"]},"gmail-personal":{"capabilities":[]}},"filesystem":"read","shell":false}
            """
        let policy = try Wire.decoder.decode(BotPermissions.self, from: Data(json.utf8))
        XCTAssertEqual(policy.level(of: "gmail-0123456789abcdef0123456789abcdef"), .draft)
        XCTAssertEqual(policy.level(of: "gmail-personal"), AccessLevel.none)
        XCTAssertEqual(policy.level(of: "github"), AccessLevel.none, "a plugin the map leaves out is off")
        XCTAssertEqual(policy.filesystem, .read)
        XCTAssertFalse(policy.shell)
        XCTAssertEqual(policy.summary, L("Limited"))
        let roundtrip = try Wire.decoder.decode(BotPermissions.self, from: JSONSerialization.data(withJSONObject: policy.json))
        XCTAssertEqual(roundtrip, policy)
        let full = try Wire.decoder.decode(BotPermissions.self, from: Data("{}".utf8))
        XCTAssertNil(full.connections)
        XCTAssertEqual(full.level(of: "github"), .write)
        XCTAssertEqual(full.summary, L("Full access"))
    }

    func testLevelsTakeInTheOnesBelowThem() {
        XCTAssertEqual(AccessLevel.write.capabilities, ["read", "draft", "write"])
        XCTAssertEqual(AccessLevel(capabilities: ["read", "draft"]), .draft)
        XCTAssertTrue(AccessLevel.draft.allows("read"))
        XCTAssertFalse(AccessLevel.draft.allows("write"))
    }

    func testAccessRequestsOfferTheAccessSheetAndNoGrantButton() throws {
        let json = """
            {"id":"m1","chat_id":"c1","author":{"kind":"bot","bot_id":"inbox"},"body":{"kind":"permission","plugin_id":"computer","plugin_name":"","tool":"access","summary":"Shell commands","decision":"pending","arguments":{}},"state":{"kind":"complete"},"created_at":1}
            """
        let message = try Wire.decoder.decode(Wire.Message.self, from: Data(json.utf8)).toModel()
        guard case let .permission(request) = message.body else { return XCTFail("not a permission card") }
        XCTAssertTrue(request.isAccess)
        XCTAssertFalse(request.isShell)
        XCTAssertEqual(request.shownSummary, L("Shell commands"))
        XCTAssertEqual(request.shownReason, L("Not allowed in this bot's Access settings."))
        XCTAssertEqual(request.choices.map(\.1), ["access", "deny"])
    }

    func testTheBotProfileCarriesPolicyIntoItsModel() throws {
        let json = """
            {"id":"inbox","name":"Inbox","description":"Read inbox","symbol_name":"envelope","accent":"indigo","runner_id":"runner","provider":"deepseek","created_at":1,"permissions":{"connections":{},"filesystem":"none","shell":false}}
            """
        let bot = try Wire.decoder.decode(Wire.Bot.self, from: Data(json.utf8)).toModel()
        XCTAssertEqual(bot.permissions?.connections, [:])
        XCTAssertEqual(bot.permissions?.filesystem, AccessLevel.none)
    }
}
