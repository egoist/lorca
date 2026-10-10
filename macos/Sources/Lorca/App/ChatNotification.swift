import Foundation

/// The message behind an alert, kept so a delayed delivery can check its current state.
struct ChatNotification: Equatable {
    enum Kind { case reply, permission, failure, summary, urgent }

    let kind: Kind
    let messageID: Message.ID
    let botID: Bot.ID
    let body: String

    init?(_ message: Message) {
        guard message.notification != "quiet" else { return nil }
        guard let botID = message.author.botID else { return nil }
        self.botID = botID
        messageID = message.id
        switch message.body {
        // An access request is the bot's to explain in its reply, which notifies on its own.
        case let .permission(request) where request.isPending && request.isSecret:
            kind = .permission
            body = L("Asks for %@", request.summary)
        case let .permission(request) where request.isPending && !request.isAccess:
            kind = .permission
            body = L("Confirmation needed: %@", request.summary)
        case let .tool(tool) where tool.run?.state == .asking:
            kind = .permission
            body = L("Confirmation needed: %@", "$ \(tool.run?.firstLine ?? "")")
        case let .tool(tool) where tool.agent?.state == .asking:
            guard let agent = tool.agent, let question = agent.question else { return nil }
            kind = .permission
            switch question.kind {
            case .start: body = L("Confirmation needed: %@", L("Start %@", agent.name))
            case .command:
                let line = question.command?.split(separator: "\n").first.map(String.init) ?? ""
                body = L("Confirmation needed: %@", "\(agent.name): $ \(line)")
            case .choices, .text:
                let line = question.text.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.last { !$0.isEmpty } ?? ""
                body = "\(L("%@ asks", agent.name)): \(line)"
            }
        case let .text(text):
            switch message.state {
            case let .failed(error):
                kind = .failure
                body = L("Reply failed: %@", error)
            case .complete where !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty:
                kind = message.notification == "summary" ? .summary : message.notification == "urgent" ? .urgent : .reply
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
            // Structured attention messages alert on arrival, without a second turn alert.
            if message.notification != nil { continue }
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
