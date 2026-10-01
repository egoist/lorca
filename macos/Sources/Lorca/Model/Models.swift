import AppKit
import Foundation

// MARK: - Device

struct ProviderCredential: Hashable, Identifiable {
    enum Kind: Hashable {
        case deepseek
        case anthropic
        case opencode
        case opencodeGo
        case chatgpt
        case grok
        /// A provider the user added: any server that speaks OpenAI's or Anthropic's API. Its
        /// wire value is `custom:` and a slug of the name it was added with.
        case custom(String)

        /// The providers Lorca has built in, in the order the CLI lists them.
        static let builtIn: [Kind] = [.deepseek, .anthropic, .opencode, .opencodeGo, .chatgpt, .grok]

        static let customPrefix = "custom:"

        var isCustom: Bool {
            if case .custom = self { true } else { false }
        }

        /// The name people know the provider by. A custom provider's is the one the user gave it,
        /// from the account's credentials, and its slug once it is deleted.
        @MainActor var name: String {
            switch self {
            case .deepseek: "DeepSeek"
            case .anthropic: "Anthropic"
            case .opencode: "OpenCode Zen"
            case .opencodeGo: "OpenCode Go"
            case .chatgpt: "ChatGPT"
            case .grok: "Grok"
            case .custom(let wire): AppStore.shared.credential(for: self)?.name ?? String(wire.dropFirst(Self.customPrefix.count))
            }
        }

        var symbolName: String {
            switch self {
            case .deepseek, .anthropic, .opencode, .opencodeGo: "key.fill"
            case .chatgpt, .grok: "person.badge.key.fill"
            case .custom: "server.rack"
            }
        }

        var subtitle: String {
            switch self {
            case .deepseek, .anthropic, .opencode, .opencodeGo: L("API key")
            case .chatgpt, .grok: L("Subscription")
            case .custom: L("Custom")
            }
        }

        /// Connects with a pasted API key; ChatGPT and Grok sign in through the browser instead,
        /// and a custom provider is set up in its own sheet.
        var usesAPIKey: Bool {
            switch self {
            case .deepseek, .anthropic, .opencode, .opencodeGo: true
            case .chatgpt, .grok, .custom: false
            }
        }

        /// What the sign-in needs, for the subscription providers.
        var signInRequirement: String {
            switch self {
            case .chatgpt: L("It needs a ChatGPT subscription.")
            case .grok: L("It needs a SuperGrok or X Premium+ subscription.")
            case .deepseek, .anthropic, .opencode, .opencodeGo, .custom: ""
            }
        }

        /// The API root the CLI calls unless the credential names another.
        var defaultBaseURL: String {
            switch self {
            case .deepseek: "https://api.deepseek.com"
            case .anthropic: "https://api.anthropic.com"
            case .opencode: "https://opencode.ai/zen"
            case .opencodeGo: "https://opencode.ai/zen/go"
            case .chatgpt, .grok, .custom: ""
            }
        }

        /// Placeholder for the key field, naming where the key comes from.
        var keyPlaceholder: String {
            switch self {
            case .deepseek: L("sk-… from platform.deepseek.com")
            case .anthropic: L("sk-ant-… from console.anthropic.com")
            case .opencode, .opencodeGo: L("API key from opencode.ai/auth")
            case .custom: L("Optional for a server on your network")
            case .chatgpt, .grok: ""
            }
        }

        /// The identifier the CLI uses on the wire.
        var wireValue: String {
            switch self {
            case .deepseek: "deepseek"
            case .anthropic: "anthropic"
            case .opencode: "opencode"
            case .opencodeGo: "opencode-go"
            case .chatgpt: "chatgpt"
            case .grok: "grok"
            case .custom(let wire): wire
            }
        }

        init?(wireValue: String) {
            switch wireValue {
            case "deepseek": self = .deepseek
            case "anthropic": self = .anthropic
            case "opencode": self = .opencode
            case "opencode-go": self = .opencodeGo
            case "chatgpt": self = .chatgpt
            case "grok": self = .grok
            case let wire where wire.hasPrefix(Self.customPrefix): self = .custom(wire)
            default: return nil
            }
        }

        /// Provider connect methods use underscores even when the stored provider id has a
        /// hyphen.
        var connectMethodSuffix: String {
            switch self {
            case .opencodeGo: "opencode_go"
            case .custom: "custom"
            default: wireValue
            }
        }

        static func thinkingLabel(_ level: String) -> String {
            switch level {
            case "off": L("Off")
            case "minimal": L("Minimal")
            case "low": L("Low")
            case "medium": L("Medium")
            case "high": L("High")
            case "xhigh": L("Extra high")
            case "max": L("Max")
            default: level.prefix(1).uppercased() + level.dropFirst()
            }
        }
    }

    var id: Kind { kind }
    var kind: Kind
    var isConnected: Bool
    var detail: String
    /// A custom API root, when the account credential has one.
    var baseURL: String? = nil
    /// A custom provider's name, the protocol its server speaks, and the models it offers.
    var name: String? = nil
    var api: CustomAPI? = nil
    var models: [CustomModel] = []
}

/// The wire protocol a custom provider's server speaks.
enum CustomAPI: String, CaseIterable, Hashable {
    case chatCompletions = "chat-completions"
    case responses
    case messages

    /// Product names, the same in every language.
    var title: String {
        switch self {
        case .chatCompletions: "OpenAI Chat Completions"
        case .responses: "OpenAI Responses"
        case .messages: "Anthropic Messages"
        }
    }

    /// What the CLI adds to the base URL for a model call.
    var path: String {
        switch self {
        case .chatCompletions: "/chat/completions"
        case .responses: "/responses"
        case .messages: "/v1/messages"
        }
    }

    var baseURLPlaceholder: String {
        self == .messages ? "https://api.example.com" : "https://api.example.com/v1"
    }

    /// The URL the CLI calls for a base URL as typed: a pasted endpoint is cut back to its root
    /// first, as the CLI does, then this protocol's path goes on.
    func endpoint(for baseURL: String) -> String {
        var root = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        while root.hasSuffix("/") { root.removeLast() }
        let pasted: [String] =
            switch self {
            case .chatCompletions: ["/chat/completions"]
            case .responses: ["/responses"]
            case .messages: ["/v1/messages", "/v1"]
            }
        if let suffix = pasted.first(where: { root.hasSuffix($0) }) { root.removeLast(suffix.count) }
        return root + path
    }
}

/// A model a custom provider offers, with what its server's model list says of it and the
/// thinking levels the CLI says it takes.
struct CustomModel: Hashable {
    var id: String
    var name: String? = nil
    var contextWindow: Int? = nil
    /// Whether it takes images.
    var images: Bool? = nil
    var levels: [String] = []

    var displayName: String { name ?? id }
}

/// A server people often add: its API, its base URL, and where its key comes from.
struct CustomProviderPreset: Hashable {
    let name: String
    let api: CustomAPI
    let baseURL: String
    let keyPlaceholder: String

    /// Services in the cloud, for the Add Provider menu.
    static var cloud: [CustomProviderPreset] {
        [
            CustomProviderPreset(
                name: "OpenAI", api: .responses, baseURL: "https://api.openai.com/v1",
                keyPlaceholder: L("sk-… from platform.openai.com")),
            CustomProviderPreset(
                name: "OpenRouter", api: .chatCompletions, baseURL: "https://openrouter.ai/api/v1",
                keyPlaceholder: L("sk-or-… from openrouter.ai/keys")),
            CustomProviderPreset(
                name: "Gemini", api: .chatCompletions, baseURL: "https://generativelanguage.googleapis.com/v1beta/openai",
                keyPlaceholder: L("Key from aistudio.google.com")),
            CustomProviderPreset(
                name: "Groq", api: .chatCompletions, baseURL: "https://api.groq.com/openai/v1",
                keyPlaceholder: L("gsk_… from console.groq.com")),
            CustomProviderPreset(
                name: "Together AI", api: .chatCompletions, baseURL: "https://api.together.xyz/v1",
                keyPlaceholder: L("Key from api.together.ai")),
        ]
    }

    /// The preset for a base URL as typed, by its host and port, so a URL pasted into an empty
    /// sheet still finds the server's name and key hint.
    static func matching(_ baseURL: String) -> CustomProviderPreset? {
        guard let url = URL(string: baseURL.trimmingCharacters(in: .whitespacesAndNewlines)), let host = url.host else { return nil }
        return (cloud + local).first { preset in
            let known = URL(string: preset.baseURL)
            return known?.host == host && known?.port == url.port
        }
    }

    /// Model servers that run on the user's own computers.
    static var local: [CustomProviderPreset] {
        [
            CustomProviderPreset(
                name: "Ollama", api: .chatCompletions, baseURL: "http://localhost:11434/v1",
                keyPlaceholder: L("Optional for a server on your network")),
            CustomProviderPreset(
                name: "LM Studio", api: .chatCompletions, baseURL: "http://localhost:1234/v1",
                keyPlaceholder: L("Optional for a server on your network")),
        ]
    }
}

/// The custom provider sheet's model list: the models the server listed and the ones the user
/// added or saved, which of them bots can pick, and the default.
enum ModelChecklist {
    /// Takes a new listing: the models to keep (picked or added by hand) stay where they are
    /// with the listing's facts, the rest of the old listing goes, and the new one follows.
    static func merge(_ models: [CustomModel], keeping keep: (String) -> Bool, with listed: [CustomModel]) -> [CustomModel] {
        let facts = Dictionary(listed.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        var merged = models.filter { keep($0.id) }.map { model in facts[model.id] ?? model }
        var seen = Set(merged.map(\.id))
        for model in listed where seen.insert(model.id).inserted { merged.append(model) }
        return merged
    }

    /// The models whose name or id holds the query.
    static func filter(_ models: [CustomModel], _ query: String) -> [CustomModel] {
        guard !query.isEmpty else { return models }
        return models.filter { $0.id.localizedCaseInsensitiveContains(query) || ($0.name?.localizedCaseInsensitiveContains(query) ?? false) }
    }

    /// The id an Add row offers for a query: one no model has yet.
    static func addCandidate(_ query: String, in models: [CustomModel]) -> String? {
        let id = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return id.isEmpty || models.contains { $0.id == id } ? nil : id
    }

    /// The picked ids as the CLI keeps them: the default first, then the others in list order.
    static func orderedIDs(_ models: [CustomModel], selected: Set<String>, defaultID: String?) -> [String] {
        let picked = models.map(\.id).filter { selected.contains($0) }
        guard let defaultID, picked.contains(defaultID) else { return picked }
        return [defaultID] + picked.filter { $0 != defaultID }
    }
}

/// A model the CLI's catalog offers, as the snapshot names it, and the thinking levels it
/// takes, lowest first.
struct ProviderModel: Hashable {
    var provider: ProviderCredential.Kind
    var id: String
    var label: String
    var levels: [String]

    /// Every thinking level, lowest first.
    private static let allThinkingLevels = ["off", "minimal", "low", "medium", "high", "xhigh", "max"]

    /// The thinking levels `model` takes among `models`, one provider's in the catalog's order:
    /// the default model's when it is nil, and every level they take for a model the catalog
    /// does not have. nil on a bot means the model's default.
    static func thinkingLevels(for model: String?, among models: [ProviderModel]) -> [(id: String, label: String)] {
        let known = models.first { $0.id == (model ?? models.first?.id) }
        let ids = known?.levels ?? allThinkingLevels.filter { level in models.contains { $0.levels.contains(level) } }
        return ids.map { ($0, ProviderCredential.Kind.thinkingLabel($0)) }
    }
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
        /// A machine the relay lists that never said what it is: never a Runner.
        case unknown

        var displayName: String {
            switch self {
            case .macos: "macOS"
            case .linux: "Linux"
            case .windows: "Windows"
            case .ios: "iOS"
            case .ipados: "iPadOS"
            case .android: "Android"
            case .unknown: ""
            }
        }

        /// Desktop systems run the agent loop. Phones and tablets never do.
        var isDesktop: Bool {
            switch self {
            case .macos, .linux, .windows: true
            case .ios, .ipados, .android, .unknown: false
            }
        }
    }

    enum Status: Hashable {
        case online
        case offline
        case pairing

        var label: String {
            switch self {
            case .online: L("Online")
            case .offline: L("Offline")
            case .pairing: L("Pairing", context: "device state")
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
    /// Plugins installed on this Runner, as it advertises them. Secrets stay on the Runner.
    var plugins: [InstalledPlugin] = []

    /// Derived from `os` alone: a desktop Device is a Runner and can be assigned bots.
    var isRunner: Bool { os.isDesktop }

    var roleLabel: String { isRunner ? L("Runner") : L("Device") }

    /// What the Device panes say about a machine that never said what it is.
    static var unknownNote: String {
        L("This machine is paired to your account but has not sent its name or system. If you don't recognize it, unpair it.")
    }
}

// MARK: - Bot

struct Bot: Identifiable, Hashable {
    let id: String
    var name: String
    /// What the bot does and how it should work.
    var description: String
    var symbolName: String
    var accent: Accent
    var runnerID: Device.ID
    var provider: ProviderCredential.Kind
    /// nil means the provider's default model.
    var model: String? = nil
    /// How much the model thinks; nil means the provider's default.
    var thinking: String? = nil
    /// A custom profile image, kept as a `file` blob like a message attachment. Shown in place
    /// of the symbol and accent once this computer has the bytes.
    var avatar: Attachment? = nil
    var createdAt: Date
}

// MARK: - Auto-review

/// One Auto-review rule: what a bot wants to do, in the user's words, and whether that runs on
/// its own or asks first. Always allow on a shell command adds the rule Auto-review proposed; on
/// a plugin tool it adds a rule for that exact tool.
struct AutoReviewRule: Hashable, Identifiable {
    enum Behavior: String, Hashable {
        case allow
        case ask

        var title: String { self == .allow ? L("Allow automatically") : L("Ask first") }
    }

    var id: String
    var text: String
    var behavior: Behavior
    /// The exact plugin tool (`github/create_issue`) Always allow saved this rule for.
    var tool: String? = nil
}

/// The check on effectful plugin actions and shell commands, shared by every Device through
/// the roster: on, a small model on the bot's provider asks only when needed; off, each one asks.
struct AutoReview: Hashable {
    var isEnabled: Bool = true
    var rules: [AutoReviewRule] = []
}

// MARK: - Plugins

/// A plugin as its Runner advertises it: installed, and in what state.
struct InstalledPlugin: Identifiable, Hashable {
    enum State: String, Hashable {
        case ready
        case needsSetup = "needs_setup"
        case needsAuth = "needs_auth"
        case connecting
        case error
        case unknown
    }

    let id: String
    var name: String
    var description: String
    var version: String
    var icon: String
    var state: State
    var detail: String

    var symbolName: String { icon.isEmpty ? "puzzlepiece.extension" : icon }

    var stateColor: NSColor {
        switch state {
        case .ready: .systemGreen
        case .connecting: .controlAccentColor
        case .error: .systemRed
        case .needsSetup, .needsAuth, .unknown: .systemOrange
        }
    }
}

/// A marketplace plugin, with the Runners that already have it.
struct MarketplacePlugin: Identifiable, Hashable {
    struct Server: Hashable {
        var name: String
        /// The URL of a remote server, or the command a local one runs on the Runner.
        var address: String
        var isRemote: Bool
        var signsIn: Bool
    }

    struct Skill: Hashable {
        var name: String
        var description: String
    }

    struct Variable: Hashable {
        var name: String
        var description: String
        var secret: Bool
        var required: Bool
    }

    let id: String
    var name: String
    var description: String
    var icon: String
    var homepage: String?
    /// Who makes it.
    var author: String
    var category: String
    var isFeatured: Bool
    var tags: [String]
    var servers: [Server]
    var skills: [Skill]
    var variables: [Variable]
    var installedOn: [Device.ID]

    var symbolName: String { icon.isEmpty ? "puzzlepiece.extension" : icon }
    /// At least one server signs in with OAuth on the Runner.
    var signsIn: Bool { servers.contains { $0.signsIn } }
}

/// A bot to add from the marketplace: the profile it starts with, the plugins it works with,
/// the routines it brings (added paused), and facts it starts out knowing.
struct BotTemplate: Identifiable, Hashable {
    struct Routine: Hashable {
        var name: String
        /// The CLI's sentence for the schedule, such as "Weekdays at 9:00 AM".
        var scheduleText: String
        var prompt: String
    }

    let id: String
    var name: String
    /// One line for the marketplace's rows.
    var summary: String
    /// What the bot does and how it should work: the new bot's description.
    var description: String
    var symbolName: String
    var accent: Accent
    var category: String
    var isFeatured: Bool
    var author: String
    var plugins: [MarketplacePlugin.ID]
    var routines: [Routine]
    var memory: [String]
}

/// What the marketplace offers, in the index's order.
struct Marketplace {
    var plugins: [MarketplacePlugin] = []
    var bots: [BotTemplate] = []
}

/// One installed plugin in full, as its Runner reports it: never a secret's value.
struct PluginDetail {
    struct Variable: Hashable {
        var name: String
        var description: String
        var secret: Bool
        var required: Bool
        var isSet: Bool
        var value: String?
    }

    struct Server: Hashable {
        var name: String
        var kind: String
        var url: String?
        var oauth: Bool
        var signedIn: Bool
        /// While a device-flow sign-in waits: the code to enter, and the page to enter it on.
        var code: String? = nil
        var link: String? = nil
    }

    var status: InstalledPlugin
    var homepage: String?
    var variables: [Variable]
    var servers: [Server]
    var skills: [(name: String, description: String)]
}

/// A bot asking before a plugin or shell action runs, or before a plugin is installed.
struct PermissionRequest: Hashable {
    enum Decision: String, Hashable {
        case pending
        case allowed
        case always
        case denied
        case expired
        /// The user sent a new message instead of answering.
        case dismissed
        /// A sign-in card: the flow finished.
        case connected
        case failed
    }

    var pluginID: String
    var pluginName: String
    var tool: String
    var summary: String
    var decision: Decision
    /// A sign-in mid-flow: where to go and the code to enter there.
    var link: String? = nil
    var code: String? = nil
    /// Why Auto-review paused the action, when it did.
    var reason: String? = nil
    /// The rule Always allow adds, which Auto-review proposed for a shell command.
    var rule: String? = nil
    /// A shell card's whole command, where `summary` is its first line.
    var command: String? = nil

    /// The command as the card and its sheet show it, without the summary's `$ ` prompt.
    var fullCommand: String {
        command ?? (summary.hasPrefix("$ ") ? String(summary.dropFirst(2)) : summary)
    }

    var isPending: Bool { decision == .pending }
    var isInstall: Bool { tool == "install" }
    /// A shell command on the bot's Runner.
    var isShell: Bool { pluginID == "computer" }
    /// A sign-in card: Sign in starts the OAuth flow on the Runner.
    var isConnect: Bool { tool == "connect" }

    /// "wants to use GitHub" / "wants to install GitHub" / "needs a sign-in to GitHub" /
    /// "wants to run a command on Workbench"
    var verbPhrase: String {
        if isConnect { return L("needs a sign-in to %@", pluginName) }
        if isShell { return L("wants to run a command on %@", pluginName) }
        return isInstall ? L("wants to install %@", pluginName) : L("wants to use %@", pluginName)
    }

    var decisionText: String {
        switch decision {
        case .pending: L("Waiting for you")
        case .allowed: isConnect ? L("Signing in") : L("Allowed once")
        case .always: L("Always allowed")
        case .denied: isConnect ? L("Not now") : L("Denied")
        case .expired: L("No answer in time")
        case .dismissed: L("Dismissed")
        case .connected: L("Signed in")
        case .failed: L("Sign-in failed")
        }
    }

    /// The buttons a pending card offers: (title, decision). A shell command offers Always
    /// allow only with a rule to add.
    var choices: [(String, String)] {
        if isConnect { return [(L("Sign in"), "allow"), (L("Not now"), "deny")] }
        if isInstall { return [(L("Allow"), "allow"), (L("Deny"), "deny")] }
        if isShell && rule == nil { return [(L("Allow once"), "allow"), (L("Deny"), "deny")] }
        return [(L("Allow once"), "allow"), (L("Always allow"), "always"), (L("Deny"), "deny")]
    }
}

/// A bot's memory as its Runner reports it: the curated index with its load budget, and the
/// other files by name. `here` is false when the bot runs elsewhere and only `runner` is known.
struct BotMemory: Hashable {
    var botID: Bot.ID
    var here: Bool
    var runner: String
    var path: String
    var text: String
    var hash: String
    var lines: Int
    var bytes: Int
    var truncated: Bool
    var maxLines: Int
    var maxBytes: Int
    var topics: [String]
    var logs: [String]

    /// "12 lines · 1.2 KB of 24 KB", or "Empty".
    var budgetSummary: String {
        guard lines > 0 else { return L("Empty") }
        return lines == 1
            ? L("%d line · %@ of %@", lines, Format.kilobytes(bytes), Format.kilobytes(maxBytes))
            : L("%d lines · %@ of %@", lines, Format.kilobytes(bytes), Format.kilobytes(maxBytes))
    }

    /// "2 topics · 5 days of logs"
    var filesSummary: String {
        var parts: [String] = []
        if !topics.isEmpty { parts.append(topics.count == 1 ? L("%d topic", topics.count) : L("%d topics", topics.count)) }
        if !logs.isEmpty { parts.append(logs.count == 1 ? L("%d day of logs", logs.count) : L("%d days of logs", logs.count)) }
        return parts.isEmpty ? L("No other notes yet") : parts.joined(separator: " · ")
    }
}

// MARK: - Routine

/// A recurring task a bot runs on a schedule in its direct chat, as the roster carries it. The
/// schedule's words, the next run, and the running state come from the CLI.
struct Routine: Identifiable, Hashable {
    let id: String
    var botID: Bot.ID
    var name: String
    /// The task, written to the bot, handed to it on every run.
    var prompt: String
    /// `every 30m`, `every 2h`, `every 1d`, or five cron fields in the Runner's local time.
    var schedule: String
    /// The schedule in words: "Weekdays at 9:00 AM".
    var scheduleText: String
    var isEnabled: Bool
    /// Why Lorca paused it, when it did: "away".
    var pausedReason: String?
    var lastRunAt: Date?
    /// How the last run ended: "sent", "pass", or "error".
    var lastOutcome: String?
    var nextRunAt: Date?
    var isRunning: Bool
    var createdAt: Date
    /// The script the Runner runs at each due time before the bot does; the bot runs only when
    /// it finds something. `nextRunAt` is then the next check.
    var check: String? = nil

    /// The line under the name in the inspector: the schedule, then what is going on.
    var detail: String {
        if isRunning { return L("%@ · Running…", scheduleText) }
        guard isEnabled else { return pausedReason == "away" ? L("%@ · Paused while you were away", scheduleText) : L("%@ · Paused", scheduleText) }
        if let nextRunAt {
            return check == nil ? L("%@ · Next %@", scheduleText, Format.upcoming(nextRunAt)) : L("%@ · Next check %@", scheduleText, Format.upcoming(nextRunAt))
        }
        return scheduleText
    }

    /// "Today 9:00 AM · replied", "Never", "Yesterday 6:00 PM · nothing to report".
    var lastRunSummary: String {
        guard let lastRunAt else { return L("Never") }
        let when = Format.daySeparator(lastRunAt)
        switch lastOutcome {
        case "sent": return L("%@ · replied", when)
        case "pass": return L("%@ · nothing to report", when)
        case "error": return L("%@ · failed", when)
        default: return when
        }
    }
}

// MARK: - Message

struct ToolInvocation: Hashable {
    var name: String
    var summary: String
    var detail: String
    var isRunning: Bool
    /// What the call does, in the bot's words: a shell command's "Install dependencies".
    var description: String?
    /// The bot a message_bot call goes to.
    var targetBotID: Bot.ID?
    /// A shell command's card, which the transcript shows only while the command needs the user
    /// (`isShown`). Every `bash` row has one.
    var run: CommandRun? = nil

    /// A finished message_bot call: the one tool the transcript shows, as "Messaged ◉ Name".
    /// Everything else a bot does with tools stays behind the "is working" row.
    var isSentMessage: Bool {
        name == "message_bot" && !isRunning && summary.hasPrefix("Messaged ")
    }

    /// Whether the transcript shows the row: a sent message's marker, or a command's card while
    /// the command needs the user. That is while Auto-review asks to run it, and once the bot
    /// handed the running command over (its turn ended, or it waits on the command at a
    /// question) until it ends. Before that the bot deals with it, and the command shows only
    /// as the working row's activity.
    var isShown: Bool {
        guard let run else { return isSentMessage }
        return run.state == .asking || (run.isLive && run.handedOver)
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

/// Where a shell command stands: Auto-review checking it, the question it asks, the command
/// running in its terminal, what the command asks, and that it ended. While it asks, its card
/// takes the answer to the question (`chats.permission`); once the bot handed the running
/// command over, the user's answer to it (`bash.stdin`) and a Stop (`bash.stop`).
struct CommandRun: Hashable {
    enum State: String, Hashable {
        /// Auto-review is judging it.
        case checking
        /// For the user's permission.
        case asking
        /// Running: printing, or quiet.
        case running
        /// Running, at a question the user can read.
        case waiting
        case exited
        /// Exited with a nonzero code.
        case failed
        case stopped
        /// Not allowed, so it never ran.
        case denied
        /// Nobody answered the question in time.
        case expired
        /// The user sent a new message instead of answering, so it never ran.
        case dismissed
    }

    /// The terminal session running it, once one does. Nil before it starts, and on a Windows
    /// Runner, where a command runs on pipes and takes no answers.
    var sessionID: String?
    /// The command, its first 8,000 characters.
    var command: String
    var state: State
    /// The line it asks with: "[sudo] password for ana:".
    var prompt: String?
    /// Its last lines, as the bottom of a terminal shows them. Never what was typed.
    var output: String?
    /// The Runner it runs on, for the question: "Workbench".
    var device: String?
    /// Why Auto-review asked.
    var reason: String?
    /// The rule Always allow adds.
    var rule: String?
    /// The bot left the command to the user: its turn ended with the command still running, or
    /// it waits on the command at a question.
    var handedOver = false

    var isLive: Bool { state == .waiting || state == .running }
    /// The command runs in a session here or on its Runner: it takes answers and a Stop.
    var takesInput: Bool { isLive && sessionID != nil }
    /// It ran and ended: by itself, with a nonzero code, or stopped.
    var hasEnded: Bool { state == .exited || state == .failed || state == .stopped }

    /// The command's first line with anything on it.
    var firstLine: String {
        command.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty } ?? command
    }

    /// Whether what the user types should show as they type it: a yes-or-no question. Anything
    /// else may be a secret.
    var asksYesOrNo: Bool {
        guard let prompt = prompt?.lowercased() else { return false }
        return prompt.contains("[y/n]") || prompt.contains("(y/n)") || prompt.contains("(yes/no")
    }

    /// The buttons the question offers: (title, decision). Always allow only with a rule to add.
    var choices: [(String, String)] {
        rule == nil ? [(L("Allow once"), "allow"), (L("Deny"), "deny")] : [(L("Allow once"), "allow"), (L("Always allow"), "always"), (L("Deny"), "deny")]
    }
}

/// A file sent with a message. The bytes live under `~/.lorca/files/<id>` once this Device
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
        if attachments.count == 1 { return first.isImage ? L("Photo") : first.name }
        return attachments.allSatisfy(\.isImage) ? L("%d photos", attachments.count) : L("%d files", attachments.count)
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
        case permission(PermissionRequest)
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

    /// A `bash` row's command.
    var commandRun: CommandRun? {
        if case let .tool(tool) = body { return tool.run }
        return nil
    }

    var text: String {
        switch body {
        case let .text(value): value
        case let .tool(tool): tool.summary
        case let .handoff(_, _, reason): reason
        case let .notice(value): value
        case let .permission(request): request.summary
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
    /// What the turns run in this chat used, from the Runner that ran them.
    var usage: ChatUsage? = nil
    /// The CLI holds messages older than the ones here; the transcript asks for them by page.
    var hasMore = false

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


// MARK: - Usage

/// Tokens and money the turns in a chat used. `contextTokens` and `contextWindow` are the last
/// turn's; the rest accumulate.
struct ChatUsage: Hashable {
    var contextTokens: Int
    var contextWindow: Int
    var inputTokens: Int
    var outputTokens: Int
    var cacheReadTokens: Int
    var costUSD: Double
    var turns: Int
    var model: String

    /// "128k of 1M · 13%", or "128k" when the window is unknown.
    var contextSummary: String {
        guard contextWindow > 0 else { return Format.tokens(contextTokens) }
        let percent = Int((Double(contextTokens) / Double(contextWindow) * 100).rounded())
        return L("%@ of %@ · %d%%", Format.tokens(contextTokens), Format.tokens(contextWindow), percent)
    }

    /// "$0.42 · 18 turns"
    var spendSummary: String {
        let dollars = costUSD < 0.01 && costUSD > 0 ? "<$0.01" : String(format: "$%.2f", costUSD)
        return turns == 1 ? L("%@ · %d turn", dollars, turns) : L("%@ · %d turns", dollars, turns)
    }
}

extension Format {
    /// "today 9:00 AM", "tomorrow 9:00 AM", "Monday 9:00 AM", "Oct 1 9:00 AM": when a run is due.
    static func upcoming(_ date: Date) -> String {
        let calendar = Calendar.current
        if calendar.isDateInToday(date) { return L("today %@", time(date)) }
        if calendar.isDateInTomorrow(date) { return L("tomorrow %@", time(date)) }
        if date.timeIntervalSinceNow < 60 * 60 * 24 * 6 {
            let weekday = Format.formatter("EEEE")
            return "\(weekday.string(from: date)) \(time(date))"
        }
        let day = Format.formatter("MMMd")
        return "\(day.string(from: date)) \(time(date))"
    }

    /// "1.2 KB", "24 KB"
    static func kilobytes(_ bytes: Int) -> String {
        let kb = Double(bytes) / 1000
        return kb >= 10 || kb == kb.rounded() ? "\(Int(kb.rounded())) KB" : String(format: "%.1f KB", kb)
    }

    /// "950", "12k", "1.2M"
    static func tokens(_ count: Int) -> String {
        if count >= 1_000_000 { return String(format: "%.1fM", Double(count) / 1_000_000) }
        if count >= 1_000 { return "\(count / 1_000)k" }
        return "\(count)"
    }
}
