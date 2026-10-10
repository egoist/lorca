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
        var relayError: RelayProblem?
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
        var relayError: RelayProblem?
        var devices: [Device]
        var bots: [Bot]
        var chats: [Chat]
        var routines: [Routine]?
        var reviews: [ReviewItem]?
        var tasks: [DurableTask]?
        var playbooks: [PlaybookSummary]?
        var autoReview: AutoReview?
        var sharedLinks: [SharedLink]?
        var providers: [Provider]?
        var models: [Model]?
        var runningChatIds: [String]
        var runningTurns: [RunningTurn]?
        var attention: AttentionView?
        var budgets: [BudgetState]?
    }

    struct BudgetsChanged: Decodable { var budgets: [BudgetState] }

    /// `connector_limits.get`: an installed plugin account's shared call limit on its Runner.
    struct CallLimits: Decodable {
        struct Limits: Decodable {
            var maxCalls: Int
            var windowSecs: Int
            var maxConcurrency: Int
        }
        var limits: Limits
        var retryAt: Double?
        /// The service the account belongs to; another account of it shares the service's limit.
        var serviceId: String
        var pluginId: String
    }

    /// A model the CLI's catalog offers, in the catalog's order.
    struct Model: Decodable {
        var provider: String
        var id: String
        var name: String
        var levels: [String]
        var decides: Bool?

        func toModel() -> ProviderModel? {
            guard let kind = ProviderCredential.Kind(wireValue: provider) else { return nil }
            return ProviderModel(provider: kind, id: id, label: name, levels: levels, decides: decides ?? false)
        }
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
        var provider: String?
        var models: [String: String]?

        func toModel() -> Lorca.AutoReview {
            let models = (models ?? [:]).compactMap { kind, model in ProviderCredential.Kind(wireValue: kind).map { ($0, model) } }
            return Lorca.AutoReview(
                isEnabled: isEnabled, rules: (rules ?? []).map { $0.toModel() },
                provider: provider.flatMap(ProviderCredential.Kind.init(wireValue:)),
                models: Dictionary(models, uniquingKeysWith: { first, _ in first }))
        }
    }

    struct RunningTurn: Decodable {
        var jobId: String
        var chatId: String
        var botId: String
        var routineId: String?
    }

    struct Routine: Decodable {
        struct Health: Decodable {
            struct Model: Decodable {
                var status: String?
                var authenticationFailures: Int?
            }
            var lastCheckAt: Double?
            var lastSuccessAt: Double?
            var status: String?
            var connectionFailures: Int?
            var authenticationFailures: Int?
            var model: Model?
        }
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
        var check: String?
        var createdAt: Double
        var timezone: String?
        var missedRunPolicy: String?
        var state: String?
        var health: Health?
        var onceAt: Double?
        var pullRequest: PullRequest?
        var calendar: Calendar?

        struct PullRequest: Decodable {
            var repo: String
            var number: Int
            var title: String?
            var url: String?
        }

        struct Calendar: Decodable {
            struct Event: Decodable { var title: String? }
            var account: String?
            var matching: String?
            var minutes: Int?
            var after: Bool?
            var nextEvent: Event?
        }

        func toModel() -> Lorca.Routine {
            let watch = pullRequest.map { RoutineWatch(repo: $0.repo, number: $0.number, title: $0.title ?? "", url: $0.url.flatMap(URL.init(string:))) }
            let events = calendar.map {
                RoutineCalendar(account: $0.account ?? "", matching: $0.matching, minutes: $0.minutes ?? 0, after: $0.after ?? false, nextEventTitle: $0.nextEvent?.title)
            }
            let zone = TimeZone(identifier: timezone ?? "") ?? .current
            let words: String =
                if let watch { L("Watches %@", watch.label) }
                else if let events { Format.aroundEvents(minutes: events.minutes, after: events.after, matching: events.matching) }
                else if let onceAt { Format.once(Date(timeIntervalSince1970: onceAt), in: zone) }
                else { Format.schedule(scheduleText ?? schedule) }
            return Lorca.Routine(
                id: id, botID: botId, name: name, prompt: prompt, schedule: schedule, scheduleText: words,
                isEnabled: isEnabled, pausedReason: pausedReason, lastRunAt: lastRunAt.map { Date(timeIntervalSince1970: $0) },
                lastOutcome: lastOutcome, nextRunAt: nextRunAt.map { Date(timeIntervalSince1970: $0) }, isRunning: isRunning ?? false,
                createdAt: Date(timeIntervalSince1970: createdAt), check: check,
                timezone: timezone ?? TimeZone.current.identifier, missedRunPolicy: missedRunPolicy ?? "coalesce",
                state: state ?? (isEnabled ? "on" : "paused"),
                health: RoutineHealth(
                    lastCheckAt: health?.lastCheckAt.map { Date(timeIntervalSince1970: $0) },
                    lastSuccessAt: health?.lastSuccessAt.map { Date(timeIntervalSince1970: $0) },
                    status: health?.status, connectionFailures: health?.connectionFailures ?? 0,
                    authenticationFailures: health?.authenticationFailures ?? 0, modelStatus: health?.model?.status,
                    modelAuthenticationFailures: health?.model?.authenticationFailures ?? 0),
                onceAt: onceAt.map { Date(timeIntervalSince1970: $0) }, pullRequest: watch, calendar: events)
        }
    }

    /// `device.service_status`: whether `lorca service` keeps the CLI running on that Device.
    struct ServiceStatus: Decodable {
        var installed: Bool
        var running: Bool
    }

    struct RoutineChanged: Decodable {
        var routine: Routine
    }
    struct DurableTaskChanged: Decodable { var task: DurableTask }

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
        var name: String?
        var api: String?
        var models: [StatusModel]?
        var reviewModel: String?

        func toModel() -> ProviderCredential? {
            guard let kind = ProviderCredential.Kind(wireValue: kind) else { return nil }
            return ProviderCredential(
                kind: kind, isConnected: isConnected, detail: detail, baseURL: baseUrl, name: name,
                api: api.flatMap(CustomAPI.init(rawValue:)), models: (models ?? []).map { $0.toModel() }, reviewModel: reviewModel)
        }
    }

    /// A custom provider's model in its status, or in a server's model list.
    struct StatusModel: Decodable {
        var id: String
        var name: String?
        var contextWindow: Int?
        var images: Bool?
        var levels: [String]?

        func toModel() -> CustomModel {
            CustomModel(id: id, name: name, contextWindow: contextWindow, images: images, levels: levels ?? [])
        }
    }

    struct CustomProviderSaved: Decodable {
        var kind: String
    }

    /// `providers.list_models`: the chat models a custom provider's server lists.
    struct ListedModels: Decodable {
        var listed: Bool
        var models: [StatusModel]
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
        var channels: [ChannelStatus]?
        var version: String?
        var update: CLIUpdate?
        /// The relay lists the machine, but it never sent its `machine` blob: no name, no `os`.
        var unknown: Bool?
    }

    struct CLIUpdate: Decodable {
        var auto: Bool
        var latest: String?
        var state: String?
        var error: String?
    }

    struct ChannelStatus: Decodable {
        struct Chat: Decodable {
            var id: String
            var title: String?
        }
        struct Listen: Decodable {
            var every: Bool?
            var mentions: Bool?
            var replies: Bool?
            var tags: [String]?
        }
        var id: String
        var botId: String
        var name: String
        var service: String
        var accountId: String
        var chats: [Chat]?
        var listen: Listen
        var task: String
        var state: String
        var detail: String?
        var heldDelivery: String?

        func toModel() -> Lorca.ChannelStatus {
            Lorca.ChannelStatus(
                id: id, botID: botId, name: name, service: service, accountID: accountId,
                chats: (chats ?? []).map { .init(id: $0.id, title: $0.title ?? "") },
                listen: listen.toModel(), task: task,
                state: Lorca.ChannelStatus.State(rawValue: state) ?? .listening, detail: detail ?? "",
                heldDelivery: heldDelivery)
        }
    }

    struct PluginStatus: Decodable {
        var id: String
        var name: String
        var description: String?
        var version: String?
        var icon: String?
        var state: String
        var detail: String?
        var source: String?
        var serviceId: String?
        var accountName: String?

        func toModel() -> InstalledPlugin {
            InstalledPlugin(
                id: id, name: name, description: description ?? "", version: version ?? "", icon: icon ?? "",
                state: InstalledPlugin.State(rawValue: state) ?? .unknown, detail: detail ?? "", source: source,
                serviceID: serviceId, accountName: accountName)
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
        var namedAccounts: Bool?

        func toModel() -> Lorca.MarketplacePlugin {
            // A server Lorca answers itself (Telegram's, Slack's bot) is Lorca, not one to list.
            let servers = (servers ?? [:]).filter { $0.value.type != "builtin" }.sorted { $0.key < $1.key }.map { name, server in
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
                installedOn: installedOn ?? [], namedAccounts: namedAccounts ?? false)
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
        var packs: [WorkflowPack]?
        var plugins: [MarketplacePlugin]
        var bots: [BotTemplate]
    }

    struct PluginInstalled: Decodable {
        var status: PluginStatus
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
                var code: String?
                var link: String?
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
                servers: servers.map { .init(name: $0.name, kind: $0.kind, url: $0.auth.url, oauth: $0.auth.oauth ?? false, signedIn: $0.auth.signedIn ?? false, code: $0.auth.code, link: $0.auth.link) },
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
        var permissions: BotPermissions?
        var createdAt: Double
    }

    struct ChatChannel: Decodable {
        var channelId: String
        var service: String
        var accountId: String
        var chatId: String
        var threadId: String?
    }

    struct Chat: Decodable {
        var id: String
        var kind: String
        var title: String?
        var channel: ChatChannel?
        var botIds: [String]
        var ownerBotId: String?
        var description: String?
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
        var apiCostUsd: Double?
        var subscriptionEstimateUsd: Double?
        var unknownPriceCalls: Int?
        var pricingKinds: [String]?
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
        var name: String?
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

    struct OutputList: Decodable {
        var outputs: [Message]
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
        var scriptCommand: String?
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
        var replyTo: ReplyTo?
        var reviewId: String?
        var version: UInt64?
        var state: String?
        var account: String?
        var draft: Draft?
        var note: String?
        var direct: Bool?
        var secret: SecretAsk?
    }

    struct Draft: Decodable {
        struct File: Decodable { var name: String; var size: Int64? }
        var kind: String
        var to: [String]?
        var cc: [String]?
        var bcc: [String]?
        var subject: String?
        var body: String?
        var attachments: [File]?
        var reply: String?
    }

    struct SecretAsk: Decodable {
        var use: String
        var site: String?
        var fields: [Field]

        struct Field: Decodable {
            var name: String
            var label: String
        }
    }

    struct SecretList: Decodable {
        var secrets: [Secret]
    }

    struct Secret: Decodable {
        var id: String
        var botId: String
        var name: String
        var label: String
        var use: String
        var site: String?
        var updatedAt: Double

        func toModel() -> SavedSecret {
            SavedSecret(
                id: id, botID: botId, name: name, label: label, use: Lorca.SecretAsk.Use(rawValue: use) ?? .command, site: site,
                updatedAt: Date(timeIntervalSince1970: updatedAt))
        }
    }

    struct ReplyTo: Decodable {
        var messageId: String
        var author: Author
        var text: String
    }

    struct Run: Decodable {
        var sessionId: String?
        var command: String?
        var state: String
        var prompt: String?
        var output: String?
        var device: String?
        var reason: String?
        var rule: String?
        var handedOver: Bool?
        var background: Bool?
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
        var queued: Bool?
        var output: Output?
        var notification: String?
    }

    struct RosterChanged: Decodable {
        var devices: [Device]
        var bots: [Bot]
        var chats: [Chat]
        var routines: [Routine]?
        var playbooks: [PlaybookSummary]?
        var autoReview: AutoReview?
        var sharedLinks: [SharedLink]?
        var providers: [Provider]?
        /// The catalog's models again, so a newer catalog the CLI installs reaches the pickers.
        var models: [Model]?
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
        var error: RelayProblem?
    }

    /// Why the last try to connect to the relay failed.
    struct RelayProblem: Decodable {
        /// The error as it came: the relay's answer, or why none came.
        var message: String
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

    /// `sync.account`: the account's providers once this Device's first pull has its credentials.
    struct SyncAccount: Decodable {
        var providers: [Provider]?
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
            name: unknown == true ? L("Unknown Device") : name,
            model: model,
            os: unknown == true ? Device.OS.unknown : Device.OS(rawValue: os) ?? .linux,
            osVersion: osVersion,
            isThisDevice: isThisDevice,
            status: status == "online" ? .online : (status == "pairing" ? .pairing : .offline),
            lastSeen: Date(timeIntervalSince1970: lastSeen),
            machineKey: machineKey,
            plugins: (plugins ?? []).map { $0.toModel() },
            channels: (channels ?? []).map { $0.toModel() },
            version: version ?? "",
            update: update.map { Device.CLIUpdate(auto: $0.auto, latest: $0.latest, state: $0.state, error: $0.error) }
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
            permissions: permissions,
            createdAt: Date(timeIntervalSince1970: createdAt)
        )
    }
}

extension Wire.Message {
    func toModel() -> Message {
        let author = self.author.toModel()

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
                    scriptCommand: self.body.scriptCommand,
                    run: self.body.run.map {
                        CommandRun(
                            sessionID: $0.sessionId, command: $0.command ?? "",
                            state: CommandRun.State(rawValue: $0.state) ?? .stopped,
                            prompt: $0.prompt, output: $0.output,
                            device: $0.device, reason: $0.reason, rule: $0.rule,
                            handedOver: $0.handedOver ?? false, background: $0.background ?? false)
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
                    command: self.body.command,
                    secret: self.body.secret.map { ask in
                        Lorca.SecretAsk(
                            use: Lorca.SecretAsk.Use(rawValue: ask.use) ?? .command, site: ask.site,
                            fields: ask.fields.map { Lorca.SecretAsk.Field(name: $0.name, label: $0.label) })
                    }))
        case "draft":
            let draft = self.body.draft
            body = .draft(
                DraftCard(
                    reviewID: self.body.reviewId ?? "", version: self.body.version ?? 0, state: self.body.state ?? "pending",
                    pluginID: self.body.pluginId ?? "", account: self.body.account ?? "",
                    fields: DraftCard.Fields(
                        kind: draft?.kind ?? "email", to: draft?.to ?? [], cc: draft?.cc ?? [], bcc: draft?.bcc ?? [],
                        subject: draft?.subject ?? "", body: draft?.body ?? "",
                        attachments: (draft?.attachments ?? []).map { .init(name: $0.name, size: $0.size ?? 0) },
                        reply: draft?.reply),
                    note: self.body.note, direct: self.body.direct ?? false))
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

        var message = Message(
            id: id, author: author, body: body, state: state,
            createdAt: Date(timeIntervalSince1970: createdAt),
            attachments: (self.body.attachments ?? []).map {
                Attachment(id: $0.id, name: $0.name, mime: $0.mime, size: $0.size, width: $0.width, height: $0.height)
            },
            replyTo: self.body.replyTo.map { ReplyQuote(messageID: $0.messageId, author: $0.author.toModel(), text: $0.text) },
            output: output)
        message.queued = queued ?? false
        message.notification = notification
        return message
    }
}

extension Wire.Author {
    func toModel() -> Message.Author {
        switch kind {
        case "you": .you
        case "bot": .bot(botId ?? "")
        case "contact": .contact(name ?? "")
        default: .system
        }
    }
}

extension Wire.Chat {
    func toModel(existingMessages: [Message]? = nil, existingUnread: Int = 0, existingHasMore: Bool = false) -> Chat {
        let modelKind: Chat.Kind = kind == "dm" ? .dm : .group
        return Chat(
            id: id,
            kind: modelKind,
            customTitle: modelKind == .group || channel != nil ? title : nil,
            botIDs: botIds,
            messages: messages.map { $0.map { $0.toModel() } } ?? existingMessages ?? [],
            unreadCount: unreadCount ?? existingUnread,
            isPinned: isPinned,
            createdAt: Date(timeIntervalSince1970: createdAt),
            usage: usage?.toModel(),
            hasMore: hasMore ?? existingHasMore,
            ownerBotID: ownerBotId,
            groupDescription: modelKind == .group ? description ?? "" : "",
            channel: channel.map { ChatChannel(channelID: $0.channelId, service: $0.service, accountID: $0.accountId, chatID: $0.chatId, threadID: $0.threadId) }
        )
    }
}

extension Wire.CallLimits {
    func toModel() -> CallLimits {
        CallLimits(
            maxCalls: limits.maxCalls, windowSecs: limits.windowSecs, maxConcurrency: limits.maxConcurrency,
            retryAt: retryAt.map { Date(timeIntervalSince1970: $0) }, sharesService: serviceId != pluginId)
    }
}

extension Wire.ChannelStatus.Listen {
    func toModel() -> ChannelListen {
        ChannelListen(every: every ?? false, mentions: mentions ?? false, replies: replies ?? false, tags: tags ?? [])
    }
}

extension Wire.ChatUsage {
    func toModel() -> ChatUsage {
        ChatUsage(
            contextTokens: contextTokens, contextWindow: contextWindow, inputTokens: inputTokens, outputTokens: outputTokens,
            cacheReadTokens: cacheReadTokens, costUSD: costUsd, turns: turns, model: model,
            apiCostUSD: apiCostUsd ?? 0, subscriptionEstimateUSD: subscriptionEstimateUsd ?? 0,
            unknownPriceCalls: unknownPriceCalls ?? 0, pricingKinds: pricingKinds ?? [])
    }
}
