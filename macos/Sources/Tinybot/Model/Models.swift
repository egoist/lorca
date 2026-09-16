import Foundation

// MARK: - Device

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

        /// The identifier the CLI uses on the wire.
        var wireValue: String {
            switch self {
            case .deepseek: "deepseek"
            case .chatgpt: "chatgpt"
            }
        }

        init?(wireValue: String) {
            switch wireValue {
            case "deepseek": self = .deepseek
            case "chatgpt": self = .chatgpt
            default: return nil
            }
        }

        /// Model ids this provider accepts, first is the default the CLI uses.
        var models: [(id: String, label: String)] {
            switch self {
            case .deepseek:
                [
                    ("deepseek-flash", "V4.1 Flash"),
                    ("deepseek-v4-pro", "V4 Pro (reasoning)"),
                ]
            case .chatgpt:
                [
                    ("gpt-5.6-terra", "GPT-5.6 Terra"),
                    ("gpt-6-astra", "GPT-6 Astra"),
                    ("gpt-5.6-sol", "GPT-5.6 Sol"),
                    ("gpt-5.6-luna", "GPT-5.6 Luna"),
                    ("gpt-5.5", "GPT-5.5"),
                    ("gpt-5.3-codex-spark", "Codex Spark (Pro)"),
                ]
            }
        }
    }

    var id: Kind { kind }
    var kind: Kind
    var isConnected: Bool
    var detail: String
}

/// A paired machine or phone. Its `os` decides whether it is a Runner: only desktop
/// systems run the CLI, hold provider credentials, and get bots assigned.
struct Device: Identifiable, Hashable {
    enum OS: String, Hashable, CaseIterable {
        case macos
        case linux
        case windows
        case ios
        case ipados
        case android

        var displayName: String {
            switch self {
            case .macos: "macOS"
            case .linux: "Linux"
            case .windows: "Windows"
            case .ios: "iOS"
            case .ipados: "iPadOS"
            case .android: "Android"
            }
        }

        /// Desktop systems run the agent loop. Phones and tablets never do.
        var isDesktop: Bool {
            switch self {
            case .macos, .linux, .windows: true
            case .ios, .ipados, .android: false
            }
        }
    }

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
    var os: OS
    var osVersion: String
    var isThisDevice: Bool
    var status: Status
    var lastSeen: Date
    var machineKey: String
    var providers: [ProviderCredential]

    /// Derived from `os` alone: a desktop Device is a Runner and can be assigned bots.
    var isRunner: Bool { os.isDesktop }

    var roleLabel: String { isRunner ? "Runner" : "Device" }

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
    /// One short line under the name: what the bot is for.
    var label: String
    /// A sentence or two about the bot.
    var description: String
    var symbolName: String
    var accent: Accent
    var runnerID: Device.ID
    var provider: ProviderCredential.Kind
    /// nil means the provider's default model.
    var model: String? = nil
    var instructions: String
    var createdAt: Date
}

// MARK: - Message

struct ToolInvocation: Hashable {
    var name: String
    var summary: String
    var detail: String
    var isRunning: Bool

    /// A finished message_bot call: the one tool the transcript shows, as "Messaged ◉ Name".
    /// Everything else a bot does with tools stays behind the "is working" row.
    var isSentMessage: Bool {
        name == "message_bot" && !isRunning && summary.hasPrefix("Messaged ")
    }

    /// The recipient's name from a "Messaged Name" summary.
    var recipientName: String {
        String(summary.dropFirst("Messaged ".count))
    }

    var symbolName: String {
        switch name {
        case "message_bot": "arrow.triangle.turn.up.right.diamond.fill"
        case "list_teammates": "person.2.fill"
        case "create_bot": "person.badge.plus"
        case "edit_bot": "person.text.rectangle"
        case "read": "doc.text"
        case "write": "square.and.pencil"
        case "edit": "pencil.line"
        case "bash": "terminal"
        case "grep": "text.magnifyingglass"
        case "find": "magnifyingglass"
        case "ls": "folder"
        default: "wrench.and.screwdriver.fill"
        }
    }
}

/// A file sent with a message. The bytes live under `~/.tinybot/files/<id>` once this Device
/// has them; `width` and `height` size an image's thumbnail before the file arrives.
struct Attachment: Hashable, Identifiable {
    let id: String
    var name: String
    var mime: String
    var size: Int
    var width: Int?
    var height: Int?

    var isImage: Bool { mime.hasPrefix("image/") }

    /// "Photo", "3 photos", "report.pdf", "2 files": the preview of a message with no text.
    static func summary(_ attachments: [Attachment]) -> String {
        guard let first = attachments.first else { return "" }
        if attachments.count == 1 { return first.isImage ? "Photo" : first.name }
        return attachments.allSatisfy(\.isImage) ? "\(attachments.count) photos" : "\(attachments.count) files"
    }

    static func sizeText(_ bytes: Int) -> String {
        if bytes < 1024 { return "\(bytes) B" }
        if bytes < 1024 * 1024 { return "\(Int((Double(bytes) / 1024).rounded())) KB" }
        return String(format: "%.1f MB", Double(bytes) / 1024 / 1024)
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

    let id: String
    var author: Author
    var body: Body
    var state: State
    var createdAt: Date
    /// Files sent with a text body; other bodies carry none.
    var attachments: [Attachment]

    init(
        id: String = "msg-\(UUID().uuidString.lowercased())",
        author: Author,
        body: Body,
        state: State = .complete,
        createdAt: Date = Date(),
        attachments: [Attachment] = []
    ) {
        self.id = id
        self.author = author
        self.body = body
        self.state = state
        self.createdAt = createdAt
        self.attachments = attachments
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
    /// A DM is a fixed one-to-one thread with a single bot. A group holds one to six bots and
    /// can gain or lose members after it is created.
    enum Kind: String, Hashable {
        case dm
        case group
    }

    static let maxGroupBots = 6

    let id: String
    var kind: Kind
    var customTitle: String?
    var botIDs: [Bot.ID]
    var messages: [Message]
    var unreadCount: Int
    var isPinned: Bool
    var createdAt: Date

    var isGroup: Bool { kind == .group }
    var isDM: Bool { kind == .dm }

    /// Whether another bot may join. Only groups grow, and never past the cap.
    var canAddBot: Bool { isGroup && botIDs.count < Self.maxGroupBots }

    /// Whether a bot may leave. Groups keep at least one bot; DMs never change.
    var canRemoveBot: Bool { isGroup && botIDs.count > 1 }

    var lastActivity: Date {
        messages.last?.createdAt ?? createdAt
    }

    func index(of messageID: Message.ID) -> Int? {
        messages.firstIndex { $0.id == messageID }
    }
}
