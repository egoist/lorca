import XCTest
@testable import Lorca

/// A routine as the CLI sends it: its timezone, its missed-run policy, how it stands, and its
/// check health, in the words the inspector row and the routine sheet use.
final class RoutineReliabilityTests: XCTestCase {
    private func routine(_ json: String) throws -> Routine {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let base = #""id":"rt-1","bot_id":"b1","name":"Brief","prompt":"Read the inbox","schedule":"0 9 * * 1-5","schedule_text":"Weekdays at 9:00 AM","created_at":0"#
        return try decoder.decode(Wire.Routine.self, from: Data("{\(base),\(json)}".utf8)).toModel()
    }

    func testHealthAndPolicyDecode() throws {
        let quiet = try routine(#""is_enabled":true,"timezone":"America/New_York","missed_run_policy":"skip","state":"on","check":"return null","health":{"last_check_at":100,"last_success_at":100,"status":"quiet"}"#)
        XCTAssertEqual(quiet.timezone, "America/New_York")
        XCTAssertEqual(quiet.missedRunPolicy, "skip")
        XCTAssertEqual(quiet.health.lastSuccessAt, Date(timeIntervalSince1970: 100))
        XCTAssertNil(quiet.lastRunAt, "a check is not a run")
        XCTAssertNil(quiet.problem)
        XCTAssertEqual(quiet.lastCheckSummary, L("%@ · nothing new", Format.daySeparator(Date(timeIntervalSince1970: 100))))

        let plain = try routine(#""is_enabled":true"#)
        XCTAssertEqual(plain.missedRunPolicy, "coalesce")
        XCTAssertEqual(plain.timezone, TimeZone.current.identifier)
        XCTAssertNil(plain.lastCheckSummary, "no check, no check line")
    }

    func testProblemsSayWhatWentWrong() throws {
        let signedOut = try routine(#""is_enabled":false,"paused_reason":"authentication","state":"blocked","check":"x","health":{"status":"blocked","authentication_failures":3}"#)
        XCTAssertEqual(signedOut.problem, .signedOut(model: false))
        XCTAssertEqual(signedOut.problem?.text, L("Needs sign-in"))
        let model = try routine(#""is_enabled":false,"paused_reason":"authentication","state":"blocked","health":{"model":{"status":"blocked","authentication_failures":3}}"#)
        XCTAssertEqual(model.problem, .signedOut(model: true))
        XCTAssertEqual(try routine(#""is_enabled":true,"state":"waiting_for_runner""#).problem, .offline)
        let connection = try routine(#""is_enabled":true,"state":"failed","check":"x","health":{"status":"failed","connection_failures":2}"#)
        XCTAssertEqual(connection.problem, .cantConnect(model: false))
        XCTAssertFalse(connection.problem!.needsUser, "a failed connection is tried again on its own")
        XCTAssertEqual(try routine(#""is_enabled":true,"state":"failed","check":"x","health":{"status":"failed"}"#).problem, .checkFailed)
        XCTAssertEqual(try routine(#""is_enabled":true,"state":"blocked","check":"x","health":{"status":"blocked"}"#).problem, .checkBlocked)
        XCTAssertNil(try routine(#""is_enabled":true,"state":"failed","is_running":true"#).problem, "a run going on says Running")
        XCTAssertTrue(signedOut.detail.hasPrefix(L("Needs sign-in")))
    }

    func testScheduleNamesAnotherTimezoneOnly() throws {
        let here = try routine(#""is_enabled":true,"timezone":"\#(TimeZone.current.identifier)""#)
        XCTAssertEqual(here.scheduleSummary, "Weekdays at 9:00 AM")
        let elsewhere = TimeZone.current.identifier == "Pacific/Kiritimati" ? "Europe/London" : "Pacific/Kiritimati"
        let away = try routine(#""is_enabled":true,"timezone":"\#(elsewhere)""#)
        XCTAssertNotEqual(away.scheduleSummary, "Weekdays at 9:00 AM")
        XCTAssertTrue(away.scheduleSummary.hasPrefix("Weekdays at 9:00 AM ("))
        var interval = away
        interval.schedule = "every 2h"
        XCTAssertEqual(interval.scheduleSummary, interval.scheduleText, "an interval counts time in any zone")
    }
}
