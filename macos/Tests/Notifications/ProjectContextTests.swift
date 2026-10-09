import XCTest
@testable import Lorca

@MainActor
final class ProjectContextTests: XCTestCase {
    /// An entry as `projects.get` lists it.
    private func entry(_ id: String, kind: String, updated: Int64, source: String = #"{"kind":"user","label":"User"}"#, verification: String = "agreed", extra: String = "") throws -> ProjectEntry {
        let json = #"{"id":"\#(id)","kind":"\#(kind)","title":"\#(id)","text":"","source":\#(source),"verification":"\#(verification)","freshness":"\#(verification)","updated_at":\#(updated),"current":true,"removed":false\#(extra)}"#
        return try Wire.decoder.decode(ProjectEntry.self, from: Data(json.utf8))
    }

    func testEntriesListBriefsFirstAndNewestFirstWithinAKind() throws {
        let entries = try [
            entry("fact", kind: "fact", updated: 30),
            entry("old-decision", kind: "decision", updated: 10),
            entry("brief", kind: "brief", updated: 1),
            entry("new-decision", kind: "decision", updated: 20),
        ]
        XCTAssertEqual(ProjectContext.ordered(entries).map(\.id), ["brief", "new-decision", "old-decision", "fact"])
    }

    func testABotsProposalIsASuggestionAndAnAgreedDecisionsLinkIsNotChecked() throws {
        let proposal = try entry("p", kind: "fact", updated: 1, source: #"{"kind":"bot","label":"Scout"}"#, verification: "unverified")
        XCTAssertTrue(proposal.isSuggestion)
        let link = #"{"kind":"url","label":"docs.example.com","url":"https://www.example.com/a"}"#
        let decision = try entry("d", kind: "decision", updated: 1, source: link)
        XCTAssertFalse(decision.canCheckLink)
        let fetched = try entry("l", kind: "document", updated: 1, source: link, verification: "fetched", extra: #","fetched_at":5"#)
        XCTAssertTrue(fetched.canCheckLink)
        XCTAssertEqual(fetched.host, "example.com")
        XCTAssertEqual(fetched.checked, Date(timeIntervalSince1970: 5))
    }

    func testACorrectionCarriesItsSourceAlongWhole() throws {
        let output = #"{"kind":"output","label":"Build","output":{"chat_id":"g","message_id":"m","output_id":"o","version":2}}"#
        let json = try entry("o", kind: "fact", updated: 1, source: output).source.json
        XCTAssertEqual((json["output"] as? [String: Any])?["output_id"] as? String, "o")
        XCTAssertEqual((json["output"] as? [String: Any])?["version"] as? Int, 2)
        XCTAssertNil(json["url"])
    }

    func testTwoVersionsOfOneEntryNameEachOther() {
        let context = ProjectContext(entries: [], conflicts: [["a", "b"]])
        XCTAssertEqual(context.otherVersions(of: "a"), ["b"])
        XCTAssertEqual(context.otherVersions(of: "c"), [])
    }

    func testTheSheetSaysWhatItsButtonDoes() throws {
        let suggestion = try entry("p", kind: "fact", updated: 1, source: #"{"kind":"bot","label":"Scout"}"#, verification: "unverified")
        let accept = ProjectEntryViewController(chatID: "g", entry: suggestion, kind: .fact)
        _ = accept.view
        XCTAssertEqual(accept.confirmButton.title, "Accept")
        XCTAssertTrue(accept.confirmButton.isEnabled, "a suggestion is accepted as it stands")

        let saved = ProjectEntryViewController(chatID: "g", entry: try entry("b", kind: "brief", updated: 1), kind: .brief)
        _ = saved.view
        XCTAssertEqual(saved.confirmButton.title, "Save")
        XCTAssertFalse(saved.confirmButton.isEnabled, "nothing to save until something changes")

        let conflicted = ProjectEntryViewController(chatID: "g", entry: try entry("b", kind: "brief", updated: 1), kind: .brief, otherVersions: ["c"])
        _ = conflicted.view
        XCTAssertEqual(conflicted.confirmButton.title, "Keep This Version")
        XCTAssertTrue(conflicted.confirmButton.isEnabled)

        let link = ProjectEntryViewController(chatID: "g", entry: nil, kind: .document)
        _ = link.view
        XCTAssertEqual(link.confirmButton.title, "Add")
        XCTAssertFalse(link.confirmButton.isEnabled, "a link needs a title and its address")
    }
}
