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
    case selectionChanged
    case connectionChanged
    case identityChanged
}

enum Selection: Hashable {
    case chat(Chat.ID)
    case device(Device.ID)
}

/// The app's model. Everything comes from the CLI over 127.0.0.1; mutations are applied
/// optimistically and confirmed by the events the CLI sends back. `TINYBOT_MOCK=1` runs the
/// seeded demo data with the in-process reply engine instead.
@MainActor
final class AppStore {
    static let shared = AppStore()

    let isMock = ProcessInfo.processInfo.environment["TINYBOT_MOCK"] == "1"
    let client = CLIClient()
    let launcher = CLILauncher()

    private(set) var devices: [Device] = []
    private(set) var bots: [Bot] = []
    private(set) var chats: [Chat] = []

    /// True when the CLI answers on localhost (mock: toggled from the Debug menu).
    private(set) var isConnected = false
    /// nil until the CLI has answered `hello`.
    private(set) var hasIdentity: Bool?
    private(set) var isIdentityDevice = false
    private(set) var identityID: String?
    private(set) var relayConnected = false
    private(set) var relayURL: String?

    /// Turns in flight, by job id: the chat and the bot (empty while a group exchange is between
    /// member turns). Drives the "is working" row and the presence dot on avatars.
    private var runningJobs: [(id: String, chatID: Chat.ID, botID: Bot.ID)] = []
    private var replyEngine: ReplyEngine?
    private var started = false

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
        started = true
        if isMock {
            resetMockData()
            isConnected = true
            hasIdentity = true
            isIdentityDevice = true
            emit(.connectionChanged)
            emit(.identityChanged)
            return
        }

        client.onStateChange = { [weak self] state in
            guard let self else { return }
            switch state {
            case .connected:
                Task { await self.bootstrap() }
            case .disconnected, .connecting:
                if self.isConnected {
                    self.isConnected = false
                    self.runningJobs.removeAll()
                    self.emit(.connectionChanged)
                }
                // The CLI went away (a stale instance stopped, or it crashed): start ours.
                if state == .disconnected { self.launcher.ensureRunning() }
            }
        }
        client.onEvent = { [weak self] name, data in
            self?.handle(event: name, data: data)
        }
        launcher.onStatusChange = { [weak self] _ in
            self?.emit(.connectionChanged)
        }
        launcher.ensureRunning()
        client.connect()
    }

    func stop() {
        client.disconnect()
        launcher.stop()
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
        launcher.ensureRunning()
        client.reconnect()
    }

    private func bootstrap() async {
        do {
            let hello = try await client.request("hello", as: Wire.Hello.self)
            hasIdentity = hello.hasIdentity
            isIdentityDevice = hello.isIdentityDevice
            relayURL = hello.relayUrl
            relayConnected = hello.relayConnected
            let snapshot = try await client.request("bootstrap", as: Wire.Snapshot.self)
            apply(snapshot: snapshot)
            isConnected = true
            emit(.connectionChanged)
            emit(.identityChanged)
        } catch {
            NSLog("bootstrap failed: \(error.localizedDescription)")
        }
    }

    private func apply(snapshot: Wire.Snapshot) {
        hasIdentity = snapshot.hasIdentity
        isIdentityDevice = snapshot.isIdentityDevice
        identityID = snapshot.identityId
        relayURL = snapshot.relayUrl
        relayConnected = snapshot.relayConnected
        devices = snapshot.devices.map { $0.toModel() }
        bots = snapshot.bots.map { $0.toModel() }
        chats = snapshot.chats.map { $0.toModel() }
        runningJobs = (snapshot.runningTurns ?? []).map { ($0.jobId, $0.chatId, $0.botId) }
        for id in snapshot.runningChatIds where !runningJobs.contains(where: { $0.chatID == id }) {
            runningJobs.append(("chat:\(id)", id, ""))
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
            var merged: [Chat] = []
            var changed: [Chat.ID] = []
            for summary in roster.chats {
                let existing = chats.first { $0.id == summary.id }
                let chat = summary.toModel(existingMessages: existing?.messages ?? [], existingUnread: existing?.unreadCount ?? 0)
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
            runningJobs.removeAll { $0.id == "pending:\(job.chatId)" }
            runningJobs.append((job.jobId, job.chatId, job.botId))
            emit(.respondingChanged(job.chatId))
            emit(.chatsChanged)

        case "job.finished":
            guard let job = decode(Wire.JobEvent.self) else { return }
            runningJobs.removeAll { $0.id == job.jobId }
            emit(.respondingChanged(job.chatId))
            emit(.chatsChanged)

        case "relay.status":
            guard let status = decode(Wire.RelayStatus.self) else { return }
            relayConnected = status.connected
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

    var thisDevice: Device? {
        devices.first(where: \.isThisDevice)
    }

    func title(for chat: Chat) -> String {
        if let custom = chat.customTitle, !custom.isEmpty { return custom }
        let names = chat.botIDs.compactMap { bot($0)?.name }
        return names.isEmpty ? "New Chat" : names.joined(separator: ", ")
    }

    func subtitle(for chat: Chat) -> String {
        let members = bots(in: chat)
        if chat.isDM, let only = members.first {
            let host = device(only.runnerID)?.name ?? "unassigned"
            return "\(only.provider.rawValue) on \(host)"
        }
        let hosts = Set(members.compactMap { device($0.runnerID)?.name })
        let runnerLabel = hosts.count == 1 ? (hosts.first ?? "") : "\(hosts.count) Runners"
        let botLabel = members.count == 1 ? "1 bot" : "\(members.count) bots"
        return "Group · \(botLabel) · \(runnerLabel)"
    }

    func preview(for chat: Chat) -> String {
        // The last thing worth previewing: tool calls never are, except a sent message.
        let shown = chat.messages.last { message in
            if case let .tool(tool) = message.body { return tool.isSentMessage }
            return true
        }
        guard let last = shown else { return "No messages yet" }
        let body: String
        switch last.body {
        case let .text(value): body = value.isEmpty ? Attachment.summary(last.attachments) : value
        case let .tool(tool): body = "Messaged \(tool.recipientName): \(tool.detail)"
        case let .handoff(from, to, reason):
            body = !chat.isGroup && chat.botIDs.contains(to)
                ? "Message from \(bot(from)?.name ?? "a teammate"): \(reason)"
                : "Handed off to \(bot(to)?.name ?? "a teammate")"
        case let .notice(value): body = value
        }

        let flattened = body
            .replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "**", with: "")
            .replacingOccurrences(of: "`", with: "")
            .trimmingCharacters(in: .whitespacesAndNewlines)

        if chat.isGroup, case let .bot(id) = last.author, case .text = last.body {
            return "\(bot(id)?.name ?? "Bot"): \(flattened)"
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
        label: String,
        description: String = "",
        symbolName: String,
        accent: Accent,
        runnerID: Device.ID,
        provider: ProviderCredential.Kind,
        model: String? = nil
    ) -> Bot.ID {
        let bot = Bot(
            id: "bot-\(UUID().uuidString.lowercased().prefix(8))",
            name: name,
            label: label,
            description: description,
            symbolName: symbolName,
            accent: accent,
            runnerID: runnerID,
            provider: provider,
            model: model,
            instructions: "",
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
                    "id": bot.id, "name": name, "label": label, "description": description, "symbol_name": symbolName,
                    "accent": accent.rawValue, "runner_id": runnerID, "provider": provider.wireValue,
                    "model": model ?? "", "chat_id": chatID,
                ])
        }
        return bot.id
    }

    func updateBot(_ id: Bot.ID, name: String, label: String, description: String? = nil, provider: ProviderCredential.Kind? = nil) {
        guard let index = bots.firstIndex(where: { $0.id == id }) else { return }
        bots[index].name = name
        bots[index].label = label
        if let description { bots[index].description = description }
        if let provider { bots[index].provider = provider }
        emit(.rosterChanged)
        emit(.chatsChanged)
        var params: [String: Any] = ["id": id, "name": name, "label": label]
        if let description { params["description"] = description }
        if let provider { params["provider"] = provider.wireValue }
        perform("bots.update", params)
    }

    /// Provider and model a bot runs with. nil model means the provider's default.
    func setBotRuntime(_ id: Bot.ID, provider: ProviderCredential.Kind, model: String?) {
        guard let index = bots.firstIndex(where: { $0.id == id }) else { return }
        bots[index].provider = provider
        bots[index].model = model
        emit(.rosterChanged)
        emit(.chatsChanged)
        for chat in chats where chat.botIDs.contains(id) { emit(.chatChanged(chat.id)) }
        perform("bots.update", ["id": id, "provider": provider.wireValue, "model": model ?? ""])
    }

    func deleteChat(_ id: Chat.ID) {
        replyEngine?.cancel(chatID: id)
        chats.removeAll { $0.id == id }
        runningJobs.removeAll { $0.chatID == id }
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

    func markRead(_ id: Chat.ID) {
        guard let index = chats.firstIndex(where: { $0.id == id }), chats[index].unreadCount > 0
        else { return }
        chats[index].unreadCount = 0
        emit(.chatsChanged)
        perform("chats.mark_read", ["chat_id": id])
    }

    func rename(_ id: Chat.ID, to title: String) {
        guard let index = chats.firstIndex(where: { $0.id == id }) else { return }
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
    /// bot acts on (it can message that bot); the message itself stays here.
    @discardableResult
    func send(_ text: String, attachments: [OutgoingAttachment] = [], in chatID: Chat.ID) -> Chat.ID {
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
            runningJobs.append((pendingID, chatID, chat.isDM ? chat.botIDs[0] : ""))
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
                "chat_id": chatID, "text": trimmed, "message_id": message.id,
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

    /// Where an attachment's bytes are on this Mac. A file sent from here is known at once; one
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
        if working { runningJobs.append((id, chatID, botID)) }
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

    func connectDeepSeek(apiKey: String) async throws {
        _ = try await client.request("providers.connect_deepseek", ["api_key": apiKey])
    }

    func connectChatGPT() async throws {
        _ = try await client.request("providers.connect_chatgpt")
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
        sortChats()
        emit(.snapshotReplaced)
    }
}
