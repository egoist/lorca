import Foundation

/// The message behind an alert, kept so a delayed delivery can check its current state.
struct ChatNotification: Equatable {
    enum Kind { case reply, permission, failure }

    let kind: Kind
    let messageID: Message.ID
    let botID: Bot.ID
    let body: String

    init?(_ message: Message) {
        guard let botID = message.author.botID else { return nil }
        self.botID = botID
        messageID = message.id
        switch message.body {
        case let .permission(request) where request.isPending:
            kind = .permission
            body = L("Confirmation needed: %@", request.summary)
        case let .tool(tool) where tool.run?.state == .asking:
            kind = .permission
            body = L("Confirmation needed: %@", "$ \(tool.run?.firstLine ?? "")")
        case let .text(text):
            switch message.state {
            case let .failed(error):
                kind = .failure
                body = L("Reply failed: %@", error)
            case .complete where !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty:
                kind = .reply
                body = text
            default: return nil
            }
        default: return nil
        }
    }

    static func finishedTurn(in chat: Chat, botID: Bot.ID, startedAt: Date) -> ChatNotification? {
        // The last terminal reply wins, so a failure after partial output shows the error.
        for message in chat.messages.reversed() {
            guard message.author.botID == botID, message.createdAt >= startedAt.addingTimeInterval(-5),
                case .text = message.body
            else { continue }
            if let notification = ChatNotification(message) { return notification }
        }
        return nil
    }

    func canDeliver(in chat: Chat, watchedChat: Chat.ID?) -> Bool {
        guard watchedChat != chat.id, chat.unreadCount > 0,
            let message = chat.messages.first(where: { $0.id == messageID })
        else { return false }
        return ChatNotification(message) == self
    }
}
