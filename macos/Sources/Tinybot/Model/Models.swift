import Foundation

// MARK: - Computer

struct ProviderCredential: Hashable, Identifiable {
    enum Kind: String, Hashable, CaseIterable {
        case deepseek = "DeepSeek"
        case chatgpt = "ChatGPT"

        var symbolName: String {
            switch self {
            case .deepseek: "key.fill"
            case .chatgpt: "person.badge.key.fill"
            }
        }

        var subtitle: String {
            switch self {
            case .deepseek: "API key"
            case .chatgpt: "Subscription"
            }
        }
    }

    var id: Kind { kind }
    var kind: Kind
    var isConnected: Bool
    var detail: String
}

struct Computer: Identifiable, Hashable {
    enum Status: Hashable {
        case online
        case offline
        case pairing

        var label: String {
            switch self {
            case .online: "Online"
            case .offline: "Offline"
            case .pairing: "Pairing"
            }
        }
    }

    let id: String
    var name: String
    var model: String
    var osVersion: String
    var isThisComputer: Bool
    var status: Status
    var lastSeen: Date
    var machineKey: String
    var providers: [ProviderCredential]

    var connectedProviders: [ProviderCredential] {
        providers.filter(\.isConnected)
    }

    func credential(for kind: ProviderCredential.Kind) -> ProviderCredential? {
        providers.first { $0.kind == kind }
    }
}

// MARK: - Bot

struct Bot: Identifiable, Hashable {
    let id: String
    var name: String
    var tagline: String
    var symbolName: String
    var accent: Accent
    var computerID: Computer.ID
    var provider: ProviderCredential.Kind
    var instructions: String
    var createdAt: Date
}

// MARK: - Message

struct ToolInvocation: Hashable {
    var name: String
    var summary: String
    var detail: String
    var isRunning: Bool

    var symbolName: String {
        switch name {
        case "message_bot": "arrow.triangle.turn.up.right.diamond.fill"
        case "list_teammates": "person.2.fill"
        default: "wrench.and.screwdriver.fill"
        }
    }
}

struct Message: Identifiable, Hashable {
    enum Author: Hashable {
        case you
        case bot(Bot.ID)
        case system

        var botID: Bot.ID? {
            if case let .bot(id) = self { return id }
            return nil
        }

        var isYou: Bool { self == .you }
    }

    enum Body: Hashable {
        case text(String)
        case tool(ToolInvocation)
        case handoff(from: Bot.ID, to: Bot.ID, reason: String)
        case notice(String)
    }

    enum State: Hashable {
        case thinking
        case streaming
        case complete
        case failed(String)
    }

    let id: UUID
    var author: Author
    var body: Body
    var state: State
    var createdAt: Date

    init(
        id: UUID = UUID(),
        author: Author,
        body: Body,
        state: State = .complete,
        createdAt: Date = Date()
    ) {
        self.id = id
        self.author = author
        self.body = body
        self.state = state
        self.createdAt = createdAt
    }

    var text: String {
        switch body {
        case let .text(value): value
        case let .tool(tool): tool.summary
        case let .handoff(_, _, reason): reason
        case let .notice(value): value
        }
    }

    var isTranscriptText: Bool {
        if case .text = body { return true }
        return false
    }
}

// MARK: - Chat

struct Chat: Identifiable, Hashable {
    let id: String
    var customTitle: String?
    var botIDs: [Bot.ID]
    var messages: [Message]
    var unreadCount: Int
    var isPinned: Bool
    var createdAt: Date

    var isGroup: Bool { botIDs.count > 1 }

    var lastActivity: Date {
        messages.last?.createdAt ?? createdAt
    }

    func index(of messageID: Message.ID) -> Int? {
        messages.firstIndex { $0.id == messageID }
    }
}
