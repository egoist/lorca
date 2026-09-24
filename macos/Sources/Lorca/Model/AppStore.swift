import AppKit
import Foundation

enum StoreEvent {
    case snapshotReplaced
    case rosterChanged
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
    case selectionChanged
    case connectionChanged
    case identityChanged
}

enum SettingsPane: String, CaseIterable {
    case general
    case providers
    case autoReview = "auto-review"
    case plugins
    case bots
    case device
    case advanced

    /// Bots and plugins live on a Runner, so these panes show one Device, picked at the top of
    /// the page. The others hold this computer's settings and the account's: the shared roster and
    /// the provider credentials.
    var isDeviceScoped: Bool {
        switch self {
        case .general, .providers, .autoReview, .advanced: false
        case .bots, .plugins, .device: true
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
    /// Auto-review, shared through the roster.
    private(set) var autoReview = AutoReview()
    /// The account's provider credentials, the same on every Device.
    private(set) var providers: [ProviderCredential] = []

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
    private(set) var relayURL: String?

    /// Turns in flight, by job id: the chat and the bot (empty while a group exchange is between
    /// member turns), and the routine when the turn is one of its runs. Drives the "is working"
    /// row, the presence dot on avatars, and the spinner on a routine.
    private var runningJobs: [(id: String, chatID: Chat.ID, botID: Bot.ID, routineID: Routine.ID?)] = []
    /// When each turn in flight was first seen, so a finished turn's reply can be told from
    /// what the bot said before it.
    private var jobStarts: [String: Date] = [:]
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
        autoReview = snapshot.autoReview?.toModel() ?? AutoReview()
        providers = (snapshot.providers ?? []).compactMap { $0.toModel() }
        runningJobs = (snapshot.runningTurns ?? []).map { ($0.jobId, $0.chatId, $0.botId, $0.routineId) }
        for id in snapshot.runningChatIds where !runningJobs.contains(where: { $0.chatID == id }) {
            runningJobs.append(("chat:\(id)", id, "", nil))
        }
        sortChats()
        emit(.snapshotReplaced)
    }

    // MARK: - Events from the CLI

    private func handle(event name: String, data: Data) {
        func decode<T: Decodable>(_ type: T.Type) -> T? {
            try? Wire.decoder.decode(Wire.Envelope<T>.self, from: data).data
        }

        switch name {
        case "snapshot":
            if let snapshot = decode(Wire.Snapshot.self) { apply(snapshot: snapshot) }

        case "roster.changed":
            guard let roster = decode(Wire.RosterChanged.self) else { return }
            devices = roster.devices.map { $0.toModel() }
            bots = roster.bots.map { $0.toModel() }
            if let incoming = roster.routines { routines = incoming.map { $0.toModel() } }
            if let incoming = roster.autoReview { autoReview = incoming.toModel() }
            if let incoming = roster.providers { providers = incoming.compactMap { $0.toModel() } }
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
            emit(.messageRemoved(payload.chatId, payload.messageId))

        case "chat.removed":
            guard let payload = decode(Wire.ChatRemoved.self) else { return }
            chats.removeAll { $0.id == payload.chatId }
            runningJobs.removeAll { $0.chatID == payload.chatId }
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

        case "relay.status":
            guard let status = decode(Wire.RelayStatus.self) else { return }
            relayConnected = status.connected
            relayUpdateRequired = status.updateRequired ?? false
            relayURL = status.url ?? relayURL
            emit(.rosterChanged)

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
        if let messageIndex = chats[chatIndex].index(of: message.id) {
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

    func observe(_ owner: AnyObject, _ handler: @escaping (StoreEvent) -> Void) {
        subscriptions.append(Subscription(owner: owner, handler: handler))
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
            return L("%@ on %@", only.provider.rawValue, host)
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
            body = !chat.isGroup && chat.botIDs.contains(to)
                ? L("Message from %@: %@", bot(from)?.name ?? L("a teammate"), reason)
                : L("Handed off to %@", bot(to)?.name ?? L("a teammate"))
        case let .notice(value): body = value
        case let .permission(request):
            let who = bot(last.author.botID ?? "")?.name ?? L("A bot")
            body = "\(who) \(request.verbPhrase)"
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
        thinking: String? = nil
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
            perform(
                "bots.create",
                [
                    "id": bot.id, "name": name, "description": description, "symbol_name": symbolName,
                    "accent": accent.rawValue, "runner_id": runnerID, "provider": provider.wireValue,
                    "model": model ?? "", "thinking": thinking ?? "", "chat_id": chatID,
                ])
        }
        return bot.id
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

    /// The marketplace with what each Runner already has, from the CLI.
    func marketplace(query: String = "") async throws -> [MarketplacePlugin] {
        if isMock {
            return MockData.marketplace().filter { query.isEmpty || $0.name.localizedCaseInsensitiveContains(query) }
        }
        return try await client.request("plugins.marketplace", ["query": query], as: Wire.Marketplace.self).plugins.map { $0.toModel() }
    }

    /// Installs a marketplace plugin on a Runner (here, or sealed to that Runner).
    func installPlugin(_ pluginID: String, on runnerID: Device.ID) async throws -> InstalledPlugin {
        guard !isMock else { return InstalledPlugin(id: pluginID, name: pluginID, description: "", version: "", icon: "", state: .ready, detail: "Ready") }
        return try await client.request("plugins.install", ["runner_id": runnerID, "plugin_id": pluginID], as: Wire.PluginInstalled.self).status.toModel()
    }

    func uninstallPlugin(_ pluginID: String, on runnerID: Device.ID) async throws {
        guard !isMock else { return }
        _ = try await client.request("plugins.uninstall", ["runner_id": runnerID, "plugin_id": pluginID])
    }

    func pluginDetail(_ pluginID: String, on runnerID: Device.ID) async throws -> PluginDetail {
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

    /// Starts the sign-in on the Runner; the browser opens there.
    func connectPlugin(_ pluginID: String, on runnerID: Device.ID) async throws -> String {
        guard !isMock else { return "Opened the sign-in page." }
        return try await client.request("plugins.connect", ["runner_id": runnerID, "plugin_id": pluginID], as: Wire.PluginConnected.self).message
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

    /// Answers a permission card: `allow`, `always`, or `deny`. The CLI confirms with the
    /// message's new decision.
    func answerPermission(chatID: Chat.ID, messageID: Message.ID, decision: String) {
        update(messageID, in: chatID) { message in
            guard case var .permission(request) = message.body else { return }
            request.decision = decision == "always" ? .always : (decision == "deny" ? .denied : .allowed)
            if request.isConnect, request.decision == .allowed { request.summary = L("Starting the sign-in…") }
            message.body = .permission(request)
        }
        perform("chats.permission", ["chat_id": chatID, "message_id": messageID, "decision": decision])
    }

    // MARK: - Routines

    /// Pauses or resumes a routine. A resumed schedule counts from now.
    func setRoutineEnabled(_ id: Routine.ID, _ enabled: Bool) {
        guard let index = routines.firstIndex(where: { $0.id == id }) else { return }
        routines[index].isEnabled = enabled
        routines[index].pausedReason = nil
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

    // MARK: - Messages

    @discardableResult
    func append(_ message: Message, to chatID: Chat.ID) -> Message.ID? {
        guard let index = chats.firstIndex(where: { $0.id == chatID }) else { return nil }
        chats[index].messages.append(message)
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
        emit(.messageChanged(chatID, messageID))
    }

    func refreshChatList() {
        emit(.chatsChanged)
    }

    /// Sends the message and returns the chat it landed in. Mentions are references the chat's
    /// bot acts on (it can message that bot); the message itself stays here. `mentions` are the
    /// bots picked from the `@` menu, which the CLI hands the bot by id.
    @discardableResult
    func send(_ text: String, attachments: [OutgoingAttachment] = [], mentions: [Bot.ID] = [], in chatID: Chat.ID) -> Chat.ID {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty || !attachments.isEmpty, let chat = chat(chatID) else { return chatID }

        // The files are known here already; the CLI keeps the ids the bubble shows.
        for outgoing in attachments { attachmentURLs[outgoing.attachment.id] = outgoing.url }
        let message = Message(author: .you, body: .text(trimmed), attachments: attachments.map(\.attachment))
        append(message, to: chatID)

        if isMock {
            replyEngine?.respond(to: trimmed, in: chat)
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
        perform(
            "chats.send",
            [
                "chat_id": chatID, "text": trimmed, "message_id": message.id, "mentions": mentions,
                "attachments": attachments.map { outgoing in
                    [
                        "id": outgoing.attachment.id, "path": outgoing.url.path, "name": outgoing.attachment.name,
                        "mime": outgoing.attachment.mime, "width": outgoing.attachment.width as Any,
                        "height": outgoing.attachment.height as Any,
                    ]
                },
            ])
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
    private var attachmentURLs: [Attachment.ID: URL] = [:]
    private var fetchingAttachments: Set<Attachment.ID> = []

    func localURL(for attachment: Attachment, in chatID: Chat.ID, messageID: Message.ID) -> URL? {
        if let url = attachmentURLs[attachment.id] { return url }
        guard !isMock, !fetchingAttachments.contains(attachment.id) else { return nil }
        fetchingAttachments.insert(attachment.id)
        Task { [weak self] in
            let params: [String: Any] = [
                "attachment": ["id": attachment.id, "name": attachment.name, "mime": attachment.mime, "size": attachment.size]
            ]
            guard let self else { return }
            do {
                let reply = try await client.request("files.path", params, as: Wire.FilePath.self)
                attachmentURLs[attachment.id] = URL(fileURLWithPath: reply.path)
                emit(.messageChanged(chatID, messageID))
            } catch {
                // Left in the fetching set: the relay does not have it, and every scroll would
                // ask again. A relaunch retries.
                NSLog("fetching \(attachment.name) failed: \(error.localizedDescription)")
            }
        }
        return nil
    }

    func isResponding(in chatID: Chat.ID) -> Bool {
        runningJobs.contains { $0.chatID == chatID }
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

    func stopResponding(in chatID: Chat.ID) {
        if isMock {
            replyEngine?.cancel(chatID: chatID)
            return
        }
        perform("chats.stop", ["chat_id": chatID])
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
    func unpairDevice(_ id: Device.ID) async throws {
        if !isMock {
            _ = try await client.request("device.unpair", ["id": id])
        }
        devices.removeAll { $0.id == id }
        emit(.rosterChanged)
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

    func credential(for kind: ProviderCredential.Kind) -> ProviderCredential? {
        providers.first { $0.kind == kind }
    }

    func disconnectProvider(_ kind: ProviderCredential.Kind) async throws {
        _ = try await client.request("providers.disconnect", ["kind": kind.wireValue])
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
        autoReview = MockData.autoReview()
        providers = MockData.providers()
        sortChats()
        emit(.snapshotReplaced)
    }
}
