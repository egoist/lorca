import Foundation

/// Whose skill it is: one bot's, in every chat it is in, or a group's, for the bots in that group.
struct PlaybookScope: Codable, Hashable {
    let kind: String
    let id: String

    static func bot(_ id: Bot.ID) -> PlaybookScope { PlaybookScope(kind: "bot", id: id) }
    static func group(_ id: Chat.ID) -> PlaybookScope { PlaybookScope(kind: "project", id: id) }

    var isGroup: Bool { kind == "project" }
    var params: [String: Any] { ["kind": kind, "id": id] }
}

/// A reference or a script the skill carries, as text: `references/checklist.md`.
struct PlaybookFile: Codable, Equatable {
    var path: String
    var text: String
}

struct PlaybookContent: Codable, Equatable {
    var name = ""
    var description = ""
    var instructions = ""
    var examples = ""
    var references: [PlaybookFile] = []
    var scripts: [PlaybookFile] = []

    var params: [String: Any] {
        let files = { (files: [PlaybookFile]) in files.map { ["path": $0.path, "text": $0.text] } }
        return ["name": name, "description": description, "instructions": instructions, "examples": examples,
                "references": files(references), "scripts": files(scripts)]
    }
}

/// A skill as the roster lists it, without its body.
struct PlaybookSummary: Decodable, Hashable {
    let id: String
    let scope: PlaybookScope
    let name: String
    let description: String
    let status: String
    let revision: Int
    let hash: String
    let updatedAt: Double

    var isDraft: Bool { status == "draft" }
}

/// One step of a skill's history: who changed it where, and what it said then.
struct PlaybookRevision: Decodable {
    struct Provenance: Decodable {
        let kind: String
        let chatId: String?
        let messageIds: [String]
    }

    let id: String
    let revision: Int
    let status: String
    let content: PlaybookContent?
    let provenance: Provenance
    let deviceId: String
    let createdAt: Double
}

/// A skill with its body and history, fetched when it opens.
struct PlaybookRecord: Decodable {
    let id: String
    let scope: PlaybookScope
    let status: String
    let revision: Int
    let hash: String
    let content: PlaybookContent?
    let revisions: [PlaybookRevision]

    var isDraft: Bool { status == "draft" }
}

extension AppStore {
    /// A bot's or a group's skills: drafts waiting for review first, then the latest changed.
    func skills(in scope: PlaybookScope) -> [PlaybookSummary] {
        playbooks.filter { $0.scope == scope }.sorted {
            $0.isDraft != $1.isDraft ? $0.isDraft : $0.updatedAt > $1.updatedAt
        }
    }

    func playbook(_ id: String, in scope: PlaybookScope) async throws -> PlaybookRecord {
        if isMock {
            guard let record = mockPlaybooks.first(where: { $0.id == id }) else { throw CLIClient.RequestError(message: L("This skill was deleted.")) }
            return record
        }
        return try await client.request("playbooks.get", ["scope": scope.params, "id": id], as: PlaybookRecord.self)
    }

    /// Saves a new skill, or a new revision over the one `record` holds. The CLI refuses it when
    /// the skill changed since `record` was read.
    func savePlaybook(_ content: PlaybookContent, in scope: PlaybookScope, over record: PlaybookRecord?) async throws -> PlaybookRecord {
        if isMock { return saveMockPlaybook(content, in: scope, over: record) }
        var params: [String: Any] = [
            "scope": scope.params, "content": content.params,
            "expected_revision": record?.revision ?? 0, "expected_hash": record?.hash ?? "",
        ]
        if let record {
            params["id"] = record.id
            params["provenance"] = ["kind": "edit"]
        }
        return try await client.request("playbooks.save", params, as: PlaybookRecord.self)
    }

    func removePlaybook(_ record: PlaybookRecord) async throws {
        if isMock {
            mockPlaybooks.removeAll { $0.id == record.id }
            return
        }
        _ = try await client.request("playbooks.remove", [
            "scope": record.scope.params, "id": record.id, "expected_revision": record.revision, "expected_hash": record.hash,
        ])
    }

    /// Has the bot's provider write a draft from the messages picked in the chat; the draft is
    /// used once it is saved.
    func draftPlaybook(in scope: PlaybookScope, botID: Bot.ID, chatID: Chat.ID, kind: String, messageIDs: [Message.ID]) async throws -> PlaybookRecord {
        if isMock {
            try await Task.sleep(nanoseconds: 1_200_000_000)
            var content = MockData.draftedSkill(kind: kind)
            if skills(in: scope).contains(where: { $0.name == content.name }) { content.name += "-2" }
            return saveMockPlaybook(content, in: scope, over: nil, status: "draft")
        }
        return try await client.request("playbooks.draft", [
            "scope": scope.params, "bot_id": botID, "chat_id": chatID, "kind": kind, "message_ids": messageIDs,
        ], as: PlaybookRecord.self)
    }

    /// The skill as a portable file: its content and bundled files, nothing about the account.
    func exportPlaybook(_ id: String, in scope: PlaybookScope) async throws -> Data {
        let data: Data
        if isMock {
            let record = try await playbook(id, in: scope)
            data = try JSONSerialization.data(withJSONObject: ["format": "lorca-playbook", "version": 1, "content": record.content?.params ?? [:]])
        } else {
            data = try await client.request("playbooks.export", ["scope": scope.params, "id": id])
        }
        return try JSONSerialization.data(withJSONObject: JSONSerialization.jsonObject(with: data), options: [.prettyPrinted, .sortedKeys])
    }

    private func saveMockPlaybook(_ content: PlaybookContent, in scope: PlaybookScope, over record: PlaybookRecord?, status: String = "saved") -> PlaybookRecord {
        let revision = (record?.revision ?? 0) + 1
        let step = PlaybookRevision(
            id: UUID().uuidString, revision: revision, status: status, content: content,
            provenance: .init(kind: record == nil ? (status == "draft" ? "workflow" : "manual") : "edit", chatId: nil, messageIds: []),
            deviceId: thisDevice?.id ?? "", createdAt: Date().timeIntervalSince1970)
        let saved = PlaybookRecord(
            id: record?.id ?? "playbook-\(UUID().uuidString.lowercased())", scope: scope, status: status, revision: revision,
            hash: UUID().uuidString, content: content, revisions: (record?.revisions ?? []) + [step])
        mockPlaybooks.removeAll { $0.id == saved.id }
        mockPlaybooks.append(saved)
        return saved
    }
}

extension PlaybookRecord {
    var summary: PlaybookSummary {
        PlaybookSummary(
            id: id, scope: scope, name: content?.name ?? "", description: content?.description ?? "", status: status,
            revision: revision, hash: hash, updatedAt: revisions.map(\.createdAt).max() ?? 0)
    }
}
