import XCTest
@testable import Lorca

final class BrowserProfileTests: XCTestCase {
    /// The Runner's `browser.sessions` answer, as the Profiles section reads it.
    func testProfilesDecodeFromTheRunnersAnswer() throws {
        let json = #"{"sessions":[{"id":"browser-1","bot_id":"bot-1","runner_id":"runner-2","name":"Work","state":"taking_over","selected":true,"revision":9,"created_at":1}]}"#
        struct List: Decodable { let sessions: [BrowserProfile] }
        let profile = try XCTUnwrap(Wire.decoder.decode(List.self, from: Data(json.utf8)).sessions.first)
        XCTAssertEqual(profile, BrowserProfile(id: "browser-1", name: "Work", state: .takingOver, revision: 9))
    }
}
