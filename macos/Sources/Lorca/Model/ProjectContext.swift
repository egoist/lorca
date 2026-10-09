import Foundation

/// One entry of a group's shared project context, as `projects.get` lists it: a brief, goal,
/// constraint, decision, fact, link, or file that every bot in the group can read. The CLI owns
/// scope, immutable revisions, source checks, and sync; the app shows the current entries and
/// sends the user's changes.
struct ProjectEntry: Decodable, Hashable, Identifiable {
    enum Kind: String, Decodable, CaseIterable {
        case brief, goal, constraint, decision, fact, document, asset

        /// The words a kind goes by; the CLI calls a link `document` and a file `asset`.
        var title: String {
            switch self {
            case .brief: L("Brief")
            case .goal: L("Goal")
            case .constraint: L("Constraint")
            case .decision: L("Decision")
            case .fact: L("Fact")
            case .document: L("Link")
            case .asset: L("File")
            }
        }

        var newTitle: String {
            switch self {
            case .brief: L("New Brief")
            case .goal: L("New Goal")
            case .constraint: L("New Constraint")
            case .decision: L("New Decision")
            case .fact: L("New Fact")
            case .document: L("New Link")
            case .asset: L("New File")
            }
        }

        var symbol: String {
            switch self {
            case .brief: "doc.text"
            case .goal: "flag"
            case .constraint: "hand.raised"
            case .decision: "checkmark.seal"
            case .fact: "info.circle"
            case .document: "link"
            case .asset: "paperclip"
            }
        }
    }

    /// Where an entry came from. Kept whole, so a correction carries a message or output
    /// reference along unchanged.
    struct Source: Codable, Hashable {
        struct Output: Codable, Hashable {
            var chatId: String
            var messageId: String
            var outputId: String
            var version: Int
        }
        var kind: String
        var label: String
        var url: String?
        var messageId: String?
        var output: Output?

        var json: [String: Any] {
            let encoder = JSONEncoder()
            encoder.keyEncodingStrategy = .convertToSnakeCase
            guard let data = try? encoder.encode(self), let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
            return object
        }
    }

    struct Asset: Decodable, Hashable {
        var name: String
        var size: Int64?
    }

    var id: String
    var kind: Kind
    var title: String
    var text: String
    var source: Source
    var verification: String
    var freshness: String
    var updatedAt: Int64
    var verifiedAt: Int64?
    var fetchedAt: Int64?
    var asset: Asset?
    var refreshError: String?
    var supersedes: [String]?
    var current: Bool?
    var removed: Bool?

    var updated: Date { Date(timeIntervalSince1970: TimeInterval(updatedAt)) }

    /// A bot's proposal, waiting for the user to accept it.
    var isSuggestion: Bool { verification == "unverified" && source.kind == "bot" }

    /// The last time the link was read or confirmed.
    var checked: Date? {
        [fetchedAt, verifiedAt].compactMap { $0 }.max().map { Date(timeIntervalSince1970: TimeInterval($0)) }
    }

    /// A bot reads the link again before it relies on it; an agreed decision is the user's to
    /// change, so only its own source is checked.
    var canCheckLink: Bool {
        source.url != nil && !(kind == .decision && verification == "agreed")
    }

    /// The host of the entry's link, for its row.
    var host: String? {
        source.url.flatMap { URL(string: $0)?.host }.map { $0.hasPrefix("www.") ? String($0.dropFirst(4)) : $0 }
    }
}

/// A group's current entries, in the order the inspector lists them, and the entries that are
/// two versions of one (two Devices changed it at once).
struct ProjectContext: Equatable {
    var entries: [ProjectEntry] = []
    var conflicts: [[String]] = []

    /// The other current versions of the entry, when two Devices changed it at once.
    func otherVersions(of id: String) -> [String] {
        conflicts.first { $0.contains(id) }.map { $0.filter { $0 != id } } ?? []
    }

    static func ordered(_ entries: [ProjectEntry]) -> [ProjectEntry] {
        let order = ProjectEntry.Kind.allCases
        return entries.sorted {
            let (a, b) = (order.firstIndex(of: $0.kind) ?? 0, order.firstIndex(of: $1.kind) ?? 0)
            return a != b ? a < b : $0.updatedAt != $1.updatedAt ? $0.updatedAt > $1.updatedAt : $0.id < $1.id
        }
    }
}

private struct ProjectPage: Decodable {
    var entries: [ProjectEntry]
    var hasMore: Bool
    var conflicts: [String: [String]]
}

private struct ProjectFilePath: Decodable {
    var path: String
}

extension AppStore {
    /// The group's current entries, every page of them.
    func projectContext(_ chatID: Chat.ID) async throws -> ProjectContext {
        if isMock { return MockData.projectContext(chatID) }
        var entries: [ProjectEntry] = []
        var conflicts: [[String]] = []
        var after: String?
        while true {
            var params: [String: Any] = ["chat_id": chatID, "limit": 100]
            if let after { params["after"] = after }
            let page = try await client.request("projects.get", params, as: ProjectPage.self)
            entries += page.entries
            conflicts = page.conflicts.values.map { $0.sorted() }
            guard page.hasMore, let last = page.entries.last?.id else { break }
            after = last
        }
        return ProjectContext(entries: ProjectContext.ordered(entries), conflicts: conflicts)
    }

    /// Adds an entry, or saves a new version of `replacing` (and of any other versions of it).
    /// What the user writes is agreed; a link the user typed becomes the entry's source.
    @discardableResult
    func saveProjectEntry(
        in chatID: Chat.ID, kind: ProjectEntry.Kind, title: String, text: String, link: String?,
        replacing: ProjectEntry?, alsoReplacing: [String] = []
    ) async throws -> ProjectEntry? {
        guard !isMock else { return nil }
        var params: [String: Any] = ["chat_id": chatID, "kind": kind.rawValue, "title": title, "text": text, "verification": "agreed"]
        if let replacing { params["supersedes"] = [replacing.id] + alsoReplacing }
        if let replacing, replacing.source.url == link {
            params["source"] = replacing.source.json
        } else if let link {
            params["source"] = ["kind": "url", "label": URL(string: link)?.host ?? link, "url": link]
        }
        return try await client.request("projects.save", params, as: ProjectEntry.self)
    }

    /// What became of an entry another Device changed while it was open here: the current
    /// version it led to, or nil when it was removed.
    func currentProjectEntry(after id: String, in chatID: Chat.ID) async throws -> ProjectEntry? {
        guard !isMock else { return nil }
        var all: [ProjectEntry] = []
        var after: String?
        while true {
            var params: [String: Any] = ["chat_id": chatID, "history": true, "limit": 100]
            if let after { params["after"] = after }
            let page = try await client.request("projects.get", params, as: ProjectPage.self)
            all += page.entries
            guard page.hasMore, let last = page.entries.last?.id else { break }
            after = last
        }
        var frontier = [id]
        var seen: Set<String> = [id]
        while let older = frontier.popLast() {
            for entry in all where entry.supersedes?.contains(older) == true && seen.insert(entry.id).inserted {
                if entry.current == true && entry.removed != true { return entry }
                frontier.append(entry.id)
            }
        }
        return nil
    }

    func removeProjectEntry(_ entry: ProjectEntry, in chatID: Chat.ID) async throws {
        guard !isMock else { return }
        let params: [String: Any] = [
            "chat_id": chatID, "kind": entry.kind.rawValue, "title": entry.title, "source": entry.source.json,
            "supersedes": [entry.id], "removed": true,
        ]
        _ = try await client.request("projects.save", params)
    }

    /// Reads the entry's link again; the answer is its new version, read or not.
    func checkProjectLink(_ entryID: String, in chatID: Chat.ID) async throws -> ProjectEntry? {
        guard !isMock else { return nil }
        return try await client.request("projects.refresh", ["chat_id": chatID, "entry_id": entryID], as: ProjectEntry.self)
    }

    func addProjectFile(_ url: URL, to chatID: Chat.ID) async throws {
        guard !isMock else { return }
        _ = try await client.request("projects.asset", ["chat_id": chatID, "file": ["path": url.path]])
    }

    /// The file on this Device, fetched first when another Device added it.
    func projectFile(_ entryID: String, in chatID: Chat.ID) async throws -> URL {
        if isMock { throw CLIClient.RequestError(message: L("The Lorca CLI is not running")) }
        let reply = try await client.request("projects.asset_path", ["chat_id": chatID, "entry_id": entryID], as: ProjectFilePath.self)
        return URL(fileURLWithPath: reply.path)
    }
}

extension MockData {
    static func projectContext(_ chatID: Chat.ID) -> ProjectContext {
        guard chatID == "chat-relay" else { return ProjectContext() }
        let now = Int64(Date().timeIntervalSince1970)
        func entry(_ id: String, _ kind: ProjectEntry.Kind, _ title: String, _ text: String, source: ProjectEntry.Source = .init(kind: "user", label: "User"), verification: String = "agreed", freshness: String = "agreed", hoursAgo: Int64, fetched: Bool = false, asset: ProjectEntry.Asset? = nil) -> ProjectEntry {
            let at = now - hoursAgo * 3600
            return ProjectEntry(id: id, kind: kind, title: title, text: text, source: source, verification: verification, freshness: freshness, updatedAt: at, verifiedAt: verification == "agreed" ? at : nil, fetchedAt: fetched ? at : nil, asset: asset, refreshError: nil, supersedes: nil, current: true, removed: false)
        }
        return ProjectContext(entries: ProjectContext.ordered([
            entry("ctx-brief", .brief, "Relay launch", "Move every Device to the TLS relay and announce it on Friday. The release notes go out with the build.", hoursAgo: 50),
            entry("ctx-decision", .decision, "Go/no-go on Friday at 10:00", "Nova makes the call after the smoke test passes on Mac, Linux, and the phone.", hoursAgo: 26),
            entry("ctx-constraint", .constraint, "No downtime for paired Devices", "Old relays keep answering until every Device has moved.", hoursAgo: 30),
            entry("ctx-fact", .fact, "Relay p95 latency is 180 ms", "Measured on the staging relay last week.", source: .init(kind: "bot", label: "Scout"), verification: "unverified", freshness: "unverified", hoursAgo: 3),
            entry("ctx-link", .document, "Release checklist", "", source: .init(kind: "url", label: "docs.example.com", url: "https://docs.example.com/relay/checklist"), verification: "fetched", freshness: "fetched", hoursAgo: 5, fetched: true),
            entry("ctx-file", .asset, "Launch announcement", "Writer's final draft.", hoursAgo: 8, asset: .init(name: "announcement.pdf", size: 248_000)),
        ]))
    }
}
