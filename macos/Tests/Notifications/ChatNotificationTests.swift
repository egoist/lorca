import XCTest
@testable import Lorca

final class ChatNotificationTests: XCTestCase {
    private let start = Date(timeIntervalSince1970: 100)

    private func chat(_ messages: [Message]) -> Chat {
        Chat(id: "chat", kind: .dm, botIDs: ["bot"], messages: messages,
             unreadCount: 1, isPinned: false, createdAt: start)
    }

    private func reply(_ text: String, state: Message.State = .complete) -> Message {
        Message(author: .bot("bot"), body: .text(text), state: state, createdAt: start)
    }

    func testFailureAfterPartialOutputNotifiesWithTheError() throws {
        let partial = reply("I have started the work.")
        let failed = reply("Partial output", state: .failed("Provider connection lost"))
        let notification = try XCTUnwrap(ChatNotification.finishedTurn(in: chat([partial, failed]), botID: "bot", startedAt: start))
        XCTAssertEqual(notification.messageID, failed.id)
        XCTAssertEqual(notification.kind, .failure)
        XCTAssertTrue(notification.body.contains("Provider connection lost"))
        XCTAssertFalse(notification.body.contains("Partial output"))
    }

    func testRecoveredFailureUsesTheSuccessfulReply() throws {
        let failed = reply("", state: .failed("Context too long"))
        let completed = reply("Done after recovery")
        let notification = try XCTUnwrap(ChatNotification.finishedTurn(in: chat([failed, completed]), botID: "bot", startedAt: start))
        XCTAssertEqual(notification.kind, .reply)
        XCTAssertEqual(notification.messageID, completed.id)
    }

    func testPendingConfirmationCanNotifyBeforeTheTurnEnds() throws {
        var request = PermissionRequest(pluginID: "computer", pluginName: "Mac", tool: "bash",
                                        summary: "Run the deployment", decision: .pending)
        var message = Message(author: .bot("bot"), body: .permission(request), createdAt: start)
        let notification = try XCTUnwrap(ChatNotification(message))
        XCTAssertEqual(notification.kind, .permission)
        XCTAssertTrue(notification.body.contains(request.summary))
        XCTAssertTrue(notification.canDeliver(in: chat([message]), watchedChat: nil))
        for decision: PermissionRequest.Decision in [.allowed, .always, .denied, .expired] {
            request.decision = decision
            message.body = .permission(request)
            XCTAssertFalse(notification.canDeliver(in: chat([message]), watchedChat: nil))
        }
        XCTAssertFalse(notification.canDeliver(in: chat([]), watchedChat: nil))
    }

    func testReadOrVisibleChatSuppressesDelivery() throws {
        let message = reply("", state: .failed("Request failed"))
        let notification = try XCTUnwrap(ChatNotification(message))
        var current = chat([message])
        XCTAssertTrue(notification.canDeliver(in: current, watchedChat: "another-chat"))
        XCTAssertFalse(notification.canDeliver(in: current, watchedChat: current.id))
        current.unreadCount = 0
        XCTAssertFalse(notification.canDeliver(in: current, watchedChat: nil))
    }

    func testAMutedChatStaysQuietUntilItsMuteRunsOut() throws {
        let message = reply("Done")
        let notification = try XCTUnwrap(ChatNotification(message))
        var current = chat([message])
        current.mute = Chat.Mute(until: Date().addingTimeInterval(3600))
        XCTAssertFalse(notification.canDeliver(in: current, watchedChat: nil))
        current.mute = Chat.Mute(until: nil)
        XCTAssertFalse(notification.canDeliver(in: current, watchedChat: nil))
        current.mute = Chat.Mute(until: Date().addingTimeInterval(-1))
        XCTAssertTrue(notification.canDeliver(in: current, watchedChat: nil))
    }

    func testStreamingEmptyAndEarlierRepliesStayQuiet() {
        var earlier = reply("Old reply")
        earlier.createdAt = start.addingTimeInterval(-60)
        let messages = [earlier, reply("Not finished", state: .streaming), reply("")]
        XCTAssertNil(ChatNotification.finishedTurn(in: chat(messages), botID: "bot", startedAt: start))
        XCTAssertNil(ChatNotification.finishedTurn(in: chat([reply("Done")]), botID: "another-bot", startedAt: start))
    }

    func testStructuredAttentionAlertsOnceAndQuietReportsHaveNoAlert() throws {
        var summary = reply("Decision: review the draft")
        summary.notification = "summary"
        XCTAssertEqual(ChatNotification(summary)?.kind, .summary)
        XCTAssertNil(ChatNotification.finishedTurn(in: chat([summary]), botID: "bot", startedAt: start))
        var urgent = reply("Urgent: a release is blocked")
        urgent.notification = "urgent"
        XCTAssertEqual(ChatNotification(urgent)?.kind, .urgent)
        var quiet = reply("Routine check: no changes")
        quiet.notification = "quiet"
        XCTAssertNil(ChatNotification(quiet))
        XCTAssertNil(ChatNotification.finishedTurn(in: chat([quiet, summary, urgent]), botID: "bot", startedAt: start))
    }

    func testAttentionSnapshotDecodesCoordinatorAndCanonicalSourceReferences() throws {
        let json = #"{"items":[{"id":"attention-item","category":"review","title":"Review draft","summary":"Decide scope","next_action":"Approve scope","coordinator_bot_id":"bot","sources":[{"chat_id":"chat","task_id":"task-08a3fb26-3a14-4084-ae89-10f7d4028c47","review_id":"review-1"}],"reporters":["scout"],"urgent":false,"revision":{"counter":2,"device_id":"runner"}}],"briefs":[],"preferences":{"summaries":false,"urgent_direct":true,"default_coordinator_bot_id":"bot","coordinators":{}}}"#
        let attention = try Wire.decoder.decode(AttentionView.self, from: Data(json.utf8))
        XCTAssertEqual(attention.items.first?.sources.first?.reviewId, "review-1")
        XCTAssertEqual(attention.items.first?.coordinatorBotId, "bot")
        XCTAssertEqual(attention.items.first?.revision.deviceId, "runner")
        XCTAssertFalse(attention.preferences.summaries)
        XCTAssertTrue(attention.preferences.urgentDirect)
    }
}
