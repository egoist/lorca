import Foundation

/// Wire shapes the CLI sends over 127.0.0.1. Keys are snake_case on the wire.
enum Wire {
    static let decoder: JSONDecoder = {
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return decoder
    }()

    struct Hello: Decodable {
        var version: String
        var hasIdentity: Bool
        var isIdentityDevice: Bool
        var deviceId: String?
        var relayUrl: String?
        var relayConnected: Bool
    }

    struct Snapshot: Decodable {
        var version: String
        var hasIdentity: Bool
        var isIdentityDevice: Bool
        var identityId: String?
        var thisDeviceId: String?
        var relayUrl: String?
        var relayConnected: Bool
        var devices: [Device]
        var bots: [Bot]
        var chats: [Chat]
        var runningChatIds: [String]
        var runningTurns: [RunningTurn]?
    }

    struct RunningTurn: Decodable {
        var jobId: String
        var chatId: String
        var botId: String
    }

    struct Provider: Decodable {
        var kind: String
        var isConnected: Bool
        var detail: String
    }

    struct Device: Decodable {
        var id: String
        var name: String
        var model: String
        var os: String
        var osVersion: String
        var machineKey: String
        var isThisDevice: Bool
        var status: String
        var lastSeen: Double
        var providers: [Provider]
    }

    struct Bot: Decodable {
        var id: String
        var name: String
        var tagline: String
        var symbolName: String
        var accent: String
        var runnerId: String
        var provider: String
        var model: String?
        var instructions: String
        var createdAt: Double
    }

    struct Chat: Decodable {
        var id: String
        var kind: String
        var title: String?
        var botIds: [String]
        var isPinned: Bool
        var createdAt: Double
        var messages: [Message]?
        var unreadCount: Int?
    }

    struct Author: Decodable {
        var kind: String
        var botId: String?
    }

    struct Attachment: Decodable {
        var id: String
        var name: String
        var mime: String
        var size: Int
        var width: Int?
        var height: Int?
    }

    struct FilePath: Decodable {
        var path: String
    }

    struct Body: Decodable {
        var kind: String
        var text: String?
        var attachments: [Attachment]?
        var name: String?
        var summary: String?
        var detail: String?
        var isRunning: Bool?
        var from: String?
        var to: String?
        var reason: String?
    }

    struct State: Decodable {
        var kind: String
        var error: String?
    }

    struct Message: Decodable {
        var id: String
        var chatId: String
        var author: Author
        var body: Body
        var state: State
        var createdAt: Double
    }

    struct RosterChanged: Decodable {
        var devices: [Device]
        var bots: [Bot]
        var chats: [Chat]
    }

    struct MessageEvent: Decodable {
        var chatId: String
        var message: Message
    }

    struct MessageRemoved: Decodable {
        var chatId: String
        var messageId: String
    }

    struct ChatRemoved: Decodable {
        var chatId: String
    }

    struct JobEvent: Decodable {
        var chatId: String
        var botId: String
        var jobId: String
    }

    struct RelayStatus: Decodable {
        var connected: Bool
        var url: String?
    }

    struct IdentityChanged: Decodable {
        var hasIdentity: Bool
    }

    struct PairStart: Decodable {
        var nonce: String
        var pairingString: String
    }

    struct PairStatus: Decodable {
        var state: String
        var error: String?
    }

    struct IdentityCreated: Decodable {
        var phrase: [String]
    }

    struct BotCreated: Decodable {
        var bot: Bot
        var chatId: String
    }

    struct ChatCreated: Decodable {
        var chat: Chat
    }

    struct Envelope<T: Decodable>: Decodable {
        var event: String
        var data: T
    }

    struct EventName: Decodable {
        var event: String
    }
}

// MARK: - Mapping into the app's models

extension Wire.Device {
    func toModel() -> Device {
        Device(
            id: id,
            name: name,
            model: model,
            os: Device.OS(rawValue: os) ?? .linux,
            osVersion: osVersion,
            isThisDevice: isThisDevice,
            status: status == "online" ? .online : (status == "pairing" ? .pairing : .offline),
            lastSeen: Date(timeIntervalSince1970: lastSeen),
            machineKey: machineKey,
            providers: providers.compactMap { provider in
                guard let kind = ProviderCredential.Kind(wireValue: provider.kind) else { return nil }
                return ProviderCredential(kind: kind, isConnected: provider.isConnected, detail: provider.detail)
            }
        )
    }
}

extension Wire.Bot {
    func toModel() -> Bot {
        Bot(
            id: id,
            name: name,
            tagline: tagline,
            symbolName: symbolName,
            accent: Accent(rawValue: accent) ?? .indigo,
            runnerID: runnerId,
            provider: ProviderCredential.Kind(wireValue: provider) ?? .deepseek,
            model: model,
            instructions: instructions,
            createdAt: Date(timeIntervalSince1970: createdAt)
        )
    }
}

extension Wire.Message {
    func toModel() -> Message {
        let author: Message.Author
        switch self.author.kind {
        case "you": author = .you
        case "bot": author = .bot(self.author.botId ?? "")
        default: author = .system
        }

        let body: Message.Body
        switch self.body.kind {
        case "tool":
            body = .tool(
                ToolInvocation(
                    name: self.body.name ?? "tool",
                    summary: self.body.summary ?? "",
                    detail: self.body.detail ?? "",
                    isRunning: self.body.isRunning ?? false
                ))
        case "handoff":
            body = .handoff(from: self.body.from ?? "", to: self.body.to ?? "", reason: self.body.reason ?? "")
        case "notice":
            body = .notice(self.body.text ?? "")
        default:
            body = .text(self.body.text ?? "")
        }

        let state: Message.State
        switch self.state.kind {
        case "thinking": state = .thinking
        case "streaming": state = .streaming
        case "failed": state = .failed(self.state.error ?? "Failed")
        default: state = .complete
        }

        return Message(
            id: id, author: author, body: body, state: state,
            createdAt: Date(timeIntervalSince1970: createdAt),
            attachments: (self.body.attachments ?? []).map {
                Attachment(id: $0.id, name: $0.name, mime: $0.mime, size: $0.size, width: $0.width, height: $0.height)
            })
    }
}

extension Wire.Chat {
    func toModel(existingMessages: [Message]? = nil, existingUnread: Int = 0) -> Chat {
        Chat(
            id: id,
            kind: kind == "dm" ? .dm : .group,
            customTitle: title,
            botIDs: botIds,
            messages: messages.map { $0.map { $0.toModel() } } ?? existingMessages ?? [],
            unreadCount: unreadCount ?? existingUnread,
            isPinned: isPinned,
            createdAt: Date(timeIntervalSince1970: createdAt)
        )
    }
}
