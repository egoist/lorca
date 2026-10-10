import Foundation

/// A bot's Access, as the CLI keeps it in the encrypted roster: the plugins it may use and how,
/// and what it may do on its Runner. A bot with no policy has full access.
struct BotPermissions: Decodable, Hashable {
    struct Connection: Decodable, Hashable {
        var capabilities: Set<String> = []
        /// Original tool names; nil is every tool the plugin has, one added later included.
        var tools: Set<String>? = nil
    }

    /// Nil is every plugin on the Runner, one installed later included; a map denies the rest.
    var connections: [String: Connection]? = nil
    var filesystem = AccessLevel.write
    var shell = true
    /// Whether the bot's emails and Slack messages wait in the chat as drafts for the user to send.
    var drafts = true

    init() {}

    enum CodingKeys: CodingKey { case connections, filesystem, shell, drafts }
    init(from decoder: Decoder) throws {
        let values = try decoder.container(keyedBy: CodingKeys.self)
        connections = try values.decodeIfPresent([String: Connection].self, forKey: .connections)
        filesystem = AccessLevel(rawValue: try values.decodeIfPresent(String.self, forKey: .filesystem) ?? "write") ?? .none
        shell = try values.decodeIfPresent(Bool.self, forKey: .shell) ?? true
        drafts = try values.decodeIfPresent(Bool.self, forKey: .drafts) ?? true
    }

    var json: [String: Any] {
        var result: [String: Any] = ["filesystem": filesystem.rawValue, "shell": shell, "drafts": drafts]
        if let connections {
            result["connections"] = connections.mapValues { grant in
                var value: [String: Any] = ["capabilities": grant.capabilities.sorted()]
                if let tools = grant.tools { value["tools"] = tools.sorted() }
                return value
            }
        }
        return result
    }

    /// What the bot may do with a plugin: the most its grant reaches.
    func level(of pluginID: String) -> AccessLevel {
        guard let connections else { return .write }
        return AccessLevel(capabilities: connections[pluginID]?.capabilities ?? [])
    }

    /// The Profile card's word for it.
    var summary: String {
        connections == nil && filesystem == .write && shell ? L("Full access") : L("Limited")
    }
}

/// How far a grant reaches, each level taking in the ones below it: a plugin's read, draft,
/// and write capabilities, or files read or changed.
enum AccessLevel: String, CaseIterable {
    case write, draft, read, none

    init(capabilities: Set<String>) {
        self = capabilities.contains("write") ? .write : capabilities.contains("draft") ? .draft : capabilities.contains("read") ? .read : .none
    }

    var capabilities: Set<String> {
        switch self {
        case .write: ["read", "draft", "write"]
        case .draft: ["read", "draft"]
        case .read: ["read"]
        case .none: []
        }
    }

    func allows(_ capability: String) -> Bool { capabilities.contains(capability) }

    var title: String {
        switch self {
        case .write: L("Read and write")
        case .draft: L("Read and draft")
        case .read: L("Read only")
        case .none: L("No access")
        }
    }
}

/// The plugins on a bot's Runner and the tools each offered when it last connected, for the
/// Access sheet.
struct BotAccessCatalog: Decodable {
    struct Plugin: Decodable {
        struct Tool: Decodable {
            var name: String
            var title: String?
            var description: String?
            /// What the tool does, as its Runner last read it: read, draft, or write.
            var capability: String?
        }
        var id: String
        var name: String
        var tools: [Tool]
    }
    var connections: [Plugin]
}
