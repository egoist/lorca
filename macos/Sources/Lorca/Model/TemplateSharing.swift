import AppKit

/// The sheets ask the local CLI; tests answer for it.
typealias TemplateReply = @MainActor (String, [String: Any]) async throws -> [String: Any]

/// A piece of a bot template as the export and import sheets list it: the CLI's text, already
/// redacted as the file holds it, and what a reader should look at before sharing it.
struct TemplateItem {
    let id: String
    let title: String
    let detail: String
    /// `email`, `phone`, `path`, `link`, `credential` (a key the export redacted).
    let flags: [String]
    var look: (symbolName: String, accent: Accent)?

    /// A memory reads as its first line, without Markdown's list or heading marks, over the rest.
    static func memory(id: String, text: String, flags: [String]) -> TemplateItem {
        let lines = text.split(whereSeparator: \.isNewline).map { line in
            line.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: #"^(#+|[-*+]|\d+\.)\s+"#, with: "", options: .regularExpression)
        }.filter { !$0.isEmpty }
        return TemplateItem(id: id, title: lines.first ?? text, detail: lines.dropFirst().joined(separator: " "), flags: flags)
    }

    static func profile(_ json: [String: Any], flags: [String]) -> TemplateItem {
        TemplateItem(id: "profile", title: json["name"] as? String ?? "", detail: json["description"] as? String ?? "", flags: flags,
            look: (json["symbol_name"] as? String ?? "sparkles", Accent(rawValue: json["accent"] as? String ?? "") ?? .indigo))
    }

    static func routine(id: String, _ json: [String: Any], scheduleText: String?, flags: [String]) -> TemplateItem {
        let schedule = scheduleText.map(Format.schedule) ?? json["schedule"] as? String ?? ""
        let prompt = json["prompt"] as? String ?? ""
        return TemplateItem(id: id, title: json["name"] as? String ?? "", detail: prompt.isEmpty ? schedule : "\(schedule) · \(prompt)", flags: flags)
    }

    static func skill(id: String, _ json: [String: Any], flags: [String]) -> TemplateItem {
        TemplateItem(id: id, title: json["name"] as? String ?? "", detail: json["description"] as? String ?? "", flags: flags)
    }

    /// The flags in a word or two, and whether one is personal, which the row tints.
    var flagSummary: (text: String, personal: Bool)? {
        let words: [String] = flags.compactMap {
            switch $0 {
            case "email": L("Email address")
            case "phone": L("Phone number")
            case "path": L("File path")
            case "link": L("Link")
            case "credential": L("Key removed")
            default: nil
            }
        }
        guard !words.isEmpty else { return nil }
        return (words.prefix(2).joined(separator: ", "), flags.contains { $0 != "credential" })
    }
}

/// What a bot has that a template can carry (`templates.contents`).
struct TemplateContents {
    struct Plugin {
        let id: String
        let name: String
    }

    let profile: TemplateItem
    let skills: [TemplateItem]
    let memories: [TemplateItem]
    let routines: [TemplateItem]
    let plugins: [Plugin]

    init(json: [String: Any], scheduleText: (String) -> String? = { _ in nil }) {
        func items(_ key: String, _ make: (String, Any, [String]) -> TemplateItem) -> [TemplateItem] {
            (json[key] as? [[String: Any]] ?? []).compactMap { item in
                guard let id = item["id"] as? String, let content = item["content"] else { return nil }
                return make(id, content, item["flags"] as? [String] ?? [])
            }
        }
        let profile = json["profile"] as? [String: Any] ?? [:]
        self.profile = .profile(profile["content"] as? [String: Any] ?? [:], flags: profile["flags"] as? [String] ?? [])
        skills = items("skills") { .skill(id: $0, $1 as? [String: Any] ?? [:], flags: $2) }
        memories = items("memories") { .memory(id: $0, text: $1 as? String ?? "", flags: $2) }
        routines = items("routines") { .routine(id: $0, $1 as? [String: Any] ?? [:], scheduleText: scheduleText($0), flags: $2) }
        plugins = (json["requirements"] as? [[String: Any]] ?? []).compactMap {
            guard let id = $0["service_id"] as? String else { return nil }
            return Plugin(id: id, name: $0["name"] as? String ?? id)
        }
    }
}

/// What `templates.import.preview` says about a file for the Runner picked.
struct TemplateImportPreview {
    struct Connection {
        let id: String
        let name: String
        let isReady: Bool
    }

    struct Plugin {
        let id: String
        let name: String
        let connections: [Connection]
        let selected: String?

        var isReady: Bool { connections.contains { $0.id == selected && $0.isReady } }
    }

    let digest: String
    let canImport: Bool
    let issues: [String]
    /// Whether the file or link held a template this Lorca reads.
    let hasTemplate: Bool
    let plugins: [Plugin]
    let profile: TemplateItem?
    let skills: [TemplateItem]
    let memories: [TemplateItem]
    let routines: [TemplateItem]

    init(json: [String: Any]) {
        digest = json["digest"] as? String ?? ""
        canImport = json["can_import"] as? Bool ?? false
        issues = json["issues"] as? [String] ?? []
        plugins = (json["requirements"] as? [[String: Any]] ?? []).compactMap { requirement in
            guard let id = requirement["service_id"] as? String else { return nil }
            let connections = (requirement["candidates"] as? [[String: Any]] ?? []).compactMap { candidate -> Connection? in
                guard let id = candidate["id"] as? String else { return nil }
                return Connection(id: id, name: candidate["name"] as? String ?? id, isReady: candidate["state"] as? String == "ready")
            }
            return Plugin(id: id, name: requirement["name"] as? String ?? id, connections: connections, selected: requirement["selected"] as? String)
        }
        hasTemplate = json["template"] is [String: Any]
        let template = json["template"] as? [String: Any] ?? [:]
        profile = (template["profile"] as? [String: Any]).map { .profile($0, flags: []) }
        skills = (template["skills"] as? [[String: Any]] ?? []).enumerated().map { .skill(id: "skill-\($0)", $1, flags: []) }
        memories = (template["memories"] as? [String] ?? []).enumerated().map { .memory(id: "memory-\($0)", text: $1, flags: []) }
        routines = (template["routines"] as? [[String: Any]] ?? []).enumerated().map {
            .routine(id: "routine-\($0)", $1, scheduleText: $1["schedule_text"] as? String, flags: [])
        }
    }
}

/// What goes in a template besides the profile, by the ids `templates.contents` gave.
struct TemplateSelection: Codable, Hashable {
    var profile = true
    var skillIds: [String] = []
    var memoryIds: [String] = []
    var routineIds: [String] = []
    var requirementIds: [String] = []

    /// The selection as `templates.export.preview` and `templates.share` take it.
    var json: [String: Any] {
        ["profile": profile, "skill_ids": skillIds, "memory_ids": memoryIds, "routine_ids": routineIds, "requirement_ids": requirementIds]
    }
}

/// A bot the account shares as a link. The roster carries it to every Device, which lists,
/// updates, and revokes it.
struct SharedLink: Decodable, Hashable, Identifiable {
    let id: String
    /// The whole address, the key in its fragment.
    let url: String
    let botId: String
    /// The bot's name when it was last shared.
    let name: String
    let selection: TemplateSelection
    let updatedAt: Double

    var updated: Date { Date(timeIntervalSince1970: updatedAt) }
}

extension AppStore {
    /// The link the bot was last shared as.
    func sharedLink(for botID: Bot.ID) -> SharedLink? {
        sharedLinks.last { $0.botId == botID }
    }

    /// Takes the link down: whoever opens it sees that it no longer works.
    func revokeLink(_ id: SharedLink.ID) async throws {
        _ = try await templateReply("templates.unshare", ["link_id": id])
    }

    func templateReply(_ method: String, _ params: [String: Any]) async throws -> [String: Any] {
        guard !isMock else { throw CLIClient.RequestError(message: L("Templates need the Lorca CLI.")) }
        let data = try await client.request(method, params)
        guard let json = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw CLIClient.RequestError(message: L("Couldn't read the CLI's answer."))
        }
        return json
    }
}
