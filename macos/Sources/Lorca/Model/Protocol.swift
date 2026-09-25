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
        var relayUpdateRequired: Bool?
    }

    struct Snapshot: Decodable {
        var version: String
        var hasIdentity: Bool
        var isIdentityDevice: Bool
        var identityId: String?
        var thisDeviceId: String?
        var relayUrl: String?
        var relayConnected: Bool
        var relayUpdateRequired: Bool?
        var devices: [Device]
        var bots: [Bot]
        var chats: [Chat]
        var routines: [Routine]?
        var autoReview: AutoReview?
        var providers: [Provider]?
        var runningChatIds: [String]
        var runningTurns: [RunningTurn]?
    }

    struct AutoReviewRule: Decodable {
        var id: String
        var text: String
        var behavior: String
        var tool: String?

        func toModel() -> Lorca.AutoReviewRule {
            Lorca.AutoReviewRule(id: id, text: text, behavior: behavior == "ask" ? .ask : .allow, tool: tool)
        }
    }

    struct AutoReview: Decodable {
        var isEnabled: Bool
        var rules: [AutoReviewRule]?

        func toModel() -> Lorca.AutoReview {
            Lorca.AutoReview(isEnabled: isEnabled, rules: (rules ?? []).map { $0.toModel() })
        }
    }

    struct RunningTurn: Decodable {
        var jobId: String
        var chatId: String
        var botId: String
        var routineId: String?
    }

    struct Routine: Decodable {
        var id: String
        var botId: String
        var name: String
        var prompt: String
        var schedule: String
        var scheduleText: String?
        var isEnabled: Bool
        var pausedReason: String?
        var lastRunAt: Double?
        var lastOutcome: String?
        var nextRunAt: Double?
        var isRunning: Bool?
        var createdAt: Double

        func toModel() -> Lorca.Routine {
            Lorca.Routine(
                id: id, botID: botId, name: name, prompt: prompt, schedule: schedule, scheduleText: Format.schedule(scheduleText ?? schedule),
                isEnabled: isEnabled, pausedReason: pausedReason, lastRunAt: lastRunAt.map { Date(timeIntervalSince1970: $0) },
                lastOutcome: lastOutcome, nextRunAt: nextRunAt.map { Date(timeIntervalSince1970: $0) }, isRunning: isRunning ?? false,
                createdAt: Date(timeIntervalSince1970: createdAt))
        }
    }

    struct RoutineChanged: Decodable {
        var routine: Routine
    }

    /// Fetched only while editing a provider; never stored in the account snapshot.
    struct ProviderAPIKey: Decodable {
        var apiKey: String?
        var baseUrl: String?
    }

    struct Provider: Decodable {
        var kind: String
        var isConnected: Bool
        var detail: String
        var baseUrl: String?

        func toModel() -> ProviderCredential? {
            guard let kind = ProviderCredential.Kind(wireValue: kind) else { return nil }
            return ProviderCredential(kind: kind, isConnected: isConnected, detail: detail, baseURL: baseUrl)
        }
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
        var plugins: [PluginStatus]?
    }

    struct PluginStatus: Decodable {
        var id: String
        var name: String
        var description: String?
        var version: String?
        var icon: String?
        var state: String
        var detail: String?

        func toModel() -> InstalledPlugin {
            InstalledPlugin(
                id: id, name: name, description: description ?? "", version: version ?? "", icon: icon ?? "",
                state: InstalledPlugin.State(rawValue: state) ?? .unknown, detail: detail ?? "")
        }
    }

    struct MarketplacePlugin: Decodable {
        struct Server: Decodable {
            struct Auth: Decodable { var type: String }
            var type: String
            var url: String?
            var command: String?
            var args: [String]?
            var auth: Auth?
        }
        struct Variable: Decodable {
            var name: String
            var description: String?
            var secret: Bool?
            var required: Bool?
        }
        struct Skill: Decodable {
            var name: String
            var description: String?
        }
        var id: String
        var name: String
        var description: String?
        var icon: String?
        var homepage: String?
        var author: String?
        var category: String?
        var featured: Bool?
        var tags: [String]?
        var servers: [String: Server]?
        var variables: [Variable]?
        var skills: [Skill]?
        var installedOn: [String]?

        func toModel() -> Lorca.MarketplacePlugin {
            let servers = (servers ?? [:]).sorted { $0.key < $1.key }.map { name, server in
                Lorca.MarketplacePlugin.Server(
                    name: name,
                    address: server.url ?? ([server.command ?? ""] + (server.args ?? [])).joined(separator: " "),
                    isRemote: server.type == "http", signsIn: server.auth?.type == "oauth")
            }
            return Lorca.MarketplacePlugin(
                id: id, name: name, description: description ?? "", icon: icon ?? "", homepage: homepage,
                author: author ?? "", category: category ?? "", isFeatured: featured ?? false, tags: tags ?? [],
                servers: servers,
                skills: (skills ?? []).map { .init(name: $0.name, description: $0.description ?? "") },
                variables: (variables ?? []).map {
                    .init(name: $0.name, description: $0.description ?? "", secret: $0.secret ?? false, required: $0.required ?? false)
                },
                installedOn: installedOn ?? [])
        }
    }

    struct BotTemplate: Decodable {
        struct Routine: Decodable {
            var name: String
            var schedule: String
            var scheduleText: String?
            var prompt: String
        }
        var id: String
        var name: String
        var summary: String?
        var description: String
        var symbolName: String?
        var accent: String?
        var category: String?
        var featured: Bool?
        var author: String?
        var plugins: [String]?
        var routines: [Routine]?
        var memory: [String]?

        func toModel() -> Lorca.BotTemplate {
            Lorca.BotTemplate(
                id: id, name: name, summary: summary ?? "", description: description,
                symbolName: symbolName ?? "sparkles", accent: Accent(rawValue: accent ?? "") ?? .indigo,
                category: category ?? "", isFeatured: featured ?? false, author: author ?? "", plugins: plugins ?? [],
                routines: (routines ?? []).map { .init(name: $0.name, scheduleText: $0.scheduleText ?? $0.schedule, prompt: $0.prompt) },
                memory: memory ?? [])
        }
    }

    struct Marketplace: Decodable {
        var plugins: [MarketplacePlugin]
        var bots: [BotTemplate]
    }

    struct PluginInstalled: Decodable {
        var status: PluginStatus
    }

    struct PluginConnected: Decodable {
        var message: String
    }

    struct PluginDetail: Decodable {
        struct Manifest: Decodable { var homepage: String? }
        struct Variable: Decodable {
            var name: String
            var description: String?
            var secret: Bool
            var required: Bool
            var isSet: Bool
            var value: String?
        }
        struct Server: Decodable {
            struct Auth: Decodable {
                var url: String?
                var oauth: Bool?
                var signedIn: Bool?
            }
            var name: String
            var kind: String
            var auth: Auth
        }
        struct Skill: Decodable {
            var name: String
            var description: String?
        }
        var manifest: Manifest
        var status: PluginStatus
        var variables: [Variable]
        var servers: [Server]
        var skills: [Skill]

        func toModel() -> Lorca.PluginDetail {
            Lorca.PluginDetail(
                status: status.toModel(), homepage: manifest.homepage,
                variables: variables.map { .init(name: $0.name, description: $0.description ?? "", secret: $0.secret, required: $0.required, isSet: $0.isSet, value: $0.value) },
                servers: servers.map { .init(name: $0.name, kind: $0.kind, url: $0.auth.url, oauth: $0.auth.oauth ?? false, signedIn: $0.auth.signedIn ?? false) },
                skills: skills.map { (name: $0.name, description: $0.description ?? "") })
        }
    }

    struct Bot: Decodable {
        var id: String
        var name: String
        var description: String
        var symbolName: String
        var accent: String
        var runnerId: String
        var provider: String
        var model: String?
        var thinking: String?
        var avatar: Attachment?
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
        var usage: ChatUsage?
        var hasMore: Bool?
    }

    struct MessagePage: Decodable {
        var messages: [Message]
        var hasMore: Bool
    }

    struct SearchResults: Decodable {
        struct ChatHit: Decodable {
            var chatId: String
            var snippet: String
        }

        struct MessageHit: Decodable {
            var chatId: String
            var messageId: String
            var snippet: String
            var author: Author
            var createdAt: Double
        }

        var chats: [ChatHit]
        var messages: [MessageHit]

        static let empty = SearchResults(chats: [], messages: [])
    }

    struct ChatUsage: Decodable {
        var contextTokens: Int
        var contextWindow: Int
        var inputTokens: Int
        var outputTokens: Int
        var cacheReadTokens: Int
        var costUsd: Double
        var turns: Int
        var model: String
    }

    struct BotMemory: Decodable {
        struct Index: Decodable {
            var text: String
            var hash: String
            var lines: Int
            var bytes: Int
            var truncated: Bool
            var maxLines: Int
            var maxBytes: Int
        }
        var botId: String
        var here: Bool
        var runner: String
        var path: String?
        var index: Index?
        var topics: [String]?
        var logs: [String]?

        func toModel() -> Lorca.BotMemory {
            Lorca.BotMemory(
                botID: botId, here: here, runner: runner, path: path ?? "",
                text: index?.text ?? "", hash: index?.hash ?? "", lines: index?.lines ?? 0, bytes: index?.bytes ?? 0,
                truncated: index?.truncated ?? false, maxLines: index?.maxLines ?? 200, maxBytes: index?.maxBytes ?? 24_000,
                topics: topics ?? [], logs: logs ?? [])
        }
    }

    struct MemoryWritten: Decodable {
        var hash: String
    }

    struct JobRetry: Decodable {
        var chatId: String
        var botId: String
        var attempt: Int
        var maxAttempts: Int
        var delayMs: Int
        var error: String
    }

    struct JobThinking: Decodable {
        var chatId: String
        var botId: String
    }

    struct ChatUsageEvent: Decodable {
        var chatId: String
        var usage: ChatUsage
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
        var description: String?
        var targetBotId: String?
        var from: String?
        var to: String?
        var reason: String?
        var pluginId: String?
        var pluginName: String?
        var tool: String?
        var decision: String?
        var link: String?
        var code: String?
        var rule: String?
        var command: String?
        var run: Run?
    }

    struct Run: Decodable {
        var sessionId: String?
        var command: String?
        var state: String
        var prompt: String?
        var output: String?
        var outcome: String?
        var device: String?
        var reason: String?
        var rule: String?
        var decision: String?
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
        var routines: [Routine]?
        var autoReview: AutoReview?
        var providers: [Provider]?
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
        var routineId: String?
    }

    struct RelayStatus: Decodable {
        var connected: Bool
        var url: String?
        var updateRequired: Bool?
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
        /// `completed`: the Device that joined.
        var device: PairedDevice?
    }

    struct PairedDevice: Decodable {
        var id: String
        var name: String
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
            plugins: (plugins ?? []).map { $0.toModel() }
        )
    }
}

extension Wire.Bot {
    func toModel() -> Bot {
        Bot(
            id: id,
            name: name,
            description: description,
            symbolName: symbolName,
            accent: Accent(rawValue: accent) ?? .indigo,
            runnerID: runnerId,
            provider: ProviderCredential.Kind(wireValue: provider) ?? .deepseek,
            model: model,
            thinking: thinking,
            avatar: avatar.map { Attachment(id: $0.id, name: $0.name, mime: $0.mime, size: $0.size, width: $0.width, height: $0.height) },
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
                    isRunning: self.body.isRunning ?? false,
                    description: self.body.description,
                    targetBotID: self.body.targetBotId,
                    run: self.body.run.map {
                        CommandRun(
                            sessionID: $0.sessionId, command: $0.command ?? "",
                            state: CommandRun.State(rawValue: $0.state) ?? .stopped,
                            prompt: $0.prompt, output: $0.output, outcome: $0.outcome,
                            device: $0.device, reason: $0.reason, rule: $0.rule, decision: $0.decision)
                    }
                ))
        case "handoff":
            body = .handoff(from: self.body.from ?? "", to: self.body.to ?? "", reason: self.body.reason ?? "")
        case "notice":
            body = .notice(self.body.text ?? "")
        case "permission":
            body = .permission(
                PermissionRequest(
                    pluginID: self.body.pluginId ?? "", pluginName: self.body.pluginName ?? "", tool: self.body.tool ?? "",
                    summary: self.body.summary ?? "", decision: PermissionRequest.Decision(rawValue: self.body.decision ?? "") ?? .pending,
                    link: self.body.link, code: self.body.code, reason: self.body.reason, rule: self.body.rule,
                    command: self.body.command))
        default:
            body = .text(self.body.text ?? "")
        }

        let state: Message.State
        switch self.state.kind {
        case "thinking": state = .thinking
        case "streaming": state = .streaming
        case "failed": state = .failed(self.state.error ?? L("Failed"))
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
    func toModel(existingMessages: [Message]? = nil, existingUnread: Int = 0, existingHasMore: Bool = false) -> Chat {
        let modelKind: Chat.Kind = kind == "dm" ? .dm : .group
        return Chat(
            id: id,
            kind: modelKind,
            customTitle: modelKind == .group ? title : nil,
            botIDs: botIds,
            messages: messages.map { $0.map { $0.toModel() } } ?? existingMessages ?? [],
            unreadCount: unreadCount ?? existingUnread,
            isPinned: isPinned,
            createdAt: Date(timeIntervalSince1970: createdAt),
            usage: usage?.toModel(),
            hasMore: hasMore ?? existingHasMore
        )
    }
}

extension Wire.ChatUsage {
    func toModel() -> ChatUsage {
        ChatUsage(
            contextTokens: contextTokens, contextWindow: contextWindow, inputTokens: inputTokens, outputTokens: outputTokens,
            cacheReadTokens: cacheReadTokens, costUSD: costUsd, turns: turns, model: model)
    }
}
