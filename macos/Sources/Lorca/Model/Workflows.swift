import Foundation

/// A guided workflow from the marketplace: an outcome, the questions it asks, and the accounts
/// it needs. A CLI whose index has none answers without `packs`.
struct WorkflowPack: Decodable, Identifiable {
    struct Question: Decodable {
        var id: String
        var label: String
        var placeholder: String
    }
    struct Requirement: Decodable {
        var serviceId: String
        var name: String
    }
    var id: String
    var name: String
    var outcome: String
    var description: String
    var symbolName: String
    var questions: [Question]
    var connections: [Requirement]

    /// Every word of the query appears in its name, outcome, or description.
    func matches(_ words: [String]) -> Bool {
        let text = [name, outcome, description].joined(separator: " ")
        return words.allSatisfy { text.localizedCaseInsensitiveContains($0) }
    }
}

/// A pack's setup on one Runner, as every `workflows.*` request answers it. The CLI keeps it;
/// the page only shows it.
struct WorkflowProgress: Decodable {
    struct Setup: Decodable {
        struct Sample: Decodable {
            var jobId: String
            var chatId: String
            var botId: String
            /// running, ready, failed, or reviewed.
            var state: String
        }
        var id: String
        var runnerId: String
        var pack: WorkflowPack
        var answers: [String: String]
        var botIds: [String: String]
        var connectionIds: [String: String]
        /// questions, connections, sample, reviewed, enabled, or cancelled.
        var phase: String
        var sample: Sample?
    }
    /// One of the Runner's installed plugins for a service: a named account, or the plugin.
    struct Account: Decodable {
        var plugin: InstalledPlugin
        var accountName: String?

        private enum CodingKeys: String, CodingKey { case accountName }

        init(from decoder: Decoder) throws {
            plugin = try Wire.PluginStatus(from: decoder).toModel()
            accountName = try decoder.container(keyedBy: CodingKeys.self).decodeIfPresent(String.self, forKey: .accountName)
        }
    }
    struct Connection: Decodable {
        var serviceId: String
        var name: String
        var selectedId: String?
        var choices: [Account]
        /// Whether the marketplace has the service, so it can be added.
        var available: Bool

        /// The account in use: the one chosen, else the Runner's only one, which setup takes.
        var account: Account? {
            if let selectedId { return choices.first { $0.plugin.id == selectedId } }
            return choices.count == 1 ? choices[0] : nil
        }
    }
    struct Specialist: Decodable {
        struct Choice: Decodable {
            var id: String
            var name: String
        }
        var id: String
        var name: String
        /// The bot setup uses, or nil when it will add a new one.
        var selectedId: String?
        var choices: [Choice]
    }
    struct Routine: Decodable {
        var id: String
        var name: String
        var scheduleText: String
        var isEnabled: Bool
    }
    struct SampleMessage: Decodable {
        struct Body: Decodable { var text: String? }
        var id: String
        var body: Body
    }
    var setup: Setup
    var connections: [Connection]
    var specialists: [Specialist]
    var routines: [Routine]
    var sampleMessages: [SampleMessage]
    var isRunning: Bool
}

@MainActor
extension AppStore {
    func workflow(_ method: String, _ params: [String: Any]) async throws -> WorkflowProgress {
        if isMock { return try await MockWorkflows.shared.handle(method, params) }
        return try await client.request("workflows.\(method)", params, as: WorkflowProgress.self)
    }
}

/// `LORCA_MOCK=1`'s stand-in for the CLI's setups: the same answers, from the mock roster, with
/// a sample that finishes a moment after it starts.
@MainActor
final class MockWorkflows {
    static let shared = MockWorkflows()
    private var setups: [String: [String: Any]] = [:]
    /// Named accounts the mock Runner has, by service: two inboxes to pick from, a calendar that
    /// waits for its sign-in, and no Drive yet.
    private var accounts: [String: [[String: Any]]] = [
        "gmail": [
            ["id": "gmail-work", "name": "Gmail · Work", "service_id": "gmail", "account_name": "Work", "state": "ready", "detail": "Connected"],
            ["id": "gmail-personal", "name": "Gmail · Personal", "service_id": "gmail", "account_name": "Personal", "state": "ready", "detail": "Connected"],
        ],
        "google-calendar": [
            [
                "id": "google-calendar-work", "name": "Google Calendar · Work", "service_id": "google-calendar", "account_name": "Work",
                "state": "needs_auth", "detail": "Sign in",
            ]
        ],
    ]

    nonisolated static func packs() -> [WorkflowPack] {
        let json: [[String: Any]] = [
            [
                "id": "meeting-preparation", "name": "Meeting preparation", "symbol_name": "calendar",
                "outcome": "Arrive at your next meeting with a briefing and agenda.",
                "description": "Choose a calendar and document account, then review one meeting brief before enabling a weekday routine.",
                "questions": [["id": "meeting-scope", "label": "Meetings", "placeholder": "Today’s meetings with people outside the team"]],
                "connections": [["service_id": "google-calendar", "name": "Google Calendar"], ["service_id": "google-drive", "name": "Google Drive"]],
            ],
            [
                "id": "inbox-triage", "name": "Inbox triage", "symbol_name": "envelope",
                "outcome": "See the messages that need you and a draft of the next action.",
                "description": "Choose an inbox, define what matters, and review a small triage sample before enabling a weekday routine.",
                "questions": [
                    ["id": "inbox-scope", "label": "Messages", "placeholder": "Unread messages from the last day"],
                    ["id": "priorities", "label": "Priorities", "placeholder": "Customer replies, deadlines, blocked teammates"],
                ],
                "connections": [["service_id": "gmail", "name": "Gmail"]],
            ],
            [
                "id": "repository-monitoring", "name": "Repository monitoring", "symbol_name": "arrow.triangle.branch",
                "outcome": "Keep up with the issues, pull requests and releases that need you.",
                "description": "Select repositories and a GitHub connection, then review a summary before enabling a weekday routine.",
                "questions": [["id": "repositories", "label": "Repositories", "placeholder": "owner/repo, owner/another-repo"]],
                "connections": [["service_id": "github", "name": "GitHub"]],
            ],
        ]
        return (try? decode([WorkflowPack].self, json)) ?? []
    }

    private static let specialists = [
        "meeting-preparation": "Meeting Preparer", "inbox-triage": "Inbox Triager", "repository-monitoring": "Repository Monitor",
    ]
    private static let routines = [
        "meeting-preparation": "Prepare upcoming meetings", "inbox-triage": "Triage the selected inbox",
        "repository-monitoring": "Monitor selected repositories",
    ]
    private static let sample = """
        **example/workflow-demo** has two pull requests waiting on you and one new issue.

        - **#128 Fix token refresh on wake**: approved by one reviewer, needs yours.
        - **#131 Pin the relay image**: CI is green; small change.
        - **#133 Crash when pairing offline**: no steps to reproduce yet.

        Start with #128: it blocks the release. Then ask the reporter of #133 for a crash log.
        """

    private nonisolated static func decode<T: Decodable>(_ type: T.Type, _ value: Any) throws -> T {
        try Wire.decoder.decode(type, from: JSONSerialization.data(withJSONObject: value))
    }

    func handle(_ method: String, _ params: [String: Any]) async throws -> WorkflowProgress {
        try await Task.sleep(nanoseconds: 250_000_000)
        let store = AppStore.shared
        if method == "start" {
            guard let packID = params["pack_id"] as? String, let runnerID = params["runner_id"] as? String,
                let pack = Self.packs().first(where: { $0.id == packID })
            else { throw MockError("This workflow is no longer in the marketplace.") }
            let id = "workflow-\(packID)-\(runnerID)"
            if setups[id] == nil || setups[id]?["phase"] as? String == "cancelled" {
                setups[id] = setups[id] ?? ["id": id, "runner_id": runnerID, "pack_id": pack.id, "answers": [String: String](), "phase": "questions"]
                setups[id]?["phase"] = "questions"
            }
            return try progress(id)
        }
        guard let id = params["id"] as? String, var setup = setups[id] else { throw MockError("Unknown workflow setup.") }
        switch method {
        case "configure":
            setup["answers"] = params["answers"] as? [String: String] ?? [:]
            if let picked = (params["bot_ids"] as? [String: String])?.values.first {
                setup["bot_id"] = picked
            } else if setup["bot_id"] == nil, let runnerID = setup["runner_id"] as? String, let packID = setup["pack_id"] as? String {
                setup["bot_id"] = store.createBot(
                    name: Self.specialists[packID] ?? "", symbolName: "eye.fill", accent: .purple, runnerID: runnerID,
                    provider: store.preferredProvider)
            }
            setup["routine_enabled"] = setup["routine_enabled"] ?? false
            setup["phase"] = "connections"
        case "connection":
            let service = params["service_id"] as? String ?? ""
            var connections = setup["connections"] as? [String: String] ?? [:]
            if let picked = params["plugin_id"] as? String {
                connections[service] = picked
            } else {
                // Adds a named account for the workflow, waiting for its sign-in.
                let pack = Self.packs().first { $0.id == setup["pack_id"] as? String }
                let name = pack?.connections.first { $0.serviceId == service }?.name ?? service
                let account: [String: Any] = [
                    "id": "\(service)-added", "name": "\(name) · \(pack?.name ?? "")", "service_id": service, "account_name": pack?.name ?? "",
                    "state": "needs_auth", "detail": "Sign in",
                ]
                accounts[service, default: []].append(account)
                connections[service] = "\(service)-added"
            }
            setup["connections"] = connections
        case "sample":
            setup["sample"] = "running"
            setup["phase"] = "sample"
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: 2_500_000_000)
                guard let self, self.setups[id]?["sample"] as? String == "running" else { return }
                self.setups[id]?["sample"] = "ready"
                store.mockWorkflowChanged()
            }
        case "review":
            setup["sample"] = "reviewed"
            setup["phase"] = "reviewed"
        case "enable":
            setup["routine_enabled"] = true
            setup["phase"] = "enabled"
        case "cancel":
            setup["sample"] = nil
            setup["routine_enabled"] = false
            setup["phase"] = "cancelled"
        default:
            break
        }
        setups[id] = setup
        return try progress(id)
    }

    private func progress(_ id: String) throws -> WorkflowProgress {
        let store = AppStore.shared
        guard let setup = setups[id], let runnerID = setup["runner_id"] as? String, let packID = setup["pack_id"] as? String,
            let pack = Self.packs().first(where: { $0.id == packID })
        else { throw MockError("Unknown workflow setup.") }
        let runnerPlugins = store.device(runnerID)?.plugins.filter { !$0.isMcpServer } ?? []
        let chosen = (setup["connections"] as? [String: String] ?? [:])
            .merging(setup["bot_id"] == nil ? [:] : autoChosen(pack, runnerPlugins)) { picked, _ in picked }
        let botID = setup["bot_id"] as? String
        let chatID = botID.map(store.dm(with:)) ?? ""
        let sampleState = setup["sample"] as? String
        let packJSON: [String: Any] = [
            "id": pack.id, "name": pack.name, "outcome": pack.outcome, "description": pack.description, "symbol_name": pack.symbolName,
            "questions": pack.questions.map { ["id": $0.id, "label": $0.label, "placeholder": $0.placeholder] },
            "connections": pack.connections.map { ["service_id": $0.serviceId, "name": $0.name] },
        ]
        var setupJSON: [String: Any] = [
            "id": id, "runner_id": runnerID, "pack": packJSON, "answers": setup["answers"] ?? [:],
            "bot_ids": botID.map { ["specialist": $0] } ?? [:], "connection_ids": chosen, "phase": setup["phase"] ?? "questions",
        ]
        if let sampleState {
            setupJSON["sample"] = ["job_id": "mock-job", "chat_id": chatID, "bot_id": botID ?? "", "state": sampleState]
        }
        let json: [String: Any] = [
            "setup": setupJSON,
            "connections": pack.connections.map { requirement in
                [
                    "service_id": requirement.serviceId, "name": requirement.name, "selected_id": chosen[requirement.serviceId] as Any,
                    "available": true,
                    "choices": choices(requirement.serviceId, runnerPlugins),
                ] as [String: Any]
            },
            "specialists": [
                [
                    "id": "specialist", "name": Self.specialists[packID] ?? "", "selected_id": botID as Any,
                    "choices": store.bots.filter { $0.runnerID == runnerID }.map { ["id": $0.id, "name": $0.name] },
                ] as [String: Any]
            ],
            "routines": botID == nil
                ? []
                : [
                    [
                        "id": "mock-routine", "name": Self.routines[packID] ?? "", "schedule_text": "Weekdays at 9:00 AM",
                        "is_enabled": setup["routine_enabled"] as? Bool ?? false,
                    ] as [String: Any]
                ],
            "sample_messages": sampleState == "ready" || sampleState == "reviewed" ? [["id": "mock-sample", "body": ["text": Self.sample]]] : [],
            "is_running": sampleState == "running",
        ]
        return try Self.decode(WorkflowProgress.self, json)
    }

    private func choices(_ service: String, _ runnerPlugins: [InstalledPlugin]) -> [[String: Any]] {
        runnerPlugins.filter { $0.marketplaceID == service }.map { ["id": $0.id, "name": $0.name, "state": $0.state.rawValue, "detail": $0.detail] }
            + (accounts[service] ?? [])
    }

    /// What configure takes as the CLI does: a service's only account.
    private func autoChosen(_ pack: WorkflowPack, _ runnerPlugins: [InstalledPlugin]) -> [String: String] {
        var chosen: [String: String] = [:]
        for requirement in pack.connections {
            let all = choices(requirement.serviceId, runnerPlugins)
            if all.count == 1, let id = all[0]["id"] as? String { chosen[requirement.serviceId] = id }
        }
        return chosen
    }
}

private struct MockError: LocalizedError {
    let errorDescription: String?
    init(_ text: String) { errorDescription = text }
}
