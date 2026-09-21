import AppKit
import Foundation

// MARK: - Device

struct ProviderCredential: Hashable, Identifiable {
    enum Kind: String, Hashable, CaseIterable {
        case deepseek = "DeepSeek"
        case anthropic = "Anthropic"
        case opencode = "OpenCode Zen"
        case opencodeGo = "OpenCode Go"
        case chatgpt = "ChatGPT"
        case grok = "Grok"

        var symbolName: String {
            switch self {
            case .deepseek, .anthropic, .opencode, .opencodeGo: "key.fill"
            case .chatgpt, .grok: "person.badge.key.fill"
            }
        }

        var subtitle: String {
            switch self {
            case .deepseek, .anthropic, .opencode, .opencodeGo: L("API key")
            case .chatgpt, .grok: L("Subscription")
            }
        }

        /// Connects with a pasted API key; ChatGPT and Grok sign in through the browser instead.
        var usesAPIKey: Bool { self != .chatgpt && self != .grok }

        /// What the sign-in needs, for the subscription providers.
        var signInRequirement: String {
            switch self {
            case .chatgpt: L("It needs a ChatGPT subscription.")
            case .grok: L("It needs a SuperGrok or X Premium+ subscription.")
            case .deepseek, .anthropic, .opencode, .opencodeGo: ""
            }
        }

        /// The API root the CLI calls unless the credential names another.
        var defaultBaseURL: String {
            switch self {
            case .deepseek: "https://api.deepseek.com"
            case .anthropic: "https://api.anthropic.com"
            case .opencode: "https://opencode.ai/zen"
            case .opencodeGo: "https://opencode.ai/zen/go"
            case .chatgpt, .grok: ""
            }
        }

        /// Placeholder for the key field, naming where the key comes from.
        var keyPlaceholder: String {
            switch self {
            case .deepseek: L("sk-… from platform.deepseek.com")
            case .anthropic: L("sk-ant-… from console.anthropic.com")
            case .opencode, .opencodeGo: L("API key from opencode.ai/auth")
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
            default: return nil
            }
        }

        /// Provider connect methods use underscores even when the stored provider id has a
        /// hyphen.
        var connectMethodSuffix: String {
            self == .opencodeGo ? "opencode_go" : wireValue
        }

        /// The thinking levels this provider's models take, lowest first. nil on a bot means
        /// the provider's default.
        var thinkingLevels: [(id: String, label: String)] {
            let ids: [String] =
                switch self {
                case .deepseek: ["off", "low", "medium", "high", "xhigh", "max"]
                case .anthropic: ["off", "minimal", "low", "medium", "high", "xhigh", "max"]
                case .opencode, .opencodeGo: ["off", "low", "medium", "high", "xhigh", "max"]
                case .chatgpt: ["low", "medium", "high", "xhigh"]
                case .grok: ["low", "medium", "high"]
                }
            return ids.map { ($0, Self.thinkingLabel($0)) }
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

        /// Model ids this provider accepts, first is the default the CLI uses.
        var models: [(id: String, label: String)] {
            switch self {
            case .deepseek:
                [
                    ("deepseek-flash", "V4.1 Flash"),
                    ("deepseek-v4-pro", "V4 Pro (reasoning)"),
                ]
            case .anthropic:
                [
                    ("claude-opus-5", "Opus 5"),
                    ("claude-sonnet-5", "Sonnet 5"),
                    ("claude-fable-5-1", "Fable 5.1"),
                    ("claude-opus-4-8", "Opus 4.8"),
                    ("claude-haiku-4-5", "Haiku 4.5"),
                ]
            case .opencode:
                [
                    ("deepseek-v4.1-flash", "DeepSeek V4.1 Flash"),
                    ("claude-sonnet-5", "Claude Sonnet 5"),
                    ("gpt-5.6-terra", "GPT-5.6 Terra"),
                    ("grok-4.6", "Grok 4.6"),
                    ("kimi-k3", "Kimi K3"),
                    ("big-pickle", "Big Pickle (free)"),
                ]
            case .opencodeGo:
                [
                    ("glm-5.3-flash", "GLM-5.3 Flash"),
                    ("deepseek-v4.1-flash", "DeepSeek V4.1 Flash"),
                    ("gpt-5.6-luna", "GPT-5.6 Luna"),
                    ("grok-4.6", "Grok 4.6"),
                    ("kimi-k3", "Kimi K3"),
                    ("qwen3.8-flash", "Qwen3.8 Flash"),
                    ("minimax-m3", "MiniMax M3"),
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
            case .grok:
                [
                    ("grok-4.6", "Grok 4.6"),
                    ("grok-4.5", "Grok 4.5"),
                    ("grok-4.3", "Grok 4.3"),
                    ("grok-4.20-0309-reasoning", "Grok 4.20 Reasoning"),
                    ("grok-build-0.1", "Grok Build 0.1"),
                ]
            }
        }
    }

    var id: Kind { kind }
    var kind: Kind
    var isConnected: Bool
    var detail: String
    /// A custom API root, when the account credential has one.
    var baseURL: String? = nil
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
}

// MARK: - Bot

struct Bot: Identifiable, Hashable {
    let id: String
    var name: String
    /// One short line under the name: what the bot is for.
    var label: String
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
    /// of the symbol and accent once this Mac has the bytes.
    var avatar: Attachment? = nil
    var createdAt: Date
}

// MARK: - Auto-review

/// One Auto-review rule: what a bot wants to do, in the user's words, and whether that runs on
/// its own or asks first. A rule from Always allow also carries a structured tool key or shell patterns.
struct AutoReviewRule: Hashable, Identifiable {
    enum Behavior: String, Hashable {
        case allow
        case ask

        var title: String { self == .allow ? L("Allow automatically") : L("Ask first") }
    }

    var id: String
    var text: String
    var behavior: Behavior
    var tool: String? = nil
    /// Scope metadata for an exact local shell rule.
    var runnerID: String? = nil
    var workdir: String? = nil
    /// The complete reviewed command for an exact local shell rule.
    var command: String? = nil
    /// Reusable shell command prefixes, scoped by Runner and working directory.
    var patterns: [String] = []
}

/// The check on effectful plugin actions and shell commands, shared by every Device through
/// the roster: on, the bot's model asks only when needed; off, each one asks.
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

/// A marketplace entry, with the Runners that already have it.
struct MarketplacePlugin: Identifiable, Hashable {
    let id: String
    var name: String
    var description: String
    var icon: String
    var homepage: String?
    var tags: [String]
    /// At least one server signs in with OAuth on the Runner.
    var signsIn: Bool
    var variableNames: [String]
    var installedOn: [Device.ID]

    var symbolName: String { icon.isEmpty ? "puzzlepiece.extension" : icon }
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

    var isPending: Bool { decision == .pending }
    var isInstall: Bool { tool == "install" }
    /// A sign-in card: Sign in starts the OAuth flow on the Runner.
    var isConnect: Bool { tool == "connect" }

    /// "wants to use GitHub" / "wants to install GitHub" / "needs a sign-in to GitHub"
    var verbPhrase: String {
        if isConnect { return L("needs a sign-in to %@", pluginName) }
        return isInstall ? L("wants to install %@", pluginName) : L("wants to use %@", pluginName)
    }

    var decisionText: String {
        switch decision {
        case .pending: L("Waiting for you")
        case .allowed: isConnect ? L("Signing in") : L("Allowed once")
        case .always: L("Always allowed")
        case .denied: isConnect ? L("Not now") : L("Denied")
        case .expired: L("No answer in time")
        case .connected: L("Signed in")
        case .failed: L("Sign-in failed")
        }
    }

    /// The buttons a pending card offers: (title, decision).
    var choices: [(String, String)] {
        if isConnect { return [(L("Sign in"), "allow"), (L("Not now"), "deny")] }
        if isInstall { return [(L("Allow"), "allow"), (L("Deny"), "deny")] }
        return [(L("Allow once"), "allow"), (L("Always allow"), "always"), (L("Deny"), "deny")]
    }
}

/// A bot's memory as its Runner reports it: the curated index with its load budget, and the
/// other files by name. `here` is false when the bot runs elsewhere and only `runner` is known.
struct BotMemory {
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

    /// The line under the name in the inspector: the schedule, then what is going on.
    var detail: String {
        if isRunning { return L("%@ · Running…", scheduleText) }
        guard isEnabled else { return pausedReason == "away" ? L("%@ · Paused while you were away", scheduleText) : L("%@ · Paused", scheduleText) }
        if let nextRunAt { return L("%@ · Next %@", scheduleText, Format.upcoming(nextRunAt)) }
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
