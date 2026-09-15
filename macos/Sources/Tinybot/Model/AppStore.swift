import AppKit
import Foundation

enum StoreEvent {
    case snapshotReplaced
    case rosterChanged
    case chatsChanged
    case chatChanged(Chat.ID)
    case messageAdded(Chat.ID, Message.ID)
    case messageChanged(Chat.ID, Message.ID)
    case selectionChanged
    case connectionChanged
}

enum Selection: Hashable {
    case chat(Chat.ID)
    case computer(Computer.ID)
}

@MainActor
final class AppStore {
    static let shared = AppStore()

    private(set) var computers: [Computer] = MockData.computers()
    private(set) var bots: [Bot] = MockData.bots()
    private(set) var chats: [Chat] = MockData.chats()

    private(set) var isConnected = true
    private var replyEngine: ReplyEngine?

    private struct Subscription {
        weak var owner: AnyObject?
        let handler: (StoreEvent) -> Void
    }

    private var subscriptions: [Subscription] = []

    private init() {
        replyEngine = ReplyEngine(store: self)
        sortChats()
    }

    // MARK: - Observation

    func observe(_ owner: AnyObject, _ handler: @escaping (StoreEvent) -> Void) {
        subscriptions.append(Subscription(owner: owner, handler: handler))
    }

    private func emit(_ event: StoreEvent) {
        subscriptions.removeAll { $0.owner == nil }
        for subscription in subscriptions {
            subscription.handler(event)
        }
    }

    // MARK: - Lookup

    func bot(_ id: Bot.ID) -> Bot? {
        bots.first { $0.id == id }
    }

    func computer(_ id: Computer.ID) -> Computer? {
        computers.first { $0.id == id }
    }

    func chat(_ id: Chat.ID) -> Chat? {
        chats.first { $0.id == id }
    }

    func bots(in chat: Chat) -> [Bot] {
        chat.botIDs.compactMap(bot)
    }

    func bots(on computerID: Computer.ID) -> [Bot] {
        bots.filter { $0.computerID == computerID }
    }

    var thisComputer: Computer? {
        computers.first(where: \.isThisComputer)
    }

    func title(for chat: Chat) -> String {
        if let custom = chat.customTitle, !custom.isEmpty { return custom }
        let names = chat.botIDs.compactMap { bot($0)?.name }
        return names.isEmpty ? "New Chat" : names.joined(separator: ", ")
    }

    func subtitle(for chat: Chat) -> String {
        let members = bots(in: chat)
        if chat.isDM, let only = members.first {
            let host = computer(only.computerID)?.name ?? "unassigned"
            return "\(only.provider.rawValue) on \(host)"
        }
        let hosts = Set(members.compactMap { computer($0.computerID)?.name })
        let computerLabel = hosts.count == 1 ? (hosts.first ?? "") : "\(hosts.count) Computers"
        let botLabel = members.count == 1 ? "1 bot" : "\(members.count) bots"
        return "Group · \(botLabel) · \(computerLabel)"
    }

    func preview(for chat: Chat) -> String {
        guard let last = chat.messages.last else { return "No messages yet" }
        let body: String
        switch last.body {
        case let .text(value): body = value
        case let .tool(tool): body = tool.summary
        case let .handoff(_, to, _): body = "Handed off to \(bot(to)?.name ?? "a teammate")"
        case let .notice(value): body = value
        }

        let flattened = body
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "**", with: "")
            .replacingOccurrences(of: "`", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)

        switch last.author {
        case .you: return "You: \(flattened)"
        case let .bot(id): return chat.isGroup ? "\(bot(id)?.name ?? ""): \(flattened)" : flattened
        case .system: return flattened
        }
    }

    // MARK: - Connection

    func setConnected(_ connected: Bool) {
        guard isConnected != connected else { return }
        isConnected = connected
        emit(.connectionChanged)
    }

    // MARK: - Chat mutation

    private func sortChats() {
        chats.sort { lhs, rhs in
            if lhs.isPinned != rhs.isPinned { return lhs.isPinned }
            return lhs.lastActivity > rhs.lastActivity
        }
    }

    /// The one DM with this bot, created on first use. DMs are keyed by the bot, so opening one
    /// twice lands in the same thread.
    @discardableResult
    func dm(with botID: Bot.ID) -> Chat.ID {
        if let existing = chats.first(where: { $0.isDM && $0.botIDs == [botID] }) {
            return existing.id
        }
        return createChat(kind: .dm, with: [botID], title: nil)
    }

    @discardableResult
    func createChat(kind: Chat.Kind, with botIDs: [Bot.ID], title: String?) -> Chat.ID {
        let botIDs: [Bot.ID] = kind == .dm ? Array(botIDs.prefix(1)) : Array(botIDs.prefix(Chat.maxGroupBots))
        let chat = Chat(
            id: "chat-\(UUID().uuidString.prefix(8))",
            kind: kind,
            customTitle: kind == .group ? title : nil,
            botIDs: botIDs,
            messages: [],
            unreadCount: 0,
            isPinned: false,
            createdAt: Date()
        )
        chats.insert(chat, at: 0)
        sortChats()
        emit(.chatsChanged)
        return chat.id
    }

    @discardableResult
    func createBot(
        name: String,
        tagline: String,
        symbolName: String,
        accent: Accent,
        computerID: Computer.ID,
        provider: ProviderCredential.Kind
    ) -> Bot.ID {
        let bot = Bot(
            id: "bot-\(UUID().uuidString.prefix(8))",
            name: name,
            tagline: tagline,
            symbolName: symbolName,
            accent: accent,
            computerID: computerID,
            provider: provider,
            instructions: "",
            createdAt: Date()
        )
        bots.append(bot)
        emit(.rosterChanged)
        emit(.chatsChanged)
        return bot.id
    }

    func deleteChat(_ id: Chat.ID) {
        replyEngine?.cancel(chatID: id)
        chats.removeAll { $0.id == id }
        emit(.chatsChanged)
    }

    func togglePin(_ id: Chat.ID) {
        guard let index = chats.firstIndex(where: { $0.id == id }) else { return }
        chats[index].isPinned.toggle()
        sortChats()
        emit(.chatsChanged)
    }

    func markRead(_ id: Chat.ID) {
        guard let index = chats.firstIndex(where: { $0.id == id }), chats[index].unreadCount > 0
        else { return }
        chats[index].unreadCount = 0
        emit(.chatsChanged)
    }

    func rename(_ id: Chat.ID, to title: String) {
        guard let index = chats.firstIndex(where: { $0.id == id }) else { return }
        chats[index].customTitle = title.trimmingCharacters(in: .whitespacesAndNewlines)
        emit(.chatChanged(id))
        emit(.chatsChanged)
    }

    func addBot(_ botID: Bot.ID, to chatID: Chat.ID) {
        guard let index = chats.firstIndex(where: { $0.id == chatID }),
            chats[index].canAddBot,
            !chats[index].botIDs.contains(botID)
        else { return }
        chats[index].botIDs.append(botID)
        let name = bot(botID)?.name ?? "A bot"
        append(
            Message(author: .system, body: .notice("\(name) joined the chat.")),
            to: chatID
        )
        emit(.chatChanged(chatID))
        emit(.chatsChanged)
    }

    func removeBot(_ botID: Bot.ID, from chatID: Chat.ID) {
        guard let index = chats.firstIndex(where: { $0.id == chatID }),
            chats[index].canRemoveBot
        else { return }
        chats[index].botIDs.removeAll { $0 == botID }
        emit(.chatChanged(chatID))
        emit(.chatsChanged)
    }

    // MARK: - Messages

    @discardableResult
    func append(_ message: Message, to chatID: Chat.ID) -> Message.ID? {
        guard let index = chats.firstIndex(where: { $0.id == chatID }) else { return nil }
        chats[index].messages.append(message)
        emit(.messageAdded(chatID, message.id))
        sortChats()
        emit(.chatsChanged)
        return message.id
    }

    /// Called for every streamed token, so this stays off the chat-list path.
    /// The sidebar preview refreshes when a step finishes instead.
    func update(_ messageID: Message.ID, in chatID: Chat.ID, _ transform: (inout Message) -> Void) {
        guard let chatIndex = chats.firstIndex(where: { $0.id == chatID }),
            let messageIndex = chats[chatIndex].index(of: messageID)
        else { return }
        transform(&chats[chatIndex].messages[messageIndex])
        emit(.messageChanged(chatID, messageID))
    }

    func refreshChatList() {
        emit(.chatsChanged)
    }

    func send(_ text: String, in chatID: Chat.ID) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, let chat = chat(chatID) else { return }

        append(Message(author: .you, body: .text(trimmed)), to: chatID)
        replyEngine?.respond(to: trimmed, in: chat)
    }

    func isResponding(in chatID: Chat.ID) -> Bool {
        replyEngine?.isRunning(chatID: chatID) ?? false
    }

    func stopResponding(in chatID: Chat.ID) {
        replyEngine?.cancel(chatID: chatID)
    }

    /// Replays the seeded conversations so the demo can be restarted from the Debug menu.
    func resetMockData() {
        for chat in chats { replyEngine?.cancel(chatID: chat.id) }
        computers = MockData.computers()
        bots = MockData.bots()
        chats = MockData.chats()
        sortChats()
        emit(.snapshotReplaced)
    }
}
