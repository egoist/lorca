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
    /// The model Auto-review runs on it unless the user picks another.
    var reviewModel: String? = nil

    /// A custom provider of decision models, which Auto-review can run and no bot can.
    var decides: Bool { api?.decides == true }
}

/// The wire protocol a custom provider's server speaks.
enum CustomAPI: String, CaseIterable, Hashable {
    case chatCompletions = "chat-completions"
    case responses
    case messages
    case systemOne = "system-one"
    case decisions

    /// Product names, the same in every language.
    var title: String {
        switch self {
        case .chatCompletions: "OpenAI Chat Completions"
        case .responses: "OpenAI Responses"
        case .messages: "Anthropic Messages"
        case .systemOne: "System One"
        case .decisions: "OpenAI Decisions"
        }
    }

    /// A decision API, whose models answer typed questions instead of chatting: Auto-review can
    /// run them, and no bot can.
    var decides: Bool { self == .systemOne || self == .decisions }

    /// What the CLI adds to the base URL for a model call.
    var path: String {
        switch self {
        case .chatCompletions: "/chat/completions"
        case .responses: "/responses"
        case .messages: "/messages"
        case .systemOne: "/systemone"
        case .decisions: "/decisions"
        }
    }

    var baseURLPlaceholder: String { "https://api.example.com/v1" }

    /// The URL the CLI calls for a base URL as typed: a pasted endpoint is cut back to its root
    /// first, as the CLI does, then this protocol's path goes on, Messages' with the `/v1` a root
    /// without one lacks (Moonshot's `…/anthropic`). A decision API's URL is its endpoint, since
    /// vendors serve one at different paths: one that ends in a decision path stays as it is.
    func endpoint(for baseURL: String) -> String {
        var root = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        while root.hasSuffix("/") { root.removeLast() }
        if decides, root.hasSuffix("/systemone") || root.hasSuffix("/decisions") { return root }
        let pasted: [String] =
            switch self {
            case .chatCompletions: ["/chat/completions"]
            case .responses: ["/responses"]
            case .messages: ["/messages"]
            case .systemOne, .decisions: []
            }
        if let suffix = pasted.first(where: { root.hasSuffix($0) }) { root.removeLast(suffix.count) }
        if self == .messages, !root.hasSuffix("/v1") { root += "/v1" }
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

    /// The preset for a base URL as typed, by its host and port and, among a server's presets,
    /// its API, so a URL pasted into an empty sheet still finds the server's name and key hint.
    static func matching(_ baseURL: String, api: CustomAPI? = nil) -> CustomProviderPreset? {
        guard let url = URL(string: baseURL.trimmingCharacters(in: .whitespacesAndNewlines)), let host = url.host else { return nil }
        let server = (cloud + local + decisions).filter { preset in
            let known = URL(string: preset.baseURL)
            return known?.host == host && known?.port == url.port
        }
        return server.first { $0.api == api } ?? server.first
    }

    /// Decision APIs, whose models Auto-review can run.
    static var decisions: [CustomProviderPreset] {
        [
            CustomProviderPreset(
                name: "OpenRouter Decisions", api: .systemOne, baseURL: "https://openrouter.ai/api/alpha/decisions",
                keyPlaceholder: L("sk-or-… from openrouter.ai/keys")),
            CustomProviderPreset(
                name: "OpenAI Decisions", api: .decisions, baseURL: "https://api.openai.com/v1/decisions",
                keyPlaceholder: L("sk-… from platform.openai.com")),
            CustomProviderPreset(
                name: "TypeSafe", api: .systemOne, baseURL: "https://api.typesafe.ai/v1/systemone",
                keyPlaceholder: L("Key from typesafe.ai")),
        ]
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
    /// A decision model, which Auto-review can run and no bot can.
    var decides = false

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
    /// The `lorca` this Device runs.
    var version: String = ""
    /// Set for a CLI that updates itself (one installed with the site's script); a CLI an app
    /// carries updates with the app.
    var update: CLIUpdate? = nil

    /// A self-updating CLI's updates, as its `machine` blob says.
    struct CLIUpdate: Hashable {
        var auto: Bool
        /// A newer release than `version`, when the last check found one.
        var latest: String?
        /// `installing`, `restarting` (waits for its bots to finish), or `installed` (restart
        /// lorca serve by hand); nil otherwise.
        var state: String?
        var error: String?
    }

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
    var permissions: BotPermissions? = nil
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
/// the roster: on, a model asks only when needed; off, each one asks. The model is the review
/// model of the picked provider, else of the bot's.
struct AutoReview: Hashable {
    var isEnabled: Bool = true
    var rules: [AutoReviewRule] = []
    /// The provider that reviews; nil for the bot's own.
    var provider: ProviderCredential.Kind? = nil
    /// The review model picked for each provider; one without uses its default.
    var models: [ProviderCredential.Kind: String] = [:]
}

// MARK: - Plugins

/// A plugin as its Runner advertises it: installed, and in what state.
struct InstalledPlugin: Identifiable, Hashable {
    enum State: String, Hashable {
        case ready
        case needsSetup = "needs_setup"
        case needsAuth = "needs_auth"
        case insufficientAccess = "insufficient_access"
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
    /// `mcp.json` for one of the Runner's own MCP servers, which the server sheet edits.
    var source: String? = nil
    /// A named account's marketplace service (gmail) and the user's name for it (Work). Its `id`
    /// is the account's own.
    var serviceID: String? = nil
    var accountName: String? = nil

    var marketplaceID: String { serviceID ?? id }

    var symbolName: String { icon.isEmpty ? "puzzlepiece.extension" : icon }
    var isMcpServer: Bool { source == "mcp.json" }

    var stateColor: NSColor {
        switch state {
        case .ready: .systemGreen
        case .connecting: .controlAccentColor
        case .error: .systemRed
        case .needsSetup, .needsAuth, .insufficientAccess, .unknown: .systemOrange
        }
    }

    /// The state in a word or two, for a row in a list. What it needs, in full, is in its sheet.
    var shortStatus: String {
        switch state {
        case .ready: L("Connected")
        case .connecting: L("Connecting…")
        case .needsAuth: L("Needs a sign-in")
        case .insufficientAccess: L("Needs more access")
        case .needsSetup: L("Needs setup")
        case .error: L("Can’t connect")
        case .unknown: detail
        }
    }

    /// Colored only when the user has something to do.
    var shortStatusColor: NSColor {
        switch state {
        case .needsSetup, .needsAuth, .insufficientAccess: .systemOrange
        case .error: .systemRed
        case .ready, .connecting, .unknown: .secondaryLabelColor
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
    var namedAccounts: Bool = false

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

    /// The line under the title: an access request names a plugin's tool as it is, or what the
    /// bot wanted to do on its Runner in the CLI's English, which reads here in the app's language.
    var shownSummary: String {
        guard isAccess else { return summary }
        switch summary {
        case "Shell commands": return L("Shell commands")
        case "Changing files": return L("Changing files")
        case "Reading files": return L("Reading files")
        default: return summary
        }
    }

    /// Why the card asks: what Auto-review said, or for an access request, where it is turned on.
    var shownReason: String? { isAccess ? L("Not allowed in this bot's Access settings.") : reason }

    var isPending: Bool { decision == .pending }
    var isInstall: Bool { tool == "install" }
    /// The bot's Access refused a call: the card opens its Access sheet or is dismissed.
    var isAccess: Bool { tool == "access" }
    /// A shell command on the bot's Runner.
    var isShell: Bool { pluginID == "computer" && !isAccess }
    /// A sign-in card: Sign in starts the OAuth flow on the Runner.
    var isConnect: Bool { tool == "connect" }

    /// "wants to use GitHub" / "wants to install GitHub" / "needs a sign-in to GitHub" /
    /// "wants to run a command on Workbench"
    var verbPhrase: String {
        if isAccess { return L("needs more access") }
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
        if isAccess { return [(L("Edit Access…"), "access"), (L("Dismiss"), "deny")] }
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
    /// `every 30m`, `every 2h`, `every 1d`, or five cron fields in `timezone`.
    var schedule: String
    /// The schedule in words: "Weekdays at 9:00 AM".
    var scheduleText: String
    var isEnabled: Bool
    /// Why Lorca paused it, when it did: "away", or "authentication" after three failed
    /// sign-ins in a row.
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
    /// The IANA timezone a cron schedule reads in.
    var timezone = TimeZone.current.identifier
    /// After due times its Runner missed: "coalesce" runs once when it is back, "skip" waits
    /// for the next one.
    var missedRunPolicy = "coalesce"
    /// How it stands, from the CLI: "on", "running", "paused", "blocked", "failed", or
    /// "waiting_for_runner".
    var state = "on"
    var health = RoutineHealth()

    /// What went wrong, while something did: the CLI's state with the kind of failure.
    var problem: RoutineProblem? {
        if isRunning { return nil }
        let model = health.modelStatus != nil
        switch state {
        case "blocked" where pausedReason == "authentication":
            return .signedOut(model: health.modelAuthenticationFailures >= 3)
        case "waiting_for_runner": return .offline
        case "blocked": return .checkBlocked
        case "failed":
            if model { return health.modelAuthenticationFailures > 0 ? .signInFailed(model: true) : .cantConnect(model: true) }
            if health.authenticationFailures > 0 { return .signInFailed(model: false) }
            if health.connectionFailures > 0 { return .cantConnect(model: false) }
            return .checkFailed
        default: return nil
        }
    }

    /// The schedule in words, with its timezone when this Mac keeps other hours, now or in half a
    /// year: "Weekdays at 9:00 AM (New York time)". An interval counts time, whatever the zone.
    var scheduleSummary: String {
        guard !schedule.hasPrefix("every "), let zone = TimeZone(identifier: timezone) else { return scheduleText }
        let now = Date()
        let differs = [now, now.addingTimeInterval(182 * 86_400)].contains { zone.secondsFromGMT(for: $0) != TimeZone.current.secondsFromGMT(for: $0) }
        guard differs else { return scheduleText }
        let city = timezone.split(separator: "/").last.map { $0.replacingOccurrences(of: "_", with: " ") } ?? timezone
        return L("%@ (%@ time)", scheduleText, city)
    }

    /// "Today 9:00 AM · nothing new", for a routine with a check that has run.
    var lastCheckSummary: String? {
        guard check != nil, let at = health.lastCheckAt else { return nil }
        let when = Format.daySeparator(at)
        switch health.status {
        case "quiet": return L("%@ · nothing new", when)
        case "ready": return L("%@ · found something", when)
        case "failed", "blocked": return L("%@ · failed", when)
        default: return when
        }
    }

    /// The line under the name in the inspector: the schedule, then what is going on.
    var detail: String {
        if isRunning { return L("%@ · Running…", scheduleText) }
        if let problem { return "\(problem.text) · \(scheduleText)" }
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

/// How a routine's checks and runs have gone, as its Runner records them.
struct RoutineHealth: Hashable {
    var lastCheckAt: Date?
    var lastSuccessAt: Date?
    /// How the last check went: "quiet", "ready", "failed", or "blocked".
    var status: String?
    var connectionFailures = 0
    var authenticationFailures = 0
    /// The runs' own streak with the model provider, which checks do not clear.
    var modelStatus: String?
    var modelAuthenticationFailures = 0
}

/// What went wrong with a routine, in the words the inspector row and the routine sheet use.
enum RoutineProblem: Hashable {
    /// Three failed sign-ins in a row paused it, of its runs (`model`) or its check.
    case signedOut(model: Bool)
    case offline
    case cantConnect(model: Bool)
    case signInFailed(model: Bool)
    case checkFailed
    /// The check called something that could change things.
    case checkBlocked

    /// One or two words for the row and the sheet's State.
    var text: String {
        switch self {
        case .signedOut: return L("Needs sign-in")
        case .offline: return L("Waiting for Runner")
        case .cantConnect: return L("Can’t connect")
        case .signInFailed: return L("Sign-in failed")
        case .checkFailed, .checkBlocked: return L("Check failed")
        }
    }

    /// Whether the user has to do something; a connection that fails is tried again on its own.
    var needsUser: Bool {
        if case .cantConnect = self { return false }
        return true
    }

    /// What happened and how to fix it, for the routine sheet.
    func explanation(bot: String, runner: String) -> String {
        switch self {
        case .signedOut(model: true):
            return L("The model provider turned down three sign-ins in a row, so the routine is paused. Reconnect the provider in Settings, then resume it.")
        case .signedOut(model: false):
            return L("The check couldn’t sign in to a plugin three times in a row, so the routine is paused. Sign in to the plugin again on %@, then resume it.", runner)
        case .offline:
            return L("%@ is offline, so the routine waits for it. To keep it available while the app is closed, run lorca service install on it.", runner)
        case .cantConnect(model: true):
            return L("The last run couldn’t reach the model provider. It tries again at the next run, waiting longer after each failure.")
        case .cantConnect(model: false):
            return L("The last check couldn’t connect. It tries again at the next check, waiting longer after each failure.")
        case .signInFailed(model: true):
            return L("The last run couldn’t sign in to the model provider. Reconnect it in Settings; after three failures in a row the routine pauses.")
        case .signInFailed(model: false):
            return L("The last check couldn’t sign in to a plugin. Sign in to it again on %@; after three failures in a row the routine pauses.", runner)
        case .checkFailed:
            return L("The check stopped with an error. %@ got the error and can fix the check.", bot)
        case .checkBlocked:
            return L("The check tried to change something, or to use something this bot's Access leaves out. Ask %@ to fix it.", bot)
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
    /// A codemode script's latest command, by its description, while no plugin call came after
    /// it: the status line reads "Running command: Run the tests".
    var scriptCommand: String?
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
    /// It runs in the background: the bot started it there, or the user sent it. Stop in the chat
    /// leaves it running.
    var background = false

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

/// A file or document link a bot published in a chat: one version of it. `id` names the output
/// across its versions; the message that carries it is the version.
struct Output: Hashable, Decodable {
    var id: String
    var name: String
    var mime: String
    var botId: String
    var version: Int
    var url: String?
    var evidence: Evidence?

    /// What the bot says it checked: a test run, a screenshot from before or after a change, or
    /// another check, and how it went.
    struct Evidence: Hashable, Decodable {
        var kind: String
        var summary: String
        var status: String
        var command: String?

        var title: String {
            switch kind {
            case "test_result": L("Test result")
            case "before_screenshot": L("Before screenshot")
            case "after_screenshot": L("After screenshot")
            default: L("Check")
            }
        }

        var statusText: String {
            switch status {
            case "passed": L("Passed")
            case "failed": L("Failed")
            default: L("Not verified")
            }
        }

        var failed: Bool { status == "failed" }
    }

    /// The document a link output points at; only an https address without credentials opens.
    var documentURL: URL? {
        guard let url, let parsed = URL(string: url), parsed.scheme == "https", parsed.host != nil,
            parsed.user == nil, parsed.password == nil else { return nil }
        return parsed
    }

    /// The SF Symbol for what the output is.
    var symbolName: String {
        if url != nil { return "link" }
        if mime.hasPrefix("image/") { return "photo" }
        if mime.hasPrefix("video/") { return "film" }
        if mime.hasPrefix("audio/") { return "waveform" }
        if mime == "application/pdf" { return "doc.richtext" }
        if mime.hasPrefix("text/") || mime == "application/json" { return "doc.text" }
        return "doc"
    }
}

/// An output and its versions, newest first. Each version is a message of its own.
struct OutputSeries: Hashable, Identifiable {
    var versions: [Message]

    var id: String { output.id }
    var latest: Message { versions[0] }
    var output: Output { latest.output! }

    /// The chat's output messages as one series per output, the latest published first.
    static func group(_ messages: [Message]) -> [OutputSeries] {
        var order: [String] = []
        var byID: [String: [Message]] = [:]
        for message in messages {
            guard let output = message.output else { continue }
            if byID[output.id] == nil { order.append(output.id) }
            byID[output.id, default: []].append(message)
        }
        return order
            .map { OutputSeries(versions: byID[$0]!.sorted { $0.output!.version > $1.output!.version }) }
            .sorted { $0.latest.createdAt > $1.latest.createdAt }
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
    /// The message the user answers with this one, quoted.
    var replyTo: ReplyQuote?
    /// A message of the user's the bot's turn holds for its next step; Send now has it read now.
    var queued = false
    var output: Output?

    init(
        id: String = "msg-\(UUID().uuidString.lowercased())",
        author: Author,
        body: Body,
        state: State = .complete,
        createdAt: Date = Date(),
        attachments: [Attachment] = [],
        replyTo: ReplyQuote? = nil,
        output: Output? = nil
    ) {
        self.id = id
        self.author = author
        self.body = body
        self.state = state
        self.createdAt = createdAt
        self.attachments = attachments
        self.replyTo = replyTo
        self.output = output
    }

    /// A finished text message, the user's or a bot's, which a reply can answer.
    var canBeQuoted: Bool {
        guard case .text = body else { return false }
        return state == .complete && author != .system
    }

    /// A `bash` row's command.
    var commandRun: CommandRun? {
        if case let .tool(tool) = body { return tool.run }
        return nil
    }

    /// A command running in its terminal that the bot's call still waits on: Run in Background
    /// sends it there, and the call returns.
    var runsInForeground: Bool {
        guard case let .tool(tool) = body, let run = tool.run else { return false }
        return tool.isRunning && run.takesInput && !run.background
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

/// A message quoted by the user's reply: who wrote it and how it opens, as the CLI keeps it
/// with the reply, so the quote reads the same where the original has not loaded.
struct ReplyQuote: Hashable {
    let messageID: Message.ID
    let author: Message.Author
    let text: String

    /// The quote of `message` the CLI makes, for a reply it has not confirmed yet: its words
    /// without the Markdown, on one line.
    @MainActor
    init?(quoting message: Message) {
        guard message.canBeQuoted else { return nil }
        let words = RenderedMessage(message.text, textColor: .labelColor).plainText
        var line = words.split(whereSeparator: \.isWhitespace).joined(separator: " ")
        if line.isEmpty { line = message.attachments.map(\.name).joined(separator: ", ") }
        if line.count > 280 { line = String(line.prefix(280)).trimmingCharacters(in: .whitespaces) + "…" }
        self.init(messageID: message.id, author: message.author, text: line)
    }

    init(messageID: Message.ID, author: Message.Author, text: String) {
        self.messageID = messageID
        self.author = author
        self.text = text
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
    /// The group member holding the work, as the CLI last said.
    var ownerBotID: Bot.ID? = nil
    /// What a group is for, which every member reads in its system prompt; empty for none.
    var groupDescription = ""

    var isGroup: Bool { kind == .group }

    /// A group's owner: the one set, else the first member, as the CLI picks.
    var owner: Bot.ID? {
        guard isGroup else { return nil }
        if let ownerBotID, botIDs.contains(ownerBotID) { return ownerBotID }
        return botIDs.first
    }
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
