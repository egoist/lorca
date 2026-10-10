import AppKit
import Foundation

enum StoreEvent {
    case attentionChanged
    case snapshotReplaced
    case rosterChanged
    case reviewsChanged
    case durableTasksChanged
    case chatsChanged
    case chatChanged(Chat.ID)
    case messageAdded(Chat.ID, Message.ID)
    case messageChanged(Chat.ID, Message.ID)
    case messageRemoved(Chat.ID, Message.ID)
    case respondingChanged(Chat.ID)
    /// A page of older messages was put ahead of the chat's first one.
    case olderMessagesLoaded(Chat.ID)
    /// A bot's turn ended; the date is when this app saw it start.
    case turnFinished(Chat.ID, Bot.ID, Date)
    /// A command in the chat has run long enough to count as a running task.
    case runningTasksChanged(Chat.ID)
    /// A bot's workflow feedback changed on its Runner: notes, suggestions, or changes.
    case feedbackChanged(Bot.ID)
    /// A Runner's limits, or what its work used of them, changed.
    case budgetsChanged
    /// The chat's published outputs, or whether one's file could be fetched, changed.
    case outputsChanged(Chat.ID)
    /// A group's shared project context changed, here or on another Device.
    case projectContextChanged(Chat.ID)
    case selectionChanged
    case connectionChanged
    case identityChanged
}

enum SettingsPane: String, CaseIterable {
    case general
    case providers
    case autoReview = "auto-review"
    case sharedLinks = "shared-links"
    case plugins
    case secrets
    case bots
    case device
    case advanced

    /// Bots and plugins live on a Runner, so these panes show one Device, picked at the top of
    /// the page. The others hold this computer's settings and the account's: the shared roster and
    /// the provider credentials.
    var isDeviceScoped: Bool {
        switch self {
        case .general, .providers, .autoReview, .sharedLinks, .advanced: false
        case .bots, .plugins, .secrets, .device: true
        }
    }
}

enum Selection: Hashable {
    case chat(Chat.ID)
    case settings(SettingsPane)

    /// Settings panes are listed by the settings sidebar; chats by the main one.
    var isSettings: Bool {
        if case .chat = self { return false }
        return true
    }
}

/// The app's model. Everything comes from the CLI over 127.0.0.1; mutations are applied
/// optimistically and confirmed by the events the CLI sends back. `LORCA_MOCK=1` runs the
/// seeded demo data with the in-process reply engine instead.
@MainActor
final class AppStore {
    static let shared = AppStore()

    let isMock = ProcessInfo.processInfo.environment["LORCA_MOCK"] == "1"
    let client = CLIClient()
    let launcher = CLILauncher()

    private(set) var devices: [Device] = []
    private(set) var bots: [Bot] = []
    private(set) var chats: [Chat] = []
    /// Every bot's routines, from the roster.
    private(set) var routines: [Routine] = []
    /// Every Runner's limits and what its turns and routines used of them.
    private(set) var budgets: [BudgetState] = []
    private(set) var reviews: [ReviewItem] = []
    private(set) var durableTasks: [DurableTask] = []
    /// Auto-review, shared through the roster.
    private(set) var autoReview = AutoReview()
    /// What waits on the user across chats, kept by the bots (`attention.changed`).
    private(set) var attention = AttentionView()
    /// The bots the account shares as links, shared through the roster.
    private(set) var sharedLinks: [SharedLink] = []
    /// The account's provider credentials, the same on every Device.
    private(set) var providers: [ProviderCredential] = []
    /// The models the CLI's catalog offers, for the Model and Thinking pickers.
    private(set) var catalog: [ProviderModel] = []
    /// Every bot's and group's skills and drafts, from the roster; a body is fetched when one opens.
    private(set) var playbooks: [PlaybookSummary] = []
    /// The demo's skills, bodies and all.
    var mockPlaybooks: [PlaybookRecord] = [] {
        didSet {
            playbooks = mockPlaybooks.map(\.summary)
            emit(.rosterChanged)
        }
    }

    /// True when the CLI answers on localhost (mock: toggled from the Debug menu).
    private(set) var isConnected = false
    /// The first connection is still loading. The window is visible throughout this phase.
    private(set) var isStarting = true
    /// nil until the CLI has supplied the initial snapshot.
    private(set) var hasIdentity: Bool?
    private(set) var isIdentityDevice = false
    private(set) var identityID: String?
    private(set) var relayConnected = false
    /// The relay refused this build's protocol: it syncs again once Lorca is updated.
    private(set) var relayUpdateRequired = false
    /// Why the last try to connect to the relay failed, as the CLI words it, until one goes
    /// through.
    private(set) var relayError: String?
    private(set) var relayURL: String?

    /// Turns in flight, by job id: the chat and the bot (empty while a group exchange is between
    /// member turns), and the routine when the turn is one of its runs. Drives the "is working"
    /// row, the presence dot on avatars, and the spinner on a routine.
    private var runningJobs: [(id: String, chatID: Chat.ID, botID: Bot.ID, routineID: Routine.ID?)] = []
    /// When each turn in flight was first seen, so a finished turn's reply can be told from
    /// what the bot said before it.
    private var jobStarts: [String: Date] = [:]
    /// When this app saw each command start running in its terminal, by row.
    private var commandStarts: [Message.ID: Date] = [:]
    /// How long a command runs before it counts as a running task.
    static let taskDelay: TimeInterval = 2
    /// The chat last reported to the CLI as on screen; `.some(nil)` is "none".
    private var reportedWatchedChat: Chat.ID??
    private var loadingOlder: Set<Chat.ID> = []
    private var replyEngine: ReplyEngine?
    private var started = false
    private var startupTask: Task<Void, Never>?
    private var bootstrapGeneration = 0
    private var isBootstrapping = true
    private var bootstrapEvents: [(name: String, data: Data)] = []
    private var isApplyingBootstrap = false

    private struct Subscription {
        weak var owner: AnyObject?
        let handler: (StoreEvent) -> Void
    }

    private var subscriptions: [Subscription] = []

    private init() {
        if isMock {
            replyEngine = ReplyEngine(store: self)
        }
    }

    // MARK: - Lifecycle

    func start() {
        guard !started else { return }
        StartupTrace.mark("store starting")
        started = true
        if isMock {
            resetMockData()
            isConnected = true
            finishStartup()
            hasIdentity = true
            isIdentityDevice = true
            emit(.connectionChanged)
            emit(.identityChanged)
            return
        }

        // A slow or silent CLI leaves the user with the offline recovery controls. This
        // deadline bounds the loading state, never the time until a window is shown.
        startupTask = Task { [weak self] in
            do { try await Task.sleep(nanoseconds: 2_500_000_000) } catch { return }
            guard let self else { return }
            self.finishStartup()
            self.emit(.connectionChanged)
        }

        client.onStateChange = { [weak self] state in
            guard let self else { return }
            switch state {
            case .connected:
                StartupTrace.mark("websocket connected")
                self.bootstrapGeneration += 1
                let generation = self.bootstrapGeneration
                Task { await self.bootstrap(generation: generation) }
            case .disconnected, .connecting:
                self.bootstrapGeneration += 1
                self.isBootstrapping = true
                self.bootstrapEvents.removeAll()
                self.reportedWatchedChat = nil
                if self.isConnected {
                    self.isConnected = false
                    self.runningJobs.removeAll()
                    self.thinkingBots.removeAll()
                    self.emit(.connectionChanged)
                }
            }
        }
        client.onEvent = { [weak self] name, data in
            guard let self else { return }
            if self.isBootstrapping {
                self.bootstrapEvents.append((name, data))
            } else {
                self.handle(event: name, data: data)
            }
        }
        launcher.onStatusChange = { [weak self] status in
            guard let self else { return }
            // A child that is starting or restarting owns the next connection attempt.
            // Cancel any socket retry left over from the previous process.
            switch status {
            case .starting, .failed: self.client.disconnect()
            default: break
            }
            if case .failed = status { self.finishStartup() }
            self.emit(.connectionChanged)
        }
        launcher.onReady = { [weak self] in self?.client.connect() }
        client.onReconnectNeeded = { [weak self] in self?.launcher.ensureRunning() }
        launcher.ensureRunning()
    }

    func stop() {
        finishStartup()
        client.disconnect()
        launcher.stop()
    }

    private func finishStartup() {
        startupTask?.cancel()
        startupTask = nil
        isStarting = false
    }

    /// What the offline state should say while the CLI is not answering.
    var offlineStatus: String {
        launcher.status.message
    }

    func reconnect() {
        guard !isMock else {
            setConnected(true)
            return
        }
        client.reconnect()
    }

    private func bootstrap(generation: Int) async {
        do {
            // The snapshot includes identity and connection state as well as messages. Publish
            // them together, so roster events cannot expose empty previews before it arrives.
            let snapshot = try await client.request("bootstrap", as: Wire.Snapshot.self)
            guard generation == bootstrapGeneration else { return }
            StartupTrace.mark("snapshot received")
            isApplyingBootstrap = true
            apply(snapshot: snapshot)
            for event in bootstrapEvents { handle(event: event.name, data: event.data) }
            bootstrapEvents.removeAll()
            isApplyingBootstrap = false
            isBootstrapping = false
            isConnected = true
            finishStartup()
            emit(.snapshotReplaced)
            emit(.connectionChanged)
            emit(.identityChanged)
            StartupTrace.mark("snapshot presented")
        } catch {
            guard generation == bootstrapGeneration else { return }
            finishStartup()
            emit(.connectionChanged)
            NSLog("bootstrap failed: \(error.localizedDescription)")
        }
    }

    private func apply(snapshot: Wire.Snapshot) {
        hasIdentity = snapshot.hasIdentity
        isIdentityDevice = snapshot.isIdentityDevice
        identityID = snapshot.identityId
        relayURL = snapshot.relayUrl
        relayConnected = snapshot.relayConnected
        relayUpdateRequired = snapshot.relayUpdateRequired ?? false
        relayError = snapshot.relayError?.message
        devices = snapshot.devices.map { $0.toModel() }
        bots = snapshot.bots.map { $0.toModel() }
        // A snapshot carries each chat's newest messages. Older pages this app already loaded
        // stay ahead of them, so a resync does not throw the transcript back to the last page.
        let loaded = chats
        chats = snapshot.chats.map { incoming in
            var chat = incoming.toModel()
            if let existing = loaded.first(where: { $0.id == chat.id }), let first = chat.messages.first,
                let index = existing.index(of: first.id), index > 0
            {
                chat.messages.insert(contentsOf: existing.messages[..<index], at: 0)
                chat.hasMore = existing.hasMore
            }
            return chat
        }
        routines = (snapshot.routines ?? []).map { $0.toModel() }
        budgets = snapshot.budgets ?? []
        reviews = snapshot.reviews ?? []
        durableTasks = snapshot.tasks ?? []
        playbooks = snapshot.playbooks ?? []
        autoReview = snapshot.autoReview?.toModel() ?? AutoReview()
        attention = snapshot.attention ?? AttentionView()
        sharedLinks = snapshot.sharedLinks ?? []
        providers = (snapshot.providers ?? []).compactMap { $0.toModel() }
        catalog = (snapshot.models ?? []).compactMap { $0.toModel() }
        runningJobs = (snapshot.runningTurns ?? []).map { ($0.jobId, $0.chatId, $0.botId, $0.routineId) }
        for id in snapshot.runningChatIds where !runningJobs.contains(where: { $0.chatID == id }) {
            runningJobs.append(("chat:\(id)", id, "", nil))
        }
        sortChats()
        // A resync may bring outputs this app missed; they are asked for again when next shown.
        staleOutputs = Set(outputMessages.keys)
        emit(.snapshotReplaced)
    }

    // MARK: - Events from the CLI

    private func handle(event name: String, data: Data) {
        func decode<T: Decodable>(_ type: T.Type) -> T? {
            try? Wire.decoder.decode(Wire.Envelope<T>.self, from: data).data
        }

        switch name {
        case "attention.changed":
            if let incoming = decode(AttentionView.self) { applyAttention(incoming) }
        case "reviews.changed":
            struct Change: Decodable { var item: ReviewItem }
            if let change = decode(Change.self) { upsertReview(change.item) }
        case "snapshot":
            if let snapshot = decode(Wire.Snapshot.self) { apply(snapshot: snapshot) }

        case "feedback.changed":
            if let envelope = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let body = envelope["data"] as? [String: Any], let botID = body["bot_id"] as? String {
                emit(.feedbackChanged(botID))
            }
        case "tasks.changed":
            guard let payload = decode(Wire.DurableTaskChanged.self) else { return }
            acceptDurableTask(payload.task)

        case "roster.changed":
            guard let roster = decode(Wire.RosterChanged.self) else { return }
            devices = roster.devices.map { $0.toModel() }
            bots = roster.bots.map { $0.toModel() }
            if let incoming = roster.routines { routines = incoming.map { $0.toModel() } }
            if let incoming = roster.playbooks { playbooks = incoming }
            if let incoming = roster.autoReview { autoReview = incoming.toModel() }
            if let incoming = roster.sharedLinks { sharedLinks = incoming }
            if let incoming = roster.providers { providers = incoming.compactMap { $0.toModel() } }
            if let incoming = roster.models { catalog = incoming.compactMap { $0.toModel() } }
            var merged: [Chat] = []
            var changed: [Chat.ID] = []
            for summary in roster.chats {
                let existing = chats.first { $0.id == summary.id }
                let chat = summary.toModel(
                    existingMessages: existing?.messages ?? [], existingUnread: existing?.unreadCount ?? 0, existingHasMore: existing?.hasMore ?? false)
                if let existing, existing.botIDs != chat.botIDs || existing.customTitle != chat.customTitle {
                    changed.append(chat.id)
                }
                merged.append(chat)
            }
            chats = merged
            sortChats()
            emit(.rosterChanged)
            emit(.chatsChanged)
            for id in changed { emit(.chatChanged(id)) }

        case "message.added", "message.updated":
            guard let payload = decode(Wire.MessageEvent.self) else { return }
            if retryNotes[payload.chatId] != nil {
                retryNotes[payload.chatId] = nil
                emit(.respondingChanged(payload.chatId))
            }
            // The thinking bot's next message (a tool call, a reply) is where its thinking went.
            if name == "message.added", let botID = payload.message.author.botId, thinkingBots[payload.chatId] == botID {
                thinkingBots[payload.chatId] = nil
            }
            upsert(payload.message.toModel(), in: payload.chatId)

        case "message.removed":
            guard let payload = decode(Wire.MessageRemoved.self),
                let index = chats.firstIndex(where: { $0.id == payload.chatId })
            else { return }
            chats[index].messages.removeAll { $0.id == payload.messageId }
            commandStarts[payload.messageId] = nil
            emit(.messageRemoved(payload.chatId, payload.messageId))
            noteOutput(nil, removing: payload.messageId, in: payload.chatId)

        case "chat.removed":
            guard let payload = decode(Wire.ChatRemoved.self) else { return }
            chats.removeAll { $0.id == payload.chatId }
            runningJobs.removeAll { $0.chatID == payload.chatId }
            outputMessages[payload.chatId] = nil
            staleOutputs.remove(payload.chatId)
            emit(.chatsChanged)

        case "job.started":
            guard let job = decode(Wire.JobEvent.self) else { return }
            // A snapshot may already list it.
            runningJobs.removeAll { $0.id == "pending:\(job.chatId)" || $0.id == job.jobId }
            runningJobs.append((job.jobId, job.chatId, job.botId, job.routineId))
            jobStarts[job.jobId] = jobStarts[job.jobId] ?? Date()
            emit(.respondingChanged(job.chatId))
            emit(.chatsChanged)

        case "job.finished":
            guard let job = decode(Wire.JobEvent.self) else { return }
            runningJobs.removeAll { $0.id == job.jobId }
            retryNotes[job.chatId] = nil
            if job.botId.isEmpty || thinkingBots[job.chatId] == job.botId {
                thinkingBots[job.chatId] = nil
            }
            emit(.respondingChanged(job.chatId))
            emit(.chatsChanged)
            if let startedAt = jobStarts.removeValue(forKey: job.jobId), !job.botId.isEmpty {
                emit(.turnFinished(job.chatId, job.botId, startedAt))
            }

        case "job.retry":
            guard let retry = decode(Wire.JobRetry.self) else { return }
            let seconds = max(1, Int((Double(retry.delayMs) / 1000).rounded()))
            retryNotes[retry.chatId] = L("Retrying (%d of %d) in %d s", retry.attempt, retry.maxAttempts, seconds)
            emit(.respondingChanged(retry.chatId))

        case "job.thinking":
            guard let job = decode(Wire.JobThinking.self) else { return }
            thinkingBots[job.chatId] = job.botId
            emit(.respondingChanged(job.chatId))

        case "chat.usage":
            guard let payload = decode(Wire.ChatUsageEvent.self),
                let index = chats.firstIndex(where: { $0.id == payload.chatId })
            else { return }
            chats[index].usage = payload.usage.toModel()
            emit(.chatChanged(payload.chatId))

        case "budgets.changed":
            guard let payload = decode(Wire.BudgetsChanged.self) else { return }
            budgets = payload.budgets
            emit(.budgetsChanged)

        case "relay.status":
            guard let status = decode(Wire.RelayStatus.self) else { return }
            relayConnected = status.connected
            relayUpdateRequired = status.updateRequired ?? false
            relayError = status.error?.message
            relayURL = status.url ?? relayURL
            emit(.rosterChanged)

        case "projects.changed":
            struct ProjectChanged: Decodable { var chatId: String }
            guard let payload = decode(ProjectChanged.self) else { return }
            emit(.projectContextChanged(payload.chatId))

        case "identity.changed":
            guard let payload = decode(Wire.IdentityChanged.self) else { return }
            hasIdentity = payload.hasIdentity
            emit(.identityChanged)

        default:
            break
        }
    }

    private func upsert(_ message: Message, in chatID: Chat.ID) {
        guard let chatIndex = chats.firstIndex(where: { $0.id == chatID }) else { return }
        noteCommand(message, in: chatID)
        noteOutput(message, in: chatID)
        if let messageIndex = chats[chatIndex].index(of: message.id) {
            var message = message
            // A draft card the Runner wrote again, unchanged, keeps what the user is typing in it.
            if case let .draft(old) = chats[chatIndex].messages[messageIndex].body, case var .draft(new) = message.body,
                new.version == old.version, new.isPending, old.isPending
            {
                new.edited = old.edited
                message.body = .draft(new)
            }
            chats[chatIndex].messages[messageIndex] = message
            emit(.messageChanged(chatID, message.id))
            if message.state == .complete { refreshChatList() }
        } else {
            chats[chatIndex].messages.append(message)
            emit(.messageAdded(chatID, message.id))
            sortChats()
            emit(.chatsChanged)
        }
    }

    // MARK: - Observation

    func applyAttention(_ view: AttentionView) {
        guard view != attention else { return }
        attention = view
        emit(.attentionChanged)
    }

    /// Takes an item off the Attention list. A coordinator resolves its items itself; this is
    /// the user saying it is done. The task or review it links to is left as it is.
    func resolveAttention(_ item: AttentionItem) async throws {
        if isMock {
            var view = attention
            view.items.removeAll { $0.id == item.id }
            return applyAttention(view)
        }
        _ = try await client.request("attention.resolve", ["id": item.id, "expected_revision": item.revision.params])
    }

    /// `summaries`, `urgent_direct`, or `default_coordinator_bot_id` (a bot's id, or nil for
    /// each chat's own coordinator). The account's, on every Device.
    func setAttentionPreference(_ key: String, _ value: Any?) async throws {
        if isMock {
            var view = attention
            switch key {
            case "summaries": view.preferences.summaries = value as? Bool ?? true
            case "urgent_direct": view.preferences.urgentDirect = value as? Bool ?? true
            default: view.preferences.defaultCoordinatorBotId = value as? String
            }
            return applyAttention(view)
        }
        _ = try await client.request("attention.preferences", [key: value ?? NSNull()])
    }

    func observe(_ owner: AnyObject, _ handler: @escaping (StoreEvent) -> Void) {
        subscriptions.append(Subscription(owner: owner, handler: handler))
    }

    /// A mock workflow's sample finished; open pages read its setup again.
    func mockWorkflowChanged() {
        emit(.rosterChanged)
    }

    private func emit(_ event: StoreEvent) {
        guard !isApplyingBootstrap else { return }
        subscriptions.removeAll { $0.owner == nil }
        for subscription in subscriptions {
            subscription.handler(event)
        }
    }

    // MARK: - Lookup

    func bot(_ id: Bot.ID) -> Bot? {
        bots.first { $0.id == id }
    }

    func device(_ id: Device.ID) -> Device? {
        devices.first { $0.id == id }
    }

    /// Devices that can be assigned bots: those with a desktop `os`.
    var runners: [Device] {
        devices.filter(\.isRunner)
    }

    func chat(_ id: Chat.ID) -> Chat? {
        chats.first { $0.id == id }
    }

    func bots(in chat: Chat) -> [Bot] {
        chat.botIDs.compactMap(bot)
    }

    func bots(on runnerID: Device.ID) -> [Bot] {
        bots.filter { $0.runnerID == runnerID }
    }

    func routine(_ id: Routine.ID) -> Routine? {
        routines.first { $0.id == id }
    }

    /// A bot's routines, oldest first, with the running state from the turns in flight.
    func routines(for botID: Bot.ID) -> [Routine] {
        routines.filter { $0.botID == botID }.sorted { $0.createdAt < $1.createdAt }.map { routine in
            var routine = routine
            routine.isRunning = routine.isRunning || runningJobs.contains { $0.routineID == routine.id }
            return routine
        }
    }

    var thisDevice: Device? {
        devices.first(where: \.isThisDevice)
    }

    func title(for chat: Chat) -> String {
        if chat.isGroup, let custom = chat.customTitle, !custom.isEmpty { return custom }
        let names = chat.botIDs.compactMap { bot($0)?.name }
        return names.isEmpty ? L("New Chat") : names.joined(separator: ", ")
    }

    func subtitle(for chat: Chat) -> String {
        let members = bots(in: chat)
        if chat.isDM, let only = members.first {
            let host = device(only.runnerID)?.name ?? L("unassigned")
            return L("%@ on %@", only.provider.name, host)
        }
        let hosts = Set(members.compactMap { device($0.runnerID)?.name })
        let runnerLabel = hosts.count == 1 ? (hosts.first ?? "") : L("%d Runners", hosts.count)
        let botLabel = members.count == 1 ? L("1 bot") : L("%d bots", members.count)
        return L("Group · %@ · %@", botLabel, runnerLabel)
    }

    func preview(for chat: Chat) -> String {
        // The last thing worth previewing: tool calls never are, except a sent message.
        let shown = chat.messages.last { message in
            if case let .tool(tool) = message.body { return tool.isSentMessage }
            return true
        }
        guard let last = shown else { return L("No messages yet") }
        let body: String
        switch last.body {
        case let .text(value): body = value.isEmpty ? Attachment.summary(last.attachments) : value
        case let .tool(tool): body = L("Messaged %@: %@", tool.targetBotID.flatMap(bot)?.name ?? L("a teammate"), tool.detail)
        case let .handoff(from, to, reason):
            body = chat.botIDs.contains(to) && !chat.botIDs.contains(from)
                ? L("Message from %@: %@", bot(from)?.name ?? L("a teammate"), reason)
                : L("Handed off to %@", bot(to)?.name ?? L("a teammate"))
        case let .notice(value): body = value
        case let .permission(request):
            let who = bot(last.author.botID ?? "")?.name ?? L("A bot")
            body = "\(who) \(request.verbPhrase)"
        case let .draft(card):
            body = card.title(botName: bot(last.author.botID ?? "")?.name ?? L("A bot"))
        }

        let flattened = body
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "**", with: "")
            .replacingOccurrences(of: "`", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)

        if chat.isGroup, case let .bot(id) = last.author, case .text = last.body {
            return "\(bot(id)?.name ?? L("Bot")): \(flattened)"
        }
        return flattened
    }

    // MARK: - Connection

    func setConnected(_ connected: Bool) {
        guard isConnected != connected else { return }
        isConnected = connected
        emit(.connectionChanged)
    }

    // MARK: - Requests

    /// Fire-and-forget request. A failure is logged and the store re-syncs from the CLI, so an
    /// optimistic change that the CLI rejected gets rolled back.
    private func perform(_ method: String, _ params: [String: Any] = [:]) {
        guard !isMock else { return }
        Task {
            do {
                _ = try await client.request(method, params)
            } catch {
                NSLog("\(method) failed: \(error.localizedDescription)")
                if let snapshot = try? await client.request("bootstrap", as: Wire.Snapshot.self) {
                    apply(snapshot: snapshot)
                }
            }
        }
    }

    // MARK: - Chat mutation

    private func sortChats() {
        chats.sort { lhs, rhs in
            if lhs.isPinned != rhs.isPinned { return lhs.isPinned }
            if lhs.lastActivity == rhs.lastActivity { return lhs.id < rhs.id }
            return lhs.lastActivity > rhs.lastActivity
        }
    }

    /// The one DM with this bot, created on first use. DMs are keyed by the bot, so opening one
    /// twice lands in the same thread.
    @discardableResult
    func dm(with botID: Bot.ID) -> Chat.ID {
        if let existing = chats.first(where: { $0.isDM && $0.botIDs == [botID] }) {
            return existing.id
        }
        return createChat(kind: .dm, with: [botID], title: nil)
    }

    @discardableResult
    func createChat(kind: Chat.Kind, with botIDs: [Bot.ID], title: String?) -> Chat.ID {
        let botIDs: [Bot.ID] = kind == .dm ? Array(botIDs.prefix(1)) : Array(botIDs.prefix(Chat.maxGroupBots))
        let chat = Chat(
            id: "chat-\(UUID().uuidString.lowercased().prefix(8))",
            kind: kind,
            customTitle: kind == .group ? title : nil,
            botIDs: botIDs,
            messages: [],
            unreadCount: 0,
            isPinned: false,
            createdAt: Date()
        )
        chats.insert(chat, at: 0)
        sortChats()
        emit(.chatsChanged)
        perform(
            "chats.create",
            ["id": chat.id, "kind": kind.rawValue, "bot_ids": botIDs, "title": title ?? ""])
        return chat.id
    }

    @discardableResult
    func createBot(
        name: String,
        description: String = "",
        symbolName: String,
        accent: Accent,
        runnerID: Device.ID,
        provider: ProviderCredential.Kind,
        model: String? = nil,
        thinking: String? = nil,
        templateID: BotTemplate.ID? = nil,
        greeting: String? = nil
    ) -> Bot.ID {
        let bot = Bot(
            id: "bot-\(UUID().uuidString.lowercased().prefix(8))",
            name: name,
            description: description,
            symbolName: symbolName,
            accent: accent,
            runnerID: runnerID,
            provider: provider,
            model: model,
            createdAt: Date()
        )
        bots.append(bot)
        emit(.rosterChanged)
        emit(.chatsChanged)

        // The CLI gives every bot its direct chat; create it under the id the app will open.
        if !isMock {
            let chatID = "chat-\(UUID().uuidString.lowercased().prefix(8))"
            let chat = Chat(
                id: chatID, kind: .dm, customTitle: nil, botIDs: [bot.id], messages: [],
                unreadCount: 0, isPinned: false, createdAt: Date())
            chats.insert(chat, at: 0)
            sortChats()
            emit(.chatsChanged)
            var params: [String: Any] = [
                "id": bot.id, "name": name, "description": description, "symbol_name": symbolName,
                "accent": accent.rawValue, "runner_id": runnerID, "provider": provider.wireValue,
                "model": model ?? "", "thinking": thinking ?? "", "chat_id": chatID,
            ]
            if let templateID {
                params["template_id"] = templateID
                params["greeting"] = greeting ?? ""
            }
            perform("bots.create", params)
        }
        return bot.id
    }

    /// Adds a bot from a marketplace template on `runnerID` and answers with its direct chat.
    /// The CLI gives it the template's routines, paused, and a first turn that answers the
    /// user's greeting: the bot sets itself up there and asks about the rest.
    func addBot(from template: BotTemplate, runnerID: Device.ID) -> Chat.ID {
        let botID = createBot(
            name: template.name, description: template.description, symbolName: template.symbolName,
            accent: template.accent, runnerID: runnerID, provider: preferredProvider, templateID: template.id,
            greeting: L("Hi %@, introduce yourself.", template.name))
        return dm(with: botID)
    }

    /// The Runner returns its new independent bot. Keep that bot and DM visible while a remote
    /// Runner's encrypted roster reaches this Device; subsequent roster events reconcile them.
    func importTemplate(_ params: [String: Any]) async throws -> Chat.ID {
        let reply = try await templateReply("templates.import", params)
        guard let object = reply["bot"] as? [String: Any], let chatID = reply["chat_id"] as? String else {
            throw CLIClient.RequestError(message: L("Couldn't read the imported bot."))
        }
        let data = try JSONSerialization.data(withJSONObject: object)
        let bot = try Wire.decoder.decode(Wire.Bot.self, from: data).toModel()
        if !bots.contains(where: { $0.id == bot.id }) { bots.append(bot) }
        if !chats.contains(where: { $0.id == chatID }) {
            chats.insert(Chat(id: chatID, kind: .dm, customTitle: nil, botIDs: [bot.id], messages: [],
                unreadCount: 0, isPinned: false, createdAt: bot.createdAt), at: 0)
        }
        emit(.rosterChanged)
        emit(.chatsChanged)
        return chatID
    }

    /// The provider a bot made without asking runs with: the first one the account connected.
    var preferredProvider: ProviderCredential.Kind {
        providerKinds.first { credential(for: $0)?.isConnected == true } ?? .deepseek
    }

    /// Every provider a bot can run with: the built-in ones, then the ones the user added,
    /// except those of decision models.
    var providerKinds: [ProviderCredential.Kind] {
        ProviderCredential.Kind.builtIn + providers.filter { $0.kind.isCustom && !$0.decides }.map(\.kind)
    }

    /// The providers Auto-review can run a model of: every one the account has connected.
    var reviewProviderKinds: [ProviderCredential.Kind] {
        providers.filter(\.isConnected).map(\.kind)
    }

    func updateBot(_ id: Bot.ID, name: String, description: String? = nil, provider: ProviderCredential.Kind? = nil) {
        guard let index = bots.firstIndex(where: { $0.id == id }) else { return }
        bots[index].name = name
        if let description { bots[index].description = description }
        if let provider { bots[index].provider = provider }
        emit(.rosterChanged)
        emit(.chatsChanged)
        var params: [String: Any] = ["id": id, "name": name]
        if let description { params["description"] = description }
        if let provider { params["provider"] = provider.wireValue }
        perform("bots.update", params)
    }

    /// The plugins on the bot's Runner and their tools, asked of that Runner.
    func botAccessCatalog(_ id: Bot.ID) async throws -> BotAccessCatalog {
        if isMock { return MockData.accessCatalog() }
        return try await client.request("bots.permissions", ["id": id], as: BotAccessCatalog.self)
    }

    /// Sends a draft card's message: the user's edit first, as the item's next version, then
    /// that version approved, which the bot's Runner sends. `always` then turns drafts off for
    /// the bot, so its next messages go out directly. Throws why it was not sent, with the card
    /// back as it was.
    func sendDraft(_ card: DraftCard, chatID: Chat.ID, messageID: Message.ID, botID: Bot.ID?, always: Bool) async throws {
        let edited = card.shown
        setDraft(chatID: chatID, messageID: messageID) { $0.state = "approved"; $0.fields = edited; $0.edited = nil }
        do {
            if isMock {
                setDraft(chatID: chatID, messageID: messageID) { $0.state = "succeeded" }
            } else {
                var version = card.version
                if edited != card.fields {
                    let data = try await client.request("reviews.edit", ["id": card.reviewID, "expected_version": version, "message": edited.parameters])
                    let item = try Wire.decoder.decode(ReviewItem.self, from: data)
                    upsertReview(item)
                    version = item.version
                }
                let data = try await client.request("reviews.approve", ["id": card.reviewID, "expected_version": version])
                upsertReview(try Wire.decoder.decode(ReviewItem.self, from: data))
            }
        } catch {
            setDraft(chatID: chatID, messageID: messageID) { $0 = card }
            throw error
        }
        if always, let botID { setDraftsDirect(botID) }
    }

    /// Discards a draft card's message: nothing is sent.
    func discardDraft(_ card: DraftCard, chatID: Chat.ID, messageID: Message.ID) async throws {
        setDraft(chatID: chatID, messageID: messageID) { $0.state = "rejected" }
        guard !isMock else { return }
        do {
            let data = try await client.request("reviews.reject", ["id": card.reviewID, "expected_version": card.version])
            upsertReview(try Wire.decoder.decode(ReviewItem.self, from: data))
        } catch {
            setDraft(chatID: chatID, messageID: messageID) { $0 = card }
            throw error
        }
    }

    /// The bot sends its messages directly from now on, as its Access says once drafts are off.
    private func setDraftsDirect(_ id: Bot.ID) {
        guard let index = bots.firstIndex(where: { $0.id == id }) else { return }
        var policy = bots[index].permissions ?? BotPermissions()
        policy.drafts = false
        bots[index].permissions = policy
        emit(.rosterChanged)
        perform("bots.update", ["id": id, "drafts": false])
    }

    /// Keeps what the user changed on a draft card until they send it. Only a change to its
    /// attachments redraws the card; typing never does.
    func editDraft(chatID: Chat.ID, messageID: Message.ID, fields: DraftCard.Fields) {
        guard let chatIndex = chats.firstIndex(where: { $0.id == chatID }), let index = chats[chatIndex].index(of: messageID),
            case var .draft(card) = chats[chatIndex].messages[index].body, card.isPending
        else { return }
        let files = card.shown.attachments
        card.edited = fields == card.fields ? nil : fields
        chats[chatIndex].messages[index].body = .draft(card)
        if files != fields.attachments { emit(.messageChanged(chatID, messageID)) }
    }

    private func setDraft(chatID: Chat.ID, messageID: Message.ID, _ change: (inout DraftCard) -> Void) {
        update(messageID, in: chatID) { message in
            guard case var .draft(card) = message.body else { return }
            change(&card)
            message.body = .draft(card)
        }
    }

    /// Only the user changes a bot's Access; the CLI also dismisses the access requests it left.
    func setBotPermissions(_ id: Bot.ID, _ policy: BotPermissions) {
        guard let index = bots.firstIndex(where: { $0.id == id }) else { return }
        bots[index].permissions = policy
        emit(.rosterChanged)
        perform("bots.update", ["id": id, "permissions": policy.json])
    }

    /// The bot's symbol and accent, the look under and behind its image.
    func setBotLook(_ id: Bot.ID, symbolName: String, accent: Accent) {
        guard let index = bots.firstIndex(where: { $0.id == id }) else { return }
        bots[index].symbolName = symbolName
        bots[index].accent = accent
        emit(.rosterChanged)
        emit(.chatsChanged)
        perform("bots.update", ["id": id, "symbol_name": symbolName, "accent": accent.rawValue])
    }

    /// A custom profile image from a file on this computer (nil removes the current one). The CLI
    /// copies it into its store, uploads it as a `file` blob, and names it in the roster, which
    /// comes back as the bot's `avatar` for every Device.
    func setBotAvatar(_ id: Bot.ID, fileURL: URL?) {
        guard bots.contains(where: { $0.id == id }) else { return }
        if let fileURL {
            let attachment = Attachment(id: "att-\(UUID().uuidString.lowercased().replacingOccurrences(of: "-", with: "").prefix(12))", name: fileURL.lastPathComponent, mime: "image/png", size: 0)
            if let image = NSImage(contentsOf: fileURL) { avatarImages[attachment.id] = image }
            attachmentURLs[attachment.id] = fileURL
            if let index = bots.firstIndex(where: { $0.id == id }) { bots[index].avatar = attachment }
            emit(.rosterChanged)
            emit(.chatsChanged)
            perform("bots.update", ["id": id, "avatar": ["id": attachment.id, "path": fileURL.path, "name": attachment.name, "mime": attachment.mime]])
        } else {
            if let index = bots.firstIndex(where: { $0.id == id }) { bots[index].avatar = nil }
            emit(.rosterChanged)
            emit(.chatsChanged)
            perform("bots.update", ["id": id, "avatar": NSNull()])
        }
    }

    /// Provider, model, and thinking level a bot runs with. nil means the provider's default.
    func setBotRuntime(_ id: Bot.ID, provider: ProviderCredential.Kind, model: String?, thinking: String?) {
        guard let index = bots.firstIndex(where: { $0.id == id }) else { return }
        bots[index].provider = provider
        bots[index].model = model
        bots[index].thinking = thinking
        emit(.rosterChanged)
        emit(.chatsChanged)
        for chat in chats where chat.botIDs.contains(id) { emit(.chatChanged(chat.id)) }
        perform("bots.update", ["id": id, "provider": provider.wireValue, "model": model ?? "", "thinking": thinking ?? ""])
    }

    /// Summarizes the chat's older part for its bot now, on the Runner.
    func compactChat(_ id: Chat.ID) {
        perform("chats.compact", ["chat_id": id])
    }

    // MARK: - Plugins

    /// The marketplace: its plugins, each with the Runners that have it, and its bots.
    func marketplace() async throws -> Marketplace {
        if isMock { return MockData.marketplace() }
        let wire = try await client.request("marketplace", [:], as: Wire.Marketplace.self)
        return Marketplace(packs: wire.packs ?? [], plugins: wire.plugins.map { $0.toModel() }, bots: wire.bots.map { $0.toModel() })
    }

    /// Installs a marketplace plugin on a Runner (here, or sealed to that Runner).
    func installPlugin(_ pluginID: String, on runnerID: Device.ID, accountName: String? = nil) async throws -> InstalledPlugin {
        if isMock {
            let named = accountName != nil
            let status = InstalledPlugin(id: named ? "\(pluginID)-\(UUID().uuidString.replacingOccurrences(of: "-", with: "").lowercased())" : pluginID,
                                         name: accountName.map { "\(pluginID) · \($0)" } ?? pluginID, description: "", version: "", icon: "", state: .ready, detail: "Connected",
                                         serviceID: named ? pluginID : nil, accountName: accountName)
            if let index = devices.firstIndex(where: { $0.id == runnerID }) { devices[index].plugins.append(status); emit(.rosterChanged) }
            return status
        }
        var params: [String: Any] = ["runner_id": runnerID, "plugin_id": pluginID]
        if let accountName { params["account_name"] = accountName }
        let status = try await client.request("plugins.install", params, as: Wire.PluginInstalled.self).status.toModel()
        rememberPlugin(status, on: runnerID)
        return status
    }

    func renamePluginAccount(_ pluginID: String, on runnerID: Device.ID, accountName: String) async throws -> InstalledPlugin {
        if isMock, var status = device(runnerID)?.plugins.first(where: { $0.id == pluginID }) {
            status.name = "\(status.name.components(separatedBy: " · ").first ?? status.name) · \(accountName)"
            status.accountName = accountName
            rememberPlugin(status, on: runnerID)
            return status
        }
        let status = try await client.request("plugins.rename", ["runner_id": runnerID, "plugin_id": pluginID, "account_name": accountName], as: Wire.PluginInstalled.self).status.toModel()
        rememberPlugin(status, on: runnerID)
        return status
    }

    /// A sealed management reply can arrive before the Runner's next machine advertisement.
    private func rememberPlugin(_ status: InstalledPlugin, on runnerID: Device.ID) {
        guard let index = devices.firstIndex(where: { $0.id == runnerID }) else { return }
        if let plugin = devices[index].plugins.firstIndex(where: { $0.id == status.id }) { devices[index].plugins[plugin] = status }
        else { devices[index].plugins.append(status) }
        emit(.rosterChanged)
    }

    func uninstallPlugin(_ pluginID: String, on runnerID: Device.ID) async throws {
        if !isMock { _ = try await client.request("plugins.uninstall", ["runner_id": runnerID, "plugin_id": pluginID]) }
        if let index = devices.firstIndex(where: { $0.id == runnerID }) { devices[index].plugins.removeAll { $0.id == pluginID }; emit(.rosterChanged) }
    }

    func pluginDetail(_ pluginID: String, on runnerID: Device.ID) async throws -> PluginDetail {
        if isMock, pluginID == BrowserProfile.pluginID, let status = device(runnerID)?.plugins.first(where: { $0.id == pluginID }) {
            // Browser runs on the Runner and signs in to nothing itself.
            return PluginDetail(status: status, homepage: "https://github.com/microsoft/playwright-mcp", variables: [], servers: [], skills: [(name: "Reading a page", description: "How to read a page without filling the context.")])
        }
        if isMock {
            let status = device(runnerID)?.plugins.first { $0.id == pluginID } ?? InstalledPlugin(id: pluginID, name: pluginID, description: "", version: "", icon: "", state: .ready, detail: "Ready")
            return PluginDetail(status: status, homepage: nil, variables: [.init(name: "GITHUB_TOKEN", description: "A personal access token, instead of signing in.", secret: true, required: false, isSet: false, value: nil)], servers: [.init(name: "github", kind: "http", url: "https://api.githubcopilot.com/mcp/", oauth: true, signedIn: status.state == .ready)], skills: [])
        }
        return try await client.request("plugins.detail", ["runner_id": runnerID, "plugin_id": pluginID], as: Wire.PluginDetail.self).toModel()
    }

    /// Sets variables on the Runner; a secret goes out in the request and is never read back.
    func setPluginVariables(_ pluginID: String, on runnerID: Device.ID, variables: [String: String]) async throws -> InstalledPlugin {
        guard !isMock else { return InstalledPlugin(id: pluginID, name: pluginID, description: "", version: "", icon: "", state: .ready, detail: "Ready") }
        return try await client.request("plugins.set_variables", ["runner_id": runnerID, "plugin_id": pluginID, "variables": variables], as: Wire.PluginInstalled.self).status.toModel()
    }

    /// Starts a plugin's sign-in for the Runner; the browser opens on this Mac.
    func connectPlugin(_ pluginID: String, on runnerID: Device.ID) async throws {
        guard !isMock else { return }
        _ = try await client.request("plugins.connect", ["runner_id": runnerID, "plugin_id": pluginID])
    }

    /// Forgets a plugin server's sign-in on its Runner. Nothing is revoked at the server; the
    /// plugin's next use asks for a sign-in again.
    func signOutPlugin(_ pluginID: String, server: String, on runnerID: Device.ID) async throws {
        guard !isMock else { return }
        _ = try await client.request("plugins.sign_out", ["runner_id": runnerID, "plugin_id": pluginID, "server": server])
    }

    // MARK: - Limits

    func budget(_ kind: String, _ id: String, runnerID: Device.ID) -> BudgetState? {
        budgets.first { $0.kind == kind && $0.id == id && $0.runnerId == runnerID }
    }

    /// The DM's newest turn when it stopped at a limit or was interrupted: the one to resume.
    func stoppedTurn(in chatID: Chat.ID, runnerID: Device.ID) -> BudgetState? {
        let newest = budgets
            .filter { $0.kind == "job" && $0.chatId == chatID && $0.runnerId == runnerID }
            .max { $0.updatedAt < $1.updatedAt }
        return newest?.isStopped == true ? newest : nil
    }

    /// Sets limits on the bot's Runner. What the work used stays.
    func setBudget(_ kind: String, _ id: String, limits: BudgetLimits, bot: Bot, chatID: Chat.ID) async throws {
        if isMock {
            let index = budgets.firstIndex { $0.kind == kind && $0.id == id }
            var state = index.map { budgets[$0] } ?? BudgetState(
                kind: kind, id: id, runnerId: bot.runnerID, chatId: chatID, limits: BudgetLimits(),
                usage: .init(tokens: 0, apiCostUsd: 0, subscriptionEstimateUsd: 0, unknownPriceCalls: 0, runtimeSecs: 0, retries: 0, connectorCalls: 0),
                state: "ready", updatedAt: Date().timeIntervalSince1970)
            state.limits = limits
            if let index { budgets[index] = state } else { budgets.append(state) }
            emit(.budgetsChanged)
            return
        }
        let params: [String: Any] = ["kind": kind, "id": id, "bot_id": bot.id, "chat_id": chatID, "runner_id": bot.runnerID, "limits": limits.params]
        _ = try await client.request("budgets.set", params)
    }

    /// Resumes a stopped turn or routine where it left off. `fresh` grants the whole allowance
    /// again; otherwise it goes on with what it used counted against the limits.
    func resumeBudget(_ kind: String, _ id: String, bot: Bot, fresh: Bool) async throws {
        if isMock {
            guard let index = budgets.firstIndex(where: { $0.kind == kind && $0.id == id }) else { return }
            budgets[index].state = "ready"
            budgets[index].reached = nil
            if fresh { budgets[index].usage = .init(tokens: 0, apiCostUsd: 0, subscriptionEstimateUsd: 0, unknownPriceCalls: 0, runtimeSecs: 0, retries: 0, connectorCalls: 0) }
            emit(.budgetsChanged)
            return
        }
        // The request id makes a repeated delivery a no-op on the Runner.
        let params: [String: Any] = ["kind": kind, "id": id, "runner_id": bot.runnerID, "renew": fresh, "run": true, "request_id": UUID().uuidString]
        _ = try await client.request("budgets.resume", params)
    }

    /// A plugin account's call limit, or with `service` the one all of its service's accounts share.
    func callLimits(_ pluginID: String, on runnerID: Device.ID, service: Bool = false) async throws -> CallLimits {
        if isMock {
            return CallLimits(maxCalls: 60, windowSecs: 60, maxConcurrency: 4, retryAt: pluginID == "github" ? Date().addingTimeInterval(4 * 60) : nil, sharesService: false)
        }
        let params: [String: Any] = ["runner_id": runnerID, "plugin_id": pluginID, "scope": service ? "service" : "account"]
        return try await client.request("connector_limits.get", params, as: Wire.CallLimits.self).toModel()
    }

    func setCallLimits(_ pluginID: String, on runnerID: Device.ID, service: Bool, limits: CallLimits) async throws {
        guard !isMock else { return }
        let params: [String: Any] = [
            "runner_id": runnerID, "plugin_id": pluginID, "scope": service ? "service" : "account",
            "limits": ["max_calls": limits.maxCalls, "window_secs": limits.windowSecs, "max_concurrency": limits.maxConcurrency],
        ]
        _ = try await client.request("connector_limits.set", params)
    }

    // MARK: - MCP servers

    /// The demo's mcp.json files, by Runner.
    private var mockMcp: [Device.ID: [McpServer]] = [:]

    private func mockMcpServers(_ runnerID: Device.ID) -> [McpServer] {
        if let servers = mockMcp[runnerID] { return servers }
        let servers = runnerID == "dev-workbench" ? MockData.mcpServers() : []
        mockMcp[runnerID] = servers
        return servers
    }

    /// The demo's Runner takes its servers as the CLI would: the ones that run are its plugins.
    private func setMockMcpServers(_ servers: [McpServer], on runnerID: Device.ID) {
        mockMcp[runnerID] = servers
        guard let index = devices.firstIndex(where: { $0.id == runnerID }) else { return }
        devices[index].plugins = devices[index].plugins.filter { !$0.isMcpServer } + servers.compactMap { $0.isEnabled ? $0.status : nil }
        emit(.rosterChanged)
    }

    /// An `mcp.*` reply as JSON objects. Not through `Wire.decoder`, whose snake-case keys would
    /// rename the variables and headers of an entry.
    private func mcpReply(_ method: String, _ params: [String: Any]) async throws -> [String: Any] {
        let data = try await client.request(method, params)
        return (try JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }

    private func mcpServer(in reply: [String: Any]) throws -> McpServer {
        guard let server = (reply["server"] as? [String: Any]).flatMap(McpServer.init(json:)) else {
            throw CLIClient.RequestError(message: L("Request failed"))
        }
        return server
    }

    /// A Runner's mcp.json: every server in it, usable or not, here or sealed to that Runner.
    func mcpServers(on runnerID: Device.ID) async throws -> McpFile {
        if isMock { return McpFile(path: "~/.lorca/mcp.json", servers: mockMcpServers(runnerID)) }
        return McpFile(json: try await mcpReply("mcp.list", ["runner_id": runnerID]))
    }

    /// One server with the tools it offered when it last connected.
    func mcpServer(_ name: String, on runnerID: Device.ID) async throws -> McpServer {
        if isMock {
            guard let server = mockMcpServers(runnerID).first(where: { $0.name == name }) else {
                throw CLIClient.RequestError(message: L("No server named %@ in mcp.json.", name))
            }
            return server
        }
        return try mcpServer(in: try await mcpReply("mcp.get", ["runner_id": runnerID, "name": name]))
    }

    /// Adds a server, or saves the one `previousName` names under `name`. The CLI checks the
    /// entry and writes it to the Runner's mcp.json, where the server starts once to list its tools.
    func saveMcpServer(_ name: String, entry: McpEntry, previousName: String? = nil, on runnerID: Device.ID) async throws -> McpServer {
        if isMock {
            var servers = mockMcpServers(runnerID)
            if previousName != name, servers.contains(where: { $0.name == name }) {
                throw CLIClient.RequestError(message: L("mcp.json already has a server named %@.", name))
            }
            let id = name.lowercased().components(separatedBy: CharacterSet.alphanumerics.inverted).filter { !$0.isEmpty }.joined(separator: "-")
            let status = InstalledPlugin(
                id: id, name: name, description: entry.about, version: "", icon: entry.symbolName, state: .ready, detail: "Ready", source: "mcp.json")
            let tools = [McpTool(name: "echo", about: "Echo back what you send.", isReadOnly: true), McpTool(name: "write_note", about: "Write a note.", isReadOnly: false)]
            let server = McpServer(name: name, id: id, isEnabled: !entry.isDisabled, entry: entry, status: entry.isDisabled ? nil : status, toolCount: tools.count, tools: tools)
            if let index = servers.firstIndex(where: { $0.name == (previousName ?? name) }) {
                servers[index] = server
            } else {
                servers.append(server)
            }
            setMockMcpServers(servers, on: runnerID)
            return server
        }
        var params: [String: Any] = ["runner_id": runnerID, "name": name, "config": entry.fields]
        if let previousName { params["previous_name"] = previousName }
        return try mcpServer(in: try await mcpReply("mcp.save", params))
    }

    /// Removes a server from the Runner's mcp.json, with its sign-in.
    func removeMcpServer(_ name: String, on runnerID: Device.ID) async throws {
        if isMock {
            setMockMcpServers(mockMcpServers(runnerID).filter { $0.name != name }, on: runnerID)
            return
        }
        _ = try await client.request("mcp.remove", ["runner_id": runnerID, "name": name])
    }

    /// Turns a server on or off; off, no bot sees it and it never starts.
    func setMcpServer(_ name: String, enabled: Bool, on runnerID: Device.ID) async throws -> McpServer {
        if isMock {
            var servers = mockMcpServers(runnerID)
            guard let index = servers.firstIndex(where: { $0.name == name }) else {
                throw CLIClient.RequestError(message: L("No server named %@ in mcp.json.", name))
            }
            var server = servers[index]
            server.isEnabled = enabled
            if enabled {
                server.entry.fields.removeValue(forKey: "disabled")
            } else {
                server.entry.fields["disabled"] = true
            }
            if enabled, server.status == nil {
                server.status = InstalledPlugin(
                    id: server.id, name: name, description: server.entry.about, version: "", icon: server.entry.symbolName, state: .ready,
                    detail: "Ready", source: "mcp.json")
            }
            servers[index] = server
            setMockMcpServers(servers, on: runnerID)
            return server
        }
        return try mcpServer(in: try await mcpReply("mcp.set_enabled", ["runner_id": runnerID, "name": name, "enabled": enabled]))
    }

    /// Connects a server and waits for it: from scratch (`fresh`), or taking a connection it has
    /// or one under way. Its state says how it went.
    /// Has a Runner read its mcp.json again, after an edit made outside Lorca, and answers the file
    /// as it reads now.
    func reloadMcpFile(on runnerID: Device.ID) async throws -> McpFile {
        if isMock { return try await mcpServers(on: runnerID) }
        return McpFile(json: try await mcpReply("mcp.reload", ["runner_id": runnerID]))
    }

    /// Offers one of a server's tools to bots, or keeps it from them, in the Runner's mcp.json. The
    /// server keeps its connection.
    func setMcpTool(_ tool: String, shown: Bool, server name: String, on runnerID: Device.ID) async throws -> McpServer {
        if isMock {
            var servers = mockMcpServers(runnerID)
            guard let index = servers.firstIndex(where: { $0.name == name }) else {
                throw CLIClient.RequestError(message: L("No server named %@ in mcp.json.", name))
            }
            if let toolIndex = servers[index].tools?.firstIndex(where: { $0.name == tool }) {
                servers[index].tools?[toolIndex].isHidden = !shown
            }
            setMockMcpServers(servers, on: runnerID)
            return servers[index]
        }
        return try mcpServer(in: try await mcpReply("mcp.hide_tool", ["runner_id": runnerID, "name": name, "tool": tool, "hidden": !shown]))
    }

    /// Forgets a remote server's sign-in on its Runner. Nothing is revoked at the server; the
    /// server's next use asks for a sign-in again.
    func signOutMcpServer(_ name: String, on runnerID: Device.ID) async throws -> McpServer {
        if isMock {
            var servers = mockMcpServers(runnerID)
            guard let index = servers.firstIndex(where: { $0.name == name }) else {
                throw CLIClient.RequestError(message: L("No server named %@ in mcp.json.", name))
            }
            var server = servers[index]
            server.isSignedIn = false
            if var status = server.status {
                status.state = .needsAuth
                status.detail = "Sign in"
                server.status = status
            }
            servers[index] = server
            setMockMcpServers(servers, on: runnerID)
            return server
        }
        return try mcpServer(in: try await mcpReply("mcp.sign_out", ["runner_id": runnerID, "name": name]))
    }

    func reconnectMcpServer(_ name: String, fresh: Bool, on runnerID: Device.ID) async throws -> McpServer {
        if isMock {
            try await Task.sleep(nanoseconds: 900_000_000)
            return try await mcpServer(name, on: runnerID)
        }
        return try mcpServer(in: try await mcpReply("mcp.reconnect", ["runner_id": runnerID, "name": name, "fresh": fresh]))
    }

    /// The servers pasted JSON holds, in any app's spelling; the CLI on this Mac reads it.
    func parseMcpJSON(_ text: String) async throws -> [ParsedServer] {
        let reply: [String: Any]
        if isMock {
            guard let object = try? JSONSerialization.jsonObject(with: Data(text.utf8)) as? [String: Any] else {
                throw CLIClient.RequestError(message: L("Not a server's JSON: expected an object."))
            }
            let servers: [String: Any] = (object["mcpServers"] as? [String: Any]) ?? (object["servers"] as? [String: Any]) ?? [:]
            var entries: [[String: Any]] = []
            if object["command"] != nil || object["url"] != nil {
                entries = [["config": object]]
            } else {
                for key in servers.keys.sorted() {
                    entries.append(["name": key, "config": servers[key] ?? [String: Any]()])
                }
            }
            reply = ["servers": entries]
        } else {
            reply = try await mcpReply("mcp.parse", ["text": text])
        }
        return (reply["servers"] as? [[String: Any]] ?? []).map { server in
            ParsedServer(
                name: server["name"] as? String, entry: (server["config"] as? [String: Any]).map { McpEntry($0) }, problem: server["problem"] as? String)
        }
    }

    /// Replaces Auto-review (the switch and the rules); the change shows at once and the CLI's
    /// roster event confirms it. A new rule gets its id from the CLI.
    func setAutoReview(_ value: AutoReview) {
        autoReview = value
        emit(.rosterChanged)
        let rules: [[String: Any]] = value.rules.map { rule in
            var row: [String: Any] = ["id": rule.id, "text": rule.text, "behavior": rule.behavior.rawValue]
            if let tool = rule.tool { row["tool"] = tool }
            return row
        }
        perform("auto_review.set", ["is_enabled": value.isEnabled, "rules": rules])
    }

    /// Picks the provider that reviews, or nil for the bot's own.
    func setReviewProvider(_ provider: ProviderCredential.Kind?) {
        autoReview.provider = provider
        emit(.rosterChanged)
        perform("auto_review.set", ["provider": provider?.wireValue ?? NSNull()])
    }

    /// Picks the model Auto-review runs on `kind`, or nil for its default review model.
    func setReviewModel(_ model: String?, for kind: ProviderCredential.Kind) {
        autoReview.models[kind] = model
        emit(.rosterChanged)
        perform("auto_review.set", ["models": [kind.wireValue: model ?? NSNull()] as [String: Any]])
    }

    /// Answers a question: a permission card's, or a command card's. `allow`, `always`, or
    /// `deny`. The CLI confirms with the card's new state.
    func answerPermission(chatID: Chat.ID, messageID: Message.ID, decision: String) {
        update(messageID, in: chatID) { message in
            switch message.body {
            case var .permission(request):
                request.decision = decision == "always" ? .always : (decision == "deny" ? .denied : .allowed)
                // An access request is only ever dismissed.
                if request.isAccess, request.decision == .denied { request.decision = .dismissed }
                if request.isConnect, request.decision == .allowed { request.summary = L("Starting the sign-in…") }
                message.body = .permission(request)
            case var .tool(tool):
                guard var run = tool.run, run.state == .asking else { return }
                run.state = decision == "deny" ? .denied : .running
                tool.run = run
                message.body = .tool(tool)
            default:
                return
            }
        }
        perform("chats.permission", ["chat_id": chatID, "message_id": messageID, "decision": decision])
    }

    // MARK: - Commands

    /// Types the user's answer into a command running in its terminal, then Return.
    /// The CLI writes it to the terminal, or seals it to the bot's Runner, and keeps nothing.
    /// Throws why it could not, such as that Runner being offline.
    func answerCommand(chatID: Chat.ID, messageID: Message.ID, text: String) async throws {
        guard !isMock else {
            finishMockCommand(chatID: chatID, messageID: messageID, state: .exited)
            return
        }
        _ = try await client.request("bash.stdin", ["chat_id": chatID, "message_id": messageID, "text": text])
    }

    /// Stops a running command; its card leaves once the Runner has.
    func stopCommand(chatID: Chat.ID, messageID: Message.ID) async throws {
        guard !isMock else {
            finishMockCommand(chatID: chatID, messageID: messageID, state: .stopped)
            return
        }
        _ = try await client.request("bash.stop", ["chat_id": chatID, "message_id": messageID])
    }

    /// Sends a command the bot is waiting on to the background (`bash.background`): the bot's
    /// call returns and the command runs on, out of the way of Stop in the chat.
    func sendCommandToBackground(chatID: Chat.ID, messageID: Message.ID) async throws {
        guard !isMock else {
            update(messageID, in: chatID) { message in
                guard case var .tool(tool) = message.body, var run = tool.run else { return }
                run.background = true
                tool.run = run
                tool.isRunning = false
                message.body = .tool(tool)
            }
            return
        }
        _ = try await client.request("bash.background", ["chat_id": chatID, "message_id": messageID])
    }

    /// The demo has no Runner: an answer or a Stop ends the command at once.
    private func finishMockCommand(chatID: Chat.ID, messageID: Message.ID, state: CommandRun.State) {
        update(messageID, in: chatID) { message in
            guard case var .tool(tool) = message.body, var run = tool.run else { return }
            run.state = state
            run.prompt = nil
            tool.run = run
            message.body = .tool(tool)
        }
    }

    // MARK: - Routines

    func review(_ id: String) -> ReviewItem? { reviews.first { $0.id == id } }

    private func upsertReview(_ item: ReviewItem) {
        if let index = reviews.firstIndex(where: { $0.id == item.id }) {
            guard reviews[index].revision <= item.revision else { return }
            reviews[index] = item
        } else { reviews.append(item) }
        emit(.reviewsChanged)
    }

    /// The plugin a call goes to, as its Runner lists it and the permission card names it: "GitHub".
    func pluginName(of item: ReviewItem) -> String {
        device(item.runnerId)?.plugins.first { $0.id == item.payload.pluginId }?.name ?? item.target.account
    }

    /// Approves the version the user saw. An edit made in the sheet is saved first, as the next
    /// version, and that is the one approved: what runs is what the editor showed.
    func approveReview(_ item: ReviewItem, payload: [String: Any]?) async throws -> ReviewItem {
        var shown = item
        if let payload { shown = try await changeReview(shown, action: "edit", fields: ["payload": payload]) }
        return try await changeReview(shown, action: "approve")
    }

    func rejectReview(_ item: ReviewItem) async throws -> ReviewItem {
        try await changeReview(item, action: "reject")
    }

    /// A change names the version the sheet displayed, so one made on another Device meanwhile is
    /// refused rather than decided blind.
    private func changeReview(_ item: ReviewItem, action: String, fields: [String: Any] = [:]) async throws -> ReviewItem {
        var params = fields
        params["id"] = item.id
        params["expected_version"] = item.version
        let data: Data
        if isMock {
            data = try MockData.changedReview(item, action: action, fields: fields)
        } else {
            data = try await client.request("reviews.\(action)", params)
        }
        let updated = try Wire.decoder.decode(ReviewItem.self, from: data)
        upsertReview(updated)
        return review(updated.id) ?? updated
    }

    /// Pauses or resumes a routine. A resumed schedule counts from now.
    func setRoutineEnabled(_ id: Routine.ID, _ enabled: Bool) {
        guard let index = routines.firstIndex(where: { $0.id == id }) else { return }
        routines[index].isEnabled = enabled
        routines[index].pausedReason = nil
        routines[index].state = enabled ? "on" : "paused"
        if !enabled { routines[index].nextRunAt = nil }
        emit(.rosterChanged)
        perform("routines.update", ["id": id, "enabled": enabled])
    }

    /// Runs the routine now, on its bot's Runner.
    func runRoutine(_ id: Routine.ID) {
        guard let index = routines.firstIndex(where: { $0.id == id }) else { return }
        routines[index].isRunning = true
        emit(.rosterChanged)
        perform("routines.run", ["id": id])
    }

    func deleteRoutine(_ id: Routine.ID) {
        routines.removeAll { $0.id == id }
        emit(.rosterChanged)
        perform("routines.delete", ["id": id])
    }

    // MARK: - Workflow feedback

    /// A bot's notes, suggestions, and changes, from its Runner, through the relay when that is
    /// another Device. Every decision is the Runner's to apply.
    func feedback(of botID: Bot.ID) async throws -> BotFeedback {
        if isMock { return mockFeedback[botID] ?? MockData.feedback(for: botID) }
        return try await client.request("feedback.list", ["bot_id": botID], as: Wire.Feedback.self).toModel()
    }

    /// Records what the user said about a message of the bot's.
    func recordFeedback(
        botID: Bot.ID, kind: FeedbackNote.Kind, chatID: Chat.ID, messageID: Message.ID, note: String,
        before: String?, after: String?, target: FeedbackTarget?
    ) async throws {
        var feedback: [String: Any] = ["kind": kind.rawValue, "origin": ["chat_id": chatID, "message_id": messageID], "note": note]
        if let before, let after {
            feedback["before"] = before
            feedback["after"] = after
        }
        if let target { feedback["target"] = target.params }
        try await feedbackRequest("feedback.record", botID: botID, ["feedback": feedback]) { mock in
            let text = note.isEmpty ? (self.chat(chatID)?.messages.first { $0.id == messageID }?.text ?? "") : note
            mock.notes.insert(FeedbackNote(id: UUID().uuidString, kind: kind, chatID: chatID, messageID: messageID, text: text, target: target, createdAt: Date()), at: 0)
            mock.noteCount += 1
        }
    }

    func decide(_ suggestion: FeedbackSuggestion, accept: Bool, botID: Bot.ID) async throws {
        try await feedbackRequest(accept ? "feedback.accept" : "feedback.reject", botID: botID, ["id": suggestion.id, "diff_hash": suggestion.diffHash]) { mock in
            mock.suggestions.removeAll { $0.id == suggestion.id }
            if accept {
                mock.changes.insert(FeedbackChange(id: UUID().uuidString, target: suggestion.target, diff: suggestion.diff, isUndo: false, canUndo: true, currentHash: "mock", createdAt: Date()), at: 0)
            }
        }
    }

    func undo(_ change: FeedbackChange, botID: Bot.ID) async throws {
        try await feedbackRequest("feedback.rollback", botID: botID, ["id": change.id, "expected_hash": change.currentHash ?? ""]) { mock in
            let reversed = DiffLine.lines(of: change.diff).map { line -> String in
                switch line {
                case let .removed(text): "+" + text
                case let .added(text): "-" + text
                }
            }
            mock.changes = mock.changes.map { FeedbackChange(id: $0.id, target: $0.target, diff: $0.diff, isUndo: $0.isUndo, canUndo: false, currentHash: $0.currentHash, createdAt: $0.createdAt) }
            mock.changes.insert(FeedbackChange(id: UUID().uuidString, target: change.target, diff: reversed.joined(separator: "\n"), isUndo: true, canUndo: true, currentHash: "mock", createdAt: Date()), at: 0)
        }
    }

    /// How often the bot looks through new feedback for changes to suggest; nil turns it off.
    func setFeedbackReview(every seconds: Int?, botID: Bot.ID) async throws {
        try await feedbackRequest("feedback.settings", botID: botID, ["review_every_secs": seconds.map { $0 as Any } ?? NSNull()]) { mock in
            mock.reviewEvery = seconds
        }
    }

    /// Takes a note's words out of the bot's feedback, or every note from a chat, now and later.
    func exclude(note: FeedbackNote, wholeChat: Bool, botID: Bot.ID) async throws {
        try await feedbackRequest("feedback.exclude", botID: botID, wholeChat ? ["chat_id": note.chatID] : ["id": note.id]) { mock in
            mock.notes.removeAll { wholeChat ? $0.chatID == note.chatID : $0.id == note.id }
            mock.noteCount = mock.notes.count
        }
    }

    /// Has the bot look through its new feedback now; answers how many changes it suggests.
    func suggestChanges(botID: Bot.ID) async throws -> Int {
        if isMock {
            try await Task.sleep(for: .seconds(1))
            return 0
        }
        let review = try await client.request("feedback.review", ["bot_id": botID], as: Wire.FeedbackReview.self)
        emit(.feedbackChanged(botID))
        return review.proposals.count
    }

    /// The demo's feedback, changed in place by the same calls.
    private var mockFeedback: [Bot.ID: BotFeedback] = [:]

    /// A CLI call that changes a bot's feedback. The Runner announces its own bots' changes; a bot
    /// on another Device answers only the call, so the change is announced here too.
    private func feedbackRequest(_ method: String, botID: Bot.ID, _ params: [String: Any], mock: (inout BotFeedback) -> Void) async throws {
        if isMock {
            var feedback = mockFeedback[botID] ?? MockData.feedback(for: botID)
            mock(&feedback)
            mockFeedback[botID] = feedback
        } else {
            var params = params
            params["bot_id"] = botID
            _ = try await client.request(method, params)
        }
        emit(.feedbackChanged(botID))
    }

    /// A bot's memory, read from its Runner. Answers with `here == false` when the bot runs on
    /// another Device, whose disk this computer cannot read.
    func botMemory(_ id: Bot.ID) async throws -> BotMemory {
        if isMock {
            return BotMemory(
                botID: id, here: true, runner: "This computer", path: "~/.lorca/workspaces/\(id)",
                text: "- 2026-09-10 · from your chat with the user · the user prefers short replies\n- 2026-09-12 · invoices are reconciled on Mondays\n",
                hash: "mock", lines: 2, bytes: 128, truncated: false, maxLines: 200, maxBytes: 24_000,
                topics: ["clients.md"], logs: ["2026-09-12", "2026-09-15"])
        }
        return try await client.request("bots.memory", ["bot_id": id], as: Wire.BotMemory.self).toModel()
    }

    /// Replaces the bot's MEMORY.md. With `expectedHash`, the write is refused when the file
    /// changed since it was read, so an edit never silently overwrites what the bot wrote.
    func writeBotMemory(_ id: Bot.ID, text: String, expectedHash: String?) async throws -> String {
        guard !isMock else { return "mock" }
        var params: [String: Any] = ["bot_id": id, "text": text]
        if let expectedHash { params["expected_hash"] = expectedHash }
        return try await client.request("bots.memory.write", params, as: Wire.MemoryWritten.self).hash
    }

    /// "Retrying (2 of 3) in 4 s", while a turn's model call waits to be asked again.
    private var retryNotes: [Chat.ID: String] = [:]

    func retryNote(for chatID: Chat.ID) -> String? {
        retryNotes[chatID]
    }

    /// The bot whose model is reasoning in a chat, until its next message or the end of its turn.
    private var thinkingBots: [Chat.ID: Bot.ID] = [:]

    func isThinking(_ botID: Bot.ID, in chatID: Chat.ID) -> Bool {
        thinkingBots[chatID] == botID
    }

    func deleteChat(_ id: Chat.ID) {
        guard let chat = chat(id) else { return }

        // A bot owns its DM, so deleting that row deletes the bot as one roster operation.
        // Groups keep their other members; a group with nobody left is removed too.
        if chat.isDM, let botID = chat.botIDs.first, bot(botID) != nil {
            let relatedChatIDs = chats.filter { $0.botIDs.contains(botID) }.map(\.id)
            for chatID in relatedChatIDs { replyEngine?.cancel(chatID: chatID) }

            var removedChatIDs = Set<Chat.ID>()
            var changedChatIDs: [Chat.ID] = []
            chats = chats.compactMap { existing in
                guard existing.botIDs.contains(botID) else { return existing }
                if existing.isDM {
                    removedChatIDs.insert(existing.id)
                    return nil
                }
                var updated = existing
                updated.botIDs.removeAll { $0 == botID }
                guard !updated.botIDs.isEmpty else {
                    removedChatIDs.insert(existing.id)
                    return nil
                }
                changedChatIDs.append(existing.id)
                return updated
            }

            bots.removeAll { $0.id == botID }
            routines.removeAll { $0.botID == botID }
            let cancelledJobs = runningJobs.filter {
                $0.botID == botID || removedChatIDs.contains($0.chatID)
            }
            let cancelledJobIDs = Set(cancelledJobs.map(\.id))
            runningJobs.removeAll { cancelledJobIDs.contains($0.id) }
            for job in cancelledJobs { jobStarts.removeValue(forKey: job.id) }
            for chatID in removedChatIDs { retryNotes.removeValue(forKey: chatID) }
            thinkingBots = thinkingBots.filter { $0.value != botID && !removedChatIDs.contains($0.key) }

            emit(.rosterChanged)
            for chatID in changedChatIDs { emit(.chatChanged(chatID)) }
            emit(.chatsChanged)
            perform("bots.delete", ["id": botID])
            return
        }

        replyEngine?.cancel(chatID: id)
        chats.removeAll { $0.id == id }
        let cancelledJobs = runningJobs.filter { $0.chatID == id }
        runningJobs.removeAll { $0.chatID == id }
        for job in cancelledJobs { jobStarts.removeValue(forKey: job.id) }
        retryNotes.removeValue(forKey: id)
        thinkingBots.removeValue(forKey: id)
        emit(.chatsChanged)
        perform("chats.delete", ["chat_id": id])
    }

    func togglePin(_ id: Chat.ID) {
        guard let index = chats.firstIndex(where: { $0.id == id }) else { return }
        chats[index].isPinned.toggle()
        sortChats()
        emit(.chatsChanged)
        perform("chats.pin", ["chat_id": id, "pinned": chats.first { $0.id == id }?.isPinned ?? false])
    }

    /// Asks the CLI for the page of messages before the chat's first one. The transcript calls
    /// this as it nears the top; one request per chat at a time.
    func loadOlderMessages(in id: Chat.ID) {
        guard !isMock, !loadingOlder.contains(id), let chat = chat(id), chat.hasMore, let first = chat.messages.first else { return }
        loadingOlder.insert(id)
        Task {
            defer { loadingOlder.remove(id) }
            guard let page = try? await client.request("chats.messages", ["chat_id": id, "before": first.id], as: Wire.MessagePage.self),
                let index = chats.firstIndex(where: { $0.id == id }), chats[index].messages.first?.id == first.id
            else { return }
            let known = Set(chats[index].messages.map(\.id))
            chats[index].messages.insert(contentsOf: page.messages.map { $0.toModel() }.filter { !known.contains($0.id) }, at: 0)
            chats[index].hasMore = page.hasMore
            emit(.olderMessagesLoaded(id))
        }
    }

    /// Loads older pages of a chat until `messageID` is in it, so a note can show the message it
    /// is about. False when the chat has no such message.
    func loadMessage(_ messageID: Message.ID, in id: Chat.ID) async throws -> Bool {
        for _ in 0..<100 {
            guard let chat = chat(id) else { return false }
            if chat.messages.contains(where: { $0.id == messageID }) { return true }
            guard !isMock, chat.hasMore, let first = chat.messages.first else { return false }
            let page = try await client.request("chats.messages", ["chat_id": id, "before": first.id], as: Wire.MessagePage.self)
            guard let index = chats.firstIndex(where: { $0.id == id }) else { return false }
            if chats[index].messages.first?.id != first.id { continue }
            let known = Set(chats[index].messages.map(\.id))
            chats[index].messages.insert(contentsOf: page.messages.map { $0.toModel() }.filter { !known.contains($0.id) }, at: 0)
            chats[index].hasMore = page.hasMore
            emit(.olderMessagesLoaded(id))
            if page.messages.isEmpty { return false }
        }
        return false
    }

    /// Full-text chat and message matches from the local SQLite index.
    func searchChats(_ query: String) async throws -> Wire.SearchResults {
        guard !isMock else { return .empty }
        return try await client.request("chats.search", ["query": query, "limit": 24], as: Wire.SearchResults.self)
    }

    /// Tells the CLI which chat the user is looking at (nil when none, or the app is not
    /// frontmost), so a reply they watch arrive is not pushed to their phone.
    func setWatchedChat(_ id: Chat.ID?) {
        guard !isMock, isConnected, reportedWatchedChat != .some(id) else { return }
        reportedWatchedChat = .some(id)
        Task { _ = try? await client.request("ui.watching", ["chat_id": id ?? NSNull()]) }
    }

    func markRead(_ id: Chat.ID) {
        guard let index = chats.firstIndex(where: { $0.id == id }), chats[index].unreadCount > 0
        else { return }
        chats[index].unreadCount = 0
        emit(.chatsChanged)
        perform("chats.mark_read", ["chat_id": id])
    }

    func rename(_ id: Chat.ID, to title: String) {
        guard let index = chats.firstIndex(where: { $0.id == id }), chats[index].isGroup else { return }
        let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
        chats[index].customTitle = trimmed
        emit(.chatChanged(id))
        emit(.chatsChanged)
        perform("chats.rename", ["chat_id": id, "title": trimmed])
    }

    /// What a group is for; every member reads it in its system prompt.
    func setDescription(_ text: String, of chatID: Chat.ID) {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard let index = chats.firstIndex(where: { $0.id == chatID }), chats[index].isGroup,
            chats[index].groupDescription != trimmed
        else { return }
        chats[index].groupDescription = trimmed
        emit(.chatChanged(chatID))
        perform("chats.set_description", ["chat_id": chatID, "description": trimmed])
    }

    func addBot(_ botID: Bot.ID, to chatID: Chat.ID) {
        guard let index = chats.firstIndex(where: { $0.id == chatID }),
            chats[index].canAddBot,
            !chats[index].botIDs.contains(botID)
        else { return }
        chats[index].botIDs.append(botID)
        if isMock {
            let name = bot(botID)?.name ?? "A bot"
            append(Message(author: .system, body: .notice("\(name) joined the chat.")), to: chatID)
        }
        emit(.chatChanged(chatID))
        emit(.chatsChanged)
        perform("chats.add_bot", ["chat_id": chatID, "bot_id": botID])
    }

    func removeBot(_ botID: Bot.ID, from chatID: Chat.ID) {
        guard let index = chats.firstIndex(where: { $0.id == chatID }),
            chats[index].canRemoveBot
        else { return }
        chats[index].botIDs.removeAll { $0 == botID }
        emit(.chatChanged(chatID))
        emit(.chatsChanged)
        perform("chats.remove_bot", ["chat_id": chatID, "bot_id": botID])
    }

    /// Makes a group member the bot holding the work.
    func setOwner(_ botID: Bot.ID, of chatID: Chat.ID) {
        guard let index = chats.firstIndex(where: { $0.id == chatID }),
            chats[index].isGroup,
            chats[index].botIDs.contains(botID),
            chats[index].owner != botID
        else { return }
        chats[index].ownerBotID = botID
        emit(.chatChanged(chatID))
        perform("chats.set_owner", ["chat_id": chatID, "bot_id": botID])
    }

    // MARK: - Messages

    @discardableResult
    func append(_ message: Message, to chatID: Chat.ID) -> Message.ID? {
        guard let index = chats.firstIndex(where: { $0.id == chatID }) else { return nil }
        chats[index].messages.append(message)
        noteCommand(message, in: chatID)
        emit(.messageAdded(chatID, message.id))
        sortChats()
        emit(.chatsChanged)
        return message.id
    }

    /// Called for every streamed token, so this stays off the chat-list path.
    /// The sidebar preview refreshes when a step finishes instead.
    func update(_ messageID: Message.ID, in chatID: Chat.ID, _ transform: (inout Message) -> Void) {
        guard let chatIndex = chats.firstIndex(where: { $0.id == chatID }),
            let messageIndex = chats[chatIndex].index(of: messageID)
        else { return }
        transform(&chats[chatIndex].messages[messageIndex])
        noteCommand(chats[chatIndex].messages[messageIndex], in: chatID)
        emit(.messageChanged(chatID, messageID))
    }

    func refreshChatList() {
        emit(.chatsChanged)
    }

    /// Sends the message and returns the chat it landed in. Mentions are references the chat's
    /// bot acts on (it can message that bot); the message itself stays here. `mentions` are the
    /// bots picked from the `@` menu, which the CLI hands the bot by id. `replyTo` is the message
    /// the user answers, which the bot reads quoted.
    @discardableResult
    func send(_ text: String, attachments: [OutgoingAttachment] = [], mentions: [Bot.ID] = [], replyTo: Message.ID? = nil, in chatID: Chat.ID) -> Chat.ID {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty || !attachments.isEmpty, let chat = chat(chatID) else { return chatID }

        // The files are known here already; the CLI keeps the ids the bubble shows.
        for outgoing in attachments { attachmentURLs[outgoing.attachment.id] = outgoing.url }
        let quote = replyTo.flatMap { id in chat.messages.first { $0.id == id } }.flatMap(ReplyQuote.init(quoting:))
        let message = Message(author: .you, body: .text(trimmed), attachments: attachments.map(\.attachment), replyTo: quote)
        append(message, to: chatID)

        if isMock {
            replyEngine?.respond(to: trimmed, in: chat, messageID: message.id)
            return chatID
        }

        // Expect a turn to start; the CLI's job events confirm or clear this. A DM names its
        // bot right away so the working row appears with the send.
        if !chat.botIDs.isEmpty {
            let pendingID = "pending:\(chatID)"
            runningJobs.append((pendingID, chatID, chat.isDM ? chat.botIDs[0] : "", nil))
            emit(.respondingChanged(chatID))
            emit(.chatsChanged)
            Task { [weak self] in
                try? await Task.sleep(nanoseconds: 4_000_000_000)
                guard let self, self.runningJobs.contains(where: { $0.id == pendingID }) else { return }
                // Nothing started (no Runner answered); stop showing the bot at work.
                self.runningJobs.removeAll { $0.id == pendingID }
                self.emit(.respondingChanged(chatID))
                self.emit(.chatsChanged)
            }
        }
        var params: [String: Any] = [
            "chat_id": chatID, "text": trimmed, "message_id": message.id, "mentions": mentions,
            "attachments": attachments.map { outgoing in
                [
                    "id": outgoing.attachment.id, "path": outgoing.url.path, "name": outgoing.attachment.name,
                    "mime": outgoing.attachment.mime, "width": outgoing.attachment.width as Any,
                    "height": outgoing.attachment.height as Any,
                ]
            },
        ]
        if let quote { params["reply_to"] = quote.messageID }
        perform("chats.send", params)
        return chatID
    }

    // MARK: - Attachments

    /// Decoded profile images by attachment id, so avatars draw without touching the disk.
    private var avatarImages: [Attachment.ID: NSImage] = [:]

    /// The bot's profile image once this computer has it. The first ask for one that is not here
    /// fetches the blob and redraws the roster when it lands; until then callers draw the
    /// symbol and accent.
    func avatarImage(for bot: Bot) -> NSImage? {
        guard let attachment = bot.avatar else { return nil }
        if let image = avatarImages[attachment.id] { return image }
        if let url = attachmentURLs[attachment.id], let image = NSImage(contentsOf: url) {
            avatarImages[attachment.id] = image
            return image
        }
        guard !isMock, !fetchingAttachments.contains(attachment.id) else { return nil }
        fetchingAttachments.insert(attachment.id)
        Task { [weak self] in
            let params: [String: Any] = [
                "attachment": ["id": attachment.id, "name": attachment.name, "mime": attachment.mime, "size": attachment.size]
            ]
            guard let self else { return }
            do {
                let reply = try await client.request("files.path", params, as: Wire.FilePath.self)
                let url = URL(fileURLWithPath: reply.path)
                attachmentURLs[attachment.id] = url
                if let image = NSImage(contentsOf: url) { avatarImages[attachment.id] = image }
                emit(.rosterChanged)
                emit(.chatsChanged)
                for chat in chats where chat.botIDs.contains(bot.id) { emit(.chatChanged(chat.id)) }
            } catch {
                NSLog("fetching \(bot.name)'s image failed: \(error.localizedDescription)")
            }
        }
        return nil
    }

    /// Where an attachment's bytes are on this computer. A file sent from here is known at once; one
    /// sent from another Device is fetched through the CLI, and the message reloads when it lands.
    /// A fetch that failed keeps its reason until the user retries, so a scroll does not ask again.
    private var attachmentURLs: [Attachment.ID: URL] = [:]
    private var fetchingAttachments: Set<Attachment.ID> = []
    private var attachmentErrors: [Attachment.ID: String] = [:]

    func localURL(for attachment: Attachment, in chatID: Chat.ID, messageID: Message.ID) -> URL? {
        if let url = attachmentURLs[attachment.id], FileManager.default.fileExists(atPath: url.path) { return url }
        guard !isMock, !fetchingAttachments.contains(attachment.id), attachmentErrors[attachment.id] == nil else { return nil }
        fetchingAttachments.insert(attachment.id)
        Task { [weak self] in
            let params: [String: Any] = [
                "attachment": ["id": attachment.id, "name": attachment.name, "mime": attachment.mime, "size": attachment.size]
            ]
            guard let self else { return }
            defer { fetchingAttachments.remove(attachment.id) }
            do {
                let reply = try await client.request("files.path", params, as: Wire.FilePath.self)
                attachmentURLs[attachment.id] = URL(fileURLWithPath: reply.path)
            } catch {
                attachmentErrors[attachment.id] = error.localizedDescription
                NSLog("fetching \(attachment.name) failed: \(error.localizedDescription)")
            }
            emit(.messageChanged(chatID, messageID))
        }
        return nil
    }

    /// Why the attachment's bytes could not be fetched, until a retry.
    func attachmentError(for attachment: Attachment) -> String? { attachmentErrors[attachment.id] }

    func retryAttachment(_ attachment: Attachment, in chatID: Chat.ID, messageID: Message.ID) {
        guard attachmentErrors.removeValue(forKey: attachment.id) != nil else { return }
        _ = localURL(for: attachment, in: chatID, messageID: messageID)
        emit(.messageChanged(chatID, messageID))
    }

    /// The attachment as a file named for what it is, for Quick Look, another app, or a copy:
    /// the bytes under their attachment id carry no extension, so the CLI keeps a named copy.
    func openableURL(for attachment: Attachment) async throws -> URL {
        if let url = attachmentURLs[attachment.id], !url.pathExtension.isEmpty,
            FileManager.default.fileExists(atPath: url.path)
        { return url }
        let params: [String: Any] = [
            "attachment": ["id": attachment.id, "name": attachment.name, "mime": attachment.mime, "size": attachment.size],
            "named": true,
        ]
        return URL(fileURLWithPath: try await client.request("files.path", params, as: Wire.FilePath.self).path)
    }

    // MARK: - Outputs

    /// Every version of each chat's outputs, oldest first: what `outputs.list` answered, and
    /// output messages that arrived since. A chat is asked for when something first shows it, and
    /// again after a resync, while what it had stays on screen.
    private var outputMessages: [Chat.ID: [Message]] = [:]
    private var outputRequests: Set<Chat.ID> = []
    private var staleOutputs: Set<Chat.ID> = []

    /// The chat's outputs, the latest published first; empty until the CLI answers.
    func outputs(in chatID: Chat.ID) -> [OutputSeries] {
        if isMock { return OutputSeries.group(chat(chatID)?.messages ?? []) }
        let known = outputMessages[chatID]
        if known == nil || staleOutputs.contains(chatID) { listOutputs(in: chatID) }
        return OutputSeries.group(known ?? [])
    }

    private func listOutputs(in chatID: Chat.ID) {
        guard outputRequests.insert(chatID).inserted else { return }
        staleOutputs.remove(chatID)
        Task { [weak self] in
            guard let self else { return }
            defer { outputRequests.remove(chatID) }
            do {
                let reply = try await client.request("outputs.list", ["chat_id": chatID], as: Wire.OutputList.self)
                // Output messages that arrived while the list was on its way are in it too.
                var messages = reply.outputs.map { $0.toModel() }
                for message in outputMessages[chatID] ?? [] where !messages.contains(where: { $0.id == message.id }) {
                    messages.append(message)
                }
                outputMessages[chatID] = messages
                emit(.outputsChanged(chatID))
            } catch {
                NSLog("listing outputs failed: \(error.localizedDescription)")
            }
        }
    }

    /// Keeps a chat's known outputs in step with a message that was added, changed, or removed.
    private func noteOutput(_ message: Message?, removing id: Message.ID? = nil, in chatID: Chat.ID) {
        guard var messages = outputMessages[chatID] else { return }
        let before = messages
        messages.removeAll { $0.id == (message?.id ?? id) }
        if let message, message.output != nil { messages.append(message) }
        guard messages != before else { return }
        outputMessages[chatID] = messages
        emit(.outputsChanged(chatID))
    }

    func isResponding(in chatID: Chat.ID) -> Bool {
        runningJobs.contains { $0.chatID == chatID }
    }

    /// The chat's running tasks: its commands running in their terminals, here or on their
    /// Runners, in the order they started, once each has run for `taskDelay`. A command
    /// Auto-review is still judging has no terminal yet, and a quick one ends before it would
    /// show, so the Running tasks button does not flash for every `ls`.
    func runningCommands(in chatID: Chat.ID) -> [Message] {
        let now = Date()
        return chat(chatID)?.messages.filter { message in
            guard message.commandRun?.takesInput == true else { return false }
            // One that was running before this app heard of it has run long enough.
            return now.timeIntervalSince(commandStarts[message.id] ?? .distantPast) >= Self.taskDelay
        } ?? []
    }

    /// The commands in `chatID` that a bot's call still waits on, which Run in Background sends
    /// there.
    func foregroundCommands(in chatID: Chat.ID) -> [Message] {
        chat(chatID)?.messages.filter(\.runsInForeground) ?? []
    }

    /// Notes when a command starts running in its terminal, and tells the observers once it has
    /// run for `taskDelay`.
    private func noteCommand(_ message: Message, in chatID: Chat.ID) {
        guard message.commandRun?.takesInput == true else {
            commandStarts[message.id] = nil
            return
        }
        guard commandStarts[message.id] == nil else { return }
        commandStarts[message.id] = Date()
        DispatchQueue.main.asyncAfter(deadline: .now() + Self.taskDelay) { [weak self] in
            self?.emit(.runningTasksChanged(chatID))
        }
    }

    /// Bots with a turn running in this chat, in the order they started.
    func workingBots(in chatID: Chat.ID) -> [Bot.ID] {
        var seen: [Bot.ID] = []
        for job in runningJobs where job.chatID == chatID && !job.botID.isEmpty && !seen.contains(job.botID) {
            seen.append(job.botID)
        }
        return seen
    }

    /// Whether the bot has a turn running anywhere: the green dot on its avatar.
    func isWorking(_ botID: Bot.ID) -> Bool {
        runningJobs.contains { $0.botID == botID }
    }

    /// The mock reply engine's turns, so the demo shows the same working state as the CLI.
    func setMockWorking(_ botID: Bot.ID, in chatID: Chat.ID, _ working: Bool) {
        let id = "mock:\(chatID):\(botID)"
        runningJobs.removeAll { $0.id == id }
        if working { runningJobs.append((id, chatID, botID, nil)) }
        emit(.respondingChanged(chatID))
        emit(.chatsChanged)
    }

    /// Has the bot's turn read a message it holds for its next step now: a command it waits on
    /// goes to the background, and a reply in progress stops where it got to.
    func sendNow(_ messageID: Message.ID, in chatID: Chat.ID) {
        if isMock {
            replyEngine?.sendNow(chatID: chatID)
            return
        }
        perform("chats.send_now", ["chat_id": chatID, "message_id": messageID])
    }

    /// Marks a message the mock turn holds, or no longer holds.
    func setMockQueued(_ messageID: Message.ID, in chatID: Chat.ID, _ queued: Bool) {
        update(messageID, in: chatID) { $0.queued = queued }
    }

    func stopResponding(in chatID: Chat.ID) {
        if isMock {
            replyEngine?.cancel(chatID: chatID)
            return
        }
        perform("chats.stop", ["chat_id": chatID])
    }

    /// The chat's tasks, what waits on the user first, then open work, each newest first.
    func tasks(in chatID: Chat.ID) -> [DurableTask] {
        durableTasks.filter { $0.chatIds.contains(chatID) }.sorted {
            ($0.state.order, -$0.updatedAt) < ($1.state.order, -$1.updatedAt)
        }
    }

    func durableTask(_ id: String) -> DurableTask? {
        durableTasks.first { $0.id == id }
    }

    private func acceptDurableTask(_ task: DurableTask) {
        if let index = durableTasks.firstIndex(where: { $0.id == task.id }) {
            guard durableTasks[index].revision < task.revision else { return }
            durableTasks[index] = task
        } else { durableTasks.append(task) }
        emit(.durableTasksChanged)
    }

    func taskRequest(_ method: String, params: [String: Any]) async throws -> DurableTask {
        let task = try await client.request(method, params, as: DurableTask.self)
        acceptDurableTask(task)
        return task
    }

    // MARK: - Identity, pairing, providers

    func createIdentity() async throws -> [String] {
        let created = try await client.request(
            "identity.create", ["device_name": Host.current().localizedName ?? ""], as: Wire.IdentityCreated.self)
        hasIdentity = true
        isIdentityDevice = true
        emit(.identityChanged)
        return created.phrase
    }

    /// Unpairs another Device. The CLI has the relay drop its key; the Device wipes its copy of
    /// the account the next time it connects. Throws when the relay could not be told.
    /// Has a Runner whose CLI updates itself install the latest release now; it restarts into it
    /// once its bots are done, and its `machine` blob says how that goes.
    func updateDevice(_ id: Device.ID) async throws {
        if isMock {
            guard let index = devices.firstIndex(where: { $0.id == id }) else { return }
            devices[index].update?.state = "restarting"
            emit(.rosterChanged)
            return
        }
        _ = try await client.request("device.update", ["id": id])
    }

    /// Whether `lorca service` keeps the CLI running on a Runner, asked of it through the CLI.
    func serviceStatus(_ id: Device.ID) async throws -> Wire.ServiceStatus {
        if isMock { return .init(installed: false, running: false) }
        return try await client.request("device.service_status", ["id": id], as: Wire.ServiceStatus.self)
    }

    func unpairDevice(_ id: Device.ID) async throws {
        if !isMock {
            _ = try await client.request("device.unpair", ["id": id])
        }
        devices.removeAll { $0.id == id }
        emit(.rosterChanged)
    }

    /// Unpairs this Device. The CLI asks the relay to drop its key, best effort, then forgets the
    /// identity here; the `identity.changed` it sends brings back onboarding. The demo has no CLI,
    /// so its Unpair opens onboarding instead (`UnpairDevice`).
    func forgetIdentity() async throws {
        _ = try await client.request("identity.forget")
    }

    /// Deletes the account on the relay and on this Device; the other Devices forget it as the
    /// relay drops them. Throws when the relay could not be told, and nothing is deleted then.
    func deleteAccount() async throws {
        _ = try await client.request("identity.delete")
    }

    func restoreIdentity(phrase: String) async throws {
        _ = try await client.request("identity.restore", ["phrase": phrase])
        hasIdentity = true
        isIdentityDevice = true
        emit(.identityChanged)
    }

    func startPairing() async throws -> Wire.PairStart {
        try await client.request("pair.start", as: Wire.PairStart.self)
    }

    func pairingStatus(nonce: String) async throws -> Wire.PairStatus {
        try await client.request("pair.status", ["nonce": nonce], as: Wire.PairStatus.self)
    }

    func cancelPairing(nonce: String) {
        perform("pair.cancel", ["nonce": nonce])
    }

    func acceptPairing(_ pairingString: String) async throws {
        _ = try await client.request(
            "pair.accept", ["pairing_string": pairingString, "device_name": Host.current().localizedName ?? ""])
        hasIdentity = true
        emit(.identityChanged)
    }

    /// Stops waiting on the other Device: `acceptPairing` throws "Pairing cancelled".
    func abortPairing() {
        perform("pair.abort")
    }

    /// Whether the account this Mac just joined has a provider connected. The CLI answers once
    /// its first pull from the relay has brought the account's credentials, or after half a
    /// minute (`sync.account`).
    func accountHasProvider() async -> Bool {
        guard let synced = try? await client.request("sync.account", as: Wire.SyncAccount.self) else {
            // A CLI that cannot answer (an older one, or one that restarted) leaves what the store has.
            return providers.contains(where: \.isConnected)
        }
        return (synced.providers ?? []).compactMap { $0.toModel() }.contains(where: \.isConnected)
    }

    func providerAPIKey(_ kind: ProviderCredential.Kind) async throws -> Wire.ProviderAPIKey {
        try await client.request("providers.api_key", ["kind": kind.wireValue], as: Wire.ProviderAPIKey.self)
    }

    /// Connects an API-key provider. An empty `baseURL` means the provider's own API.
    func connectAPIKey(_ kind: ProviderCredential.Kind, apiKey: String, baseURL: String = "") async throws {
        var params: [String: Any] = ["api_key": apiKey]
        let trimmed = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        if !trimmed.isEmpty {
            params["base_url"] = trimmed
        }
        _ = try await client.request("providers.connect_\(kind.connectMethodSuffix)", params)
    }

    /// Runs a subscription sign-in (`providers.connect_chatgpt`, `providers.connect_grok`): the
    /// CLI opens the browser on this computer and the tokens go to the whole account.
    func connectSignIn(_ kind: ProviderCredential.Kind) async throws {
        _ = try await client.request("providers.connect_\(kind.wireValue)")
    }

    /// Stops a sign-in that is waiting on the browser: its `connectSignIn` fails, and finishing
    /// in the browser afterwards connects nothing.
    func cancelSignIn() {
        perform("providers.auth.cancel")
    }

    func credential(for kind: ProviderCredential.Kind) -> ProviderCredential? {
        providers.first { $0.kind == kind }
    }

    /// The models a bot of `kind` can run, in the catalog's order; the first is the default the
    /// CLI uses. A custom provider's are the ones saved with it, with the levels the CLI says
    /// they take. Decision models are Auto-review's alone.
    func models(for kind: ProviderCredential.Kind) -> [ProviderModel] {
        reviewModels(for: kind).filter { !$0.decides }
    }

    /// Every model of `kind` Auto-review can run: the ones bots can, and decision models.
    func reviewModels(for kind: ProviderCredential.Kind) -> [ProviderModel] {
        guard kind.isCustom else { return catalog.filter { $0.provider == kind } }
        let credential = credential(for: kind)
        return (credential?.models ?? []).map {
            ProviderModel(provider: kind, id: $0.id, label: $0.displayName, levels: $0.levels, decides: credential?.decides == true)
        }
    }

    /// The thinking levels `model` of `kind` takes, lowest first; see
    /// `ProviderModel.thinkingLevels(for:among:)`.
    func thinkingLevels(for kind: ProviderCredential.Kind, model: String?) -> [(id: String, label: String)] {
        ProviderModel.thinkingLevels(for: model, among: models(for: kind))
    }

    /// Disconnects a provider for the whole account, or deletes a custom one.
    func disconnectProvider(_ kind: ProviderCredential.Kind) async throws {
        if isMock, kind.isCustom {
            providers.removeAll { $0.kind == kind }
            emit(.rosterChanged)
            return
        }
        _ = try await client.request("providers.disconnect", ["kind": kind.wireValue])
    }

    /// The chat models a custom provider's server lists, asked through the CLI on this
    /// computer; nil when the server publishes no list.
    func listCustomModels(name: String, api: CustomAPI, baseURL: String, apiKey: String) async throws -> [CustomModel]? {
        if isMock { return MockData.listedModels(baseURL: baseURL) }
        let params: [String: Any] = ["name": name, "api": api.rawValue, "base_url": baseURL, "api_key": apiKey]
        let listing = try await client.request("providers.list_models", params, as: Wire.ListedModels.self)
        return listing.listed ? listing.models.map { $0.toModel() } : nil
    }

    /// Adds a custom provider, or saves the one `kind` names, once the CLI has heard from its
    /// server. `models` lists the ids bots can pick, the default first. Answers the provider's kind.
    @discardableResult
    func saveCustomProvider(
        kind: ProviderCredential.Kind?, name: String, api: CustomAPI, baseURL: String, apiKey: String, models: [String]
    ) async throws -> ProviderCredential.Kind {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let baseURL = baseURL.trimmingCharacters(in: .whitespacesAndNewlines)
        let models = models.map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        if isMock {
            let kind = kind ?? .custom(ProviderCredential.Kind.customPrefix + name.lowercased().replacingOccurrences(of: " ", with: "-"))
            let saved = ProviderCredential(
                kind: kind, isConnected: true, detail: baseURL, baseURL: baseURL, name: name, api: api,
                models: models.map { CustomModel(id: $0, levels: ["low", "medium", "high"]) })
            if let index = providers.firstIndex(where: { $0.kind == kind }) { providers[index] = saved } else { providers.append(saved) }
            emit(.rosterChanged)
            return kind
        }
        var params: [String: Any] = ["name": name, "api": api.rawValue, "base_url": baseURL, "api_key": apiKey, "models": models]
        if let kind { params["kind"] = kind.wireValue }
        let saved = try await client.request("providers.connect_custom", params, as: Wire.CustomProviderSaved.self)
        return ProviderCredential.Kind(wireValue: saved.kind) ?? .custom(saved.kind)
    }

    func setRelayURL(_ url: String) {
        relayURL = url.isEmpty ? nil : url
        perform("config.set", ["relay_url": url])
    }

    /// Replays the seeded conversations so the demo can be restarted from the Debug menu.
    func resetMockData() {
        guard isMock else { return }
        for chat in chats { replyEngine?.cancel(chatID: chat.id) }
        devices = MockData.devices()
        bots = MockData.bots()
        chats = MockData.chats()
        routines = MockData.routines()
        budgets = MockData.budgets()
        reviews = MockData.reviews()
        mockPlaybooks = MockData.playbooks()
        autoReview = MockData.autoReview()
        sharedLinks = MockData.sharedLinks()
        providers = MockData.providers()
        catalog = MockData.models()
        sortChats()
        emit(.snapshotReplaced)
    }
}
