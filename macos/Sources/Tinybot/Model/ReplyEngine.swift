import Foundation

/// Stands in for the CLI's agent loop: streams tokens, runs fake tools, hands off between bots.
@MainActor
final class ReplyEngine {
    private unowned let store: AppStore
    private var tasks: [Chat.ID: Task<Void, Never>] = [:]
    private var pendingThinkingID: [Chat.ID: Message.ID] = [:]
    private var turnCount = 0

    init(store: AppStore) {
        self.store = store
    }

    func isRunning(chatID: Chat.ID) -> Bool {
        tasks[chatID] != nil
    }

    func cancel(chatID: Chat.ID) {
        tasks[chatID]?.cancel()
        tasks[chatID] = nil
    }

    func respond(to prompt: String, in chat: Chat) {
        cancel(chatID: chat.id)
        let script = makeScript(prompt: prompt, chat: chat)
        guard !script.isEmpty else { return }

        turnCount += 1
        let chatID = chat.id
        tasks[chatID] = Task { [weak self] in
            for step in script {
                if Task.isCancelled { break }
                await self?.run(step, in: chatID)
            }
            self?.finish(chatID: chatID)
        }
    }

    private func finish(chatID: Chat.ID) {
        tasks[chatID] = nil
        if let orphan = pendingThinkingID.removeValue(forKey: chatID) {
            store.update(orphan, in: chatID) { $0.state = .complete }
        }
    }

    // MARK: - Steps

    private enum Step {
        case think(Bot.ID, seconds: Double)
        case say(Bot.ID, String)
        case tool(Bot.ID, ToolInvocation, seconds: Double)
        case handoff(from: Bot.ID, to: Bot.ID, reason: String)
    }

    private func run(_ step: Step, in chatID: Chat.ID) async {
        switch step {
        case let .think(botID, seconds):
            guard let id = store.append(
                Message(author: .bot(botID), body: .text(""), state: .thinking), to: chatID)
            else { return }
            pendingThinkingID[chatID] = id
            await sleep(seconds)

        case let .say(botID, text):
            let existing = pendingThinkingID.removeValue(forKey: chatID)
            let messageID: Message.ID
            if let existing {
                messageID = existing
                store.update(existing, in: chatID) { $0.state = .streaming }
            } else {
                guard let id = store.append(
                    Message(author: .bot(botID), body: .text(""), state: .streaming), to: chatID)
                else { return }
                messageID = id
            }

            var rendered = ""
            for token in tokenize(text) {
                if Task.isCancelled {
                    store.update(messageID, in: chatID) { $0.state = .complete }
                    return
                }
                rendered += token
                store.update(messageID, in: chatID) { $0.body = .text(rendered) }
                await sleep(Double.random(in: 0.016...0.042))
            }
            store.update(messageID, in: chatID) { $0.state = .complete }
            store.refreshChatList()

        case let .tool(botID, invocation, seconds):
            pendingThinkingID.removeValue(forKey: chatID)
            var running = invocation
            running.isRunning = true
            guard let id = store.append(
                Message(author: .bot(botID), body: .tool(running), state: .streaming), to: chatID)
            else { return }
            await sleep(seconds)
            store.update(id, in: chatID) { message in
                var finished = invocation
                finished.isRunning = false
                message.body = .tool(finished)
                message.state = .complete
            }
            store.refreshChatList()

        case let .handoff(from, to, reason):
            pendingThinkingID.removeValue(forKey: chatID)
            store.append(
                Message(author: .bot(from), body: .handoff(from: from, to: to, reason: reason)),
                to: chatID)
            await sleep(0.6)
        }
    }

    private func sleep(_ seconds: Double) async {
        try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
    }

    private func tokenize(_ text: String) -> [String] {
        var tokens: [String] = []
        var current = ""
        for character in text {
            current.append(character)
            if character == " " || character == "\n" {
                tokens.append(current)
                current = ""
            }
        }
        if !current.isEmpty { tokens.append(current) }
        return tokens
    }

    // MARK: - Script

    private func makeScript(prompt: String, chat: Chat) -> [Step] {
        let members = store.bots(in: chat)
        guard !members.isEmpty else { return [] }

        let mentioned = members.filter { bot in
            prompt.range(of: "@\(bot.name)", options: .caseInsensitive) != nil
        }
        let everyone = prompt.range(of: "@everyone", options: .caseInsensitive) != nil
        let responders = everyone ? members : (mentioned.isEmpty ? [members[0]] : mentioned)
        guard let lead = responders.first else { return [] }

        // A bot on an offline Runner cannot run its turn; the envelope waits on the relay.
        if let host = store.device(lead.runnerID), host.status == .offline {
            return [
                .think(lead.id, seconds: 0.5),
                .say(
                    lead.id,
                    "I'm queued on the relay — \(host.name) is offline, so this turn runs when that Runner reconnects."
                ),
            ]
        }

        var script: [Step] = [.think(lead.id, seconds: Double.random(in: 0.5...1.1))]

        let wantsWork = prompt.count > 46 || prompt.contains("?") == false
        let candidate = members.first { $0.id != lead.id && store.device($0.runnerID)?.status != .offline }

        if chat.isGroup, wantsWork, let helper = candidate, turnCount % 2 == 0 {
            script.append(
                .tool(
                    lead.id,
                    ToolInvocation(
                        name: "list_teammates",
                        summary: "Listed \(members.count) teammates",
                        detail: teammateDetail(members),
                        isRunning: false
                    ),
                    seconds: 0.7
                ))
            script.append(.handoff(from: lead.id, to: helper.id, reason: handoffReason(for: helper)))
            script.append(.think(helper.id, seconds: 0.5))
            script.append(.say(helper.id, reply(for: helper, prompt: prompt)))
            script.append(.say(lead.id, summary(from: helper)))
        } else {
            script.append(.say(lead.id, reply(for: lead, prompt: prompt)))
        }

        if everyone {
            for bot in members.dropFirst() where bot.id != lead.id {
                script.append(.think(bot.id, seconds: 0.4))
                script.append(.say(bot.id, reply(for: bot, prompt: prompt)))
            }
        }

        return script
    }

    private func teammateDetail(_ members: [Bot]) -> String {
        let rows = members.map { bot -> String in
            let host = store.device(bot.runnerID)
            let status = host?.status == .offline ? "offline" : bot.provider.rawValue
            return
                "  { \"name\": \"\(bot.name)\", \"runner\": \"\(host?.name ?? "?")\", \"provider\": \"\(status)\" }"
        }
        return "{\n  \"teammates\": [\n\(rows.joined(separator: ",\n"))\n  ]\n}"
    }

    private func handoffReason(for bot: Bot) -> String {
        switch bot.id {
        case "bot-patch": "Write the change"
        case "bot-scout": "Pull the context first"
        case "bot-quill": "Say it in plain words"
        case "bot-ember": "Check what is deployed"
        default: "Take the next step"
        }
    }

    private func summary(from bot: Bot) -> String {
        [
            "That matches what I expected from \(bot.name). I'll keep the thread here until you say otherwise.",
            "\(bot.name) has it. Tell me if you want that turned into a task on another Runner.",
            "Good — that closes the loop. Anything you want me to push back on?",
        ].randomElement() ?? ""
    }

    private func reply(for bot: Bot, prompt: String) -> String {
        let lowered = prompt.lowercased()

        if lowered.contains("hello") || lowered.contains("hi ") || lowered == "hi" {
            return "Here. \(bot.tagline.split(separator: "·").last.map(String.init)?.trimmingCharacters(in: .whitespaces) ?? "Ready when you are")."
        }

        let pool: [String]
        switch bot.id {
        case "bot-patch":
            pool = [
                """
                Smallest version that works:

                ```rust
                pub async fn serve(addr: SocketAddr) -> Result<()> {
                    let listener = TcpListener::bind(addr).await?;
                    tracing::info!(%addr, "tinybot serve");
                    while let Ok((stream, _)) = listener.accept().await {
                        tokio::spawn(handle(stream));
                    }
                    Ok(())
                }
                ```

                One task per connection, and `handle` owns the decrypt step so the accept loop stays dumb.
                """,
                """
                I'd keep this in the CLI rather than the app. The app should stay a renderer — the moment it knows how to decrypt, the key material has two homes and the threat model gets harder to explain.
                """,
                """
                Two options, and they are not close:

                - Put it behind `bootstrap` and let the app render whatever comes back.
                - Add a new event kind and teach both sides about it.

                The first one is free. Take the first one.
                """,
            ]
        case "bot-scout":
            pool = [
                """
                Checked the tree. The only place that touches this is the `blobs` handler and one test fixture, so the change is contained. Nothing in `web/` reads it.
                """,
                """
                Relevant prior art: Happy wraps the account DEK to each machine public key at pairing time, which is what `ARCHITECTURE.md` already describes. Following it means recovery is the backup phrase and nothing else, which is the property you want.
                """,
                """
                I found two answers and they disagree. The schema says `seq` is unique per identity; the handler treats it as unique per `(identity, kind)`. Worth deciding before the first migration lands, because it is painful afterwards.
                """,
            ]
        case "bot-quill":
            pool = [
                """
                Draft: "Your bots run on your Macs. Assign one to a Runner, and it works there with that machine's key. The relay carries ciphertext and nothing else."

                Three sentences, no adjectives doing work they haven't earned.
                """,
                """
                I'd cut "seamlessly" and "powerful". They are the words people skim. What is left says the same thing and is shorter.
                """,
            ]
        case "bot-ember":
            pool = [
                "Blast radius first: this touches the Worker only, no D1 migration, so a bad deploy is a rollback and not a restore.",
                "Deployed. The relay is answering the challenge endpoint in about 40ms from here, which is the number to watch when we add the blob listing.",
            ]
        default:
            pool = [
                """
                Here's how I'd sequence it:

                - Get the local websocket answering `bootstrap` with the same shape this app already renders.
                - Then swap the mock store for those events, one screen at a time.
                - Pairing last, because it is the only part that needs two machines to test.

                Want me to hand the first piece to Patch?
                """,
                """
                The constraint that decides this is **provider credentials live on the assigned Runner**. Anything that needs a key has to run there, so the answer is a job envelope, not a call from here.
                """,
                "Short answer: yes. Longer answer: yes, but not until pairing works on two machines, because that is where this gets interesting.",
            ]
        }

        return pool.randomElement() ?? ""
    }
}
