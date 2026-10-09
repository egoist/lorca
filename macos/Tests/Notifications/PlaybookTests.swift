import AppKit
import XCTest
@testable import Lorca

@MainActor
final class PlaybookTests: XCTestCase {
    private func descendants<T: NSView>(_ view: NSView, as type: T.Type) -> [T] {
        ((view as? T).map { [$0] } ?? []) + view.subviews.flatMap { descendants($0, as: type) }
    }

    private func record(status: String = "saved") -> PlaybookRecord {
        let content = PlaybookContent(name: "weekly-report", description: "Use for the Friday report", instructions: "Compare the numbers")
        let step = { (revision: Int, status: String) in
            PlaybookRevision(id: "r\(revision)", revision: revision, status: status, content: content,
                             provenance: .init(kind: revision == 1 ? "workflow" : "edit", chatId: nil, messageIds: []), deviceId: "", createdAt: Double(revision))
        }
        let steps = status == "draft" ? [step(1, "draft")] : [step(1, "draft"), step(2, "saved")]
        return PlaybookRecord(id: "playbook-test", scope: .bot("bot"), status: status, revision: steps.count, hash: "hash", content: content, revisions: steps)
    }

    func testFileEditsStayWithTheirFileAndUnsafeNamesAreRefused() throws {
        let pane = PlaybookFilesPane(folder: "references")
        pane.files = [PlaybookFile(path: "references/first.md", text: "First"), PlaybookFile(path: "references/second.md", text: "Second")]
        pane.text.string = "First, corrected"
        pane.table.selectRowIndexes([1], byExtendingSelection: false)
        XCTAssertEqual(pane.text.string, "Second")
        pane.text.string = "Second, corrected"
        XCTAssertEqual(pane.files.map(\.text), ["First, corrected", "Second, corrected"])

        let field = NSTextField(string: "../outside.md")
        field.tag = 0
        pane.controlTextDidEndEditing(Notification(name: NSControl.textDidEndEditingNotification, object: field))
        XCTAssertEqual(pane.files[0].path, "references/first.md")
        field.stringValue = "checklist.md"
        pane.controlTextDidEndEditing(Notification(name: NSControl.textDidEndEditingNotification, object: field))
        XCTAssertEqual(pane.files.map(\.path), ["references/checklist.md", "references/second.md"])
    }

    func testEditorSavesOnlyACompleteSkillAndShowsHistoryOnceSaved() throws {
        let new = PlaybookViewController(scope: .bot("bot"))
        _ = new.view
        let tabs = try XCTUnwrap(descendants(new.view, as: NSTabView.self).first)
        XCTAssertEqual(tabs.tabViewItems.map(\.label), [L("Instructions"), L("Examples"), L("References"), L("Scripts")])
        XCTAssertFalse(new.confirmButton.isEnabled)

        let saved = PlaybookViewController(scope: .bot("bot"), record: record())
        _ = saved.view
        XCTAssertTrue(saved.confirmButton.isEnabled)
        let savedTabs = try XCTUnwrap(descendants(saved.view, as: NSTabView.self).first)
        XCTAssertEqual(savedTabs.tabViewItems.last?.label, L("History"))
        let history = try XCTUnwrap(savedTabs.tabViewItems.last?.view as? PlaybookHistoryPane)
        XCTAssertEqual(history.table.numberOfRows, 2)
        XCTAssertFalse(history.text.isEditable)

        // A name typed the way people write it goes to the CLI as a slug; one it would refuse
        // keeps Save off.
        let name = try XCTUnwrap(descendants(saved.view, as: NSTextField.self).first { $0.isEditable && $0.stringValue == "weekly-report" })
        name.stringValue = "Weekly Report"
        saved.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: name))
        XCTAssertTrue(saved.confirmButton.isEnabled)
        name.stringValue = "weekly/report"
        saved.controlTextDidChange(Notification(name: NSControl.textDidChangeNotification, object: name))
        XCTAssertFalse(saved.confirmButton.isEnabled)
    }

    func testCapturePicksTheRequestWithTheReplyAndNeedsTwoCorrections() throws {
        let ask = Message(id: "ask", author: .you, body: .text("Compare this week's numbers"))
        let other = Message(id: "other", author: .bot("writer"), body: .text("Another bot's work"))
        let reply = Message(id: "reply", author: .bot("chef"), body: .text("Revenue is up 4%"))
        let fix = Message(id: "fix", author: .you, body: .text("Use metric units"))
        let again = Message(id: "again", author: .you, body: .text("Again: metric units"))
        let group = Chat(id: "room", kind: .group, botIDs: ["chef", "writer"], messages: [ask, other, reply, fix, again], unreadCount: 0, isPinned: false, createdAt: Date())

        let workflow = PlaybookCaptureViewController(chat: group, message: reply)
        _ = workflow.view
        let rows = descendants(workflow.view, as: SelectableMessageRow.self)
        XCTAssertEqual(rows.count, 2, "Another bot's messages and later ones stay out")
        XCTAssertTrue(rows.allSatisfy(\.isSelected), "The reply and the request before it start picked")
        XCTAssertTrue(workflow.confirmButton.isEnabled)
        XCTAssertEqual(descendants(workflow.view, as: NSPopUpButton.self).first?.numberOfItems, 2, "A group chooses whose skill it is")

        let corrections = PlaybookCaptureViewController(chat: group, message: again)
        _ = corrections.view
        let picks = descendants(corrections.view, as: SelectableMessageRow.self)
        XCTAssertEqual(picks.count, 3)
        XCTAssertEqual(picks.filter(\.isSelected).count, 1)
        XCTAssertFalse(corrections.confirmButton.isEnabled, "One correction is not a pattern")
        picks[1].onToggle?()
        XCTAssertTrue(corrections.confirmButton.isEnabled)
    }
}
