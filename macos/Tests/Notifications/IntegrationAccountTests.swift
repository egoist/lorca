import XCTest
@testable import Lorca

final class IntegrationAccountTests: XCTestCase {
    func testNamedAccountStatusKeepsTheInstanceAndServiceSeparate() throws {
        let json = #"{"id":"gmail-0123456789abcdef0123456789abcdef","name":"Gmail · Work","state":"insufficient_access","detail":"Sign in again to grant the required access","service_id":"gmail","account_name":"Work"}"#
        let account = try Wire.decoder.decode(Wire.PluginStatus.self, from: Data(json.utf8)).toModel()
        XCTAssertEqual(account.id, "gmail-0123456789abcdef0123456789abcdef")
        XCTAssertEqual(account.marketplaceID, "gmail")
        XCTAssertEqual(account.accountName, "Work")
        XCTAssertEqual(account.state, .insufficientAccess)
        XCTAssertFalse(account.isMcpServer)
    }

    func testLegacyStatusesRemainReadable() throws {
        let json = #"{"id":"github","name":"GitHub","state":"ready","detail":"Ready"}"#
        let plugin = try Wire.decoder.decode(Wire.PluginStatus.self, from: Data(json.utf8)).toModel()
        XCTAssertEqual(plugin.marketplaceID, "github")
        XCTAssertNil(plugin.serviceID)
        XCTAssertNil(plugin.accountName)
        XCTAssertEqual(plugin.state, .ready)
    }

    func testMarketplaceOffersAccountsWithoutInstallingConfigurationInTheApp() throws {
        let json = #"{"id":"gmail","name":"Gmail","named_accounts":true,"servers":{"gmail":{"type":"http","url":"https://gmailmcp.googleapis.com/mcp/v1","auth":{"type":"oauth"}}},"installed_on":["runner"]}"#
        let plugin = try Wire.decoder.decode(Wire.MarketplacePlugin.self, from: Data(json.utf8)).toModel()
        XCTAssertTrue(plugin.namedAccounts)
        XCTAssertTrue(plugin.signsIn)
        XCTAssertEqual(plugin.installedOn, ["runner"])
    }
}
