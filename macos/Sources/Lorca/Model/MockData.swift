import Foundation

/// Stand-in for the snapshot the CLI will send over the local websocket.
enum MockData {
    static func minutesAgo(_ minutes: Double) -> Date {
        Date(timeIntervalSinceNow: -minutes * 60)
    }

    static func devices() -> [Device] {
        [
            Device(
                id: "dev-workbench",
                name: "Workbench",
                model: "MacBook Pro (M4 Pro)",
                os: .macos,
                osVersion: "macOS 27.0",
                isThisDevice: true,
                status: .online,
                lastSeen: Date(),
                machineKey: "mk_7c41…a09f",
                plugins: plugins()
            ),
            Device(
                id: "dev-studio",
                name: "Studio",
                model: "Mac Studio (M3 Ultra)",
                os: .macos,
                osVersion: "macOS 27.0",
                isThisDevice: false,
                status: .online,
                lastSeen: minutesAgo(1),
                machineKey: "mk_1f88…23bd"
            ),
            Device(
                id: "dev-closet",
                name: "Closet mini",
                model: "Mac mini (M2)",
                os: .macos,
                osVersion: "macOS 26.4",
                isThisDevice: false,
                status: .offline,
                lastSeen: minutesAgo(184),
                machineKey: "mk_c052…77e1"
            ),
            Device(
                id: "dev-phone",
                name: "iPhone",
                model: "iPhone 17 Pro",
                os: .ios,
                osVersion: "iOS 27.0",
                isThisDevice: false,
                status: .online,
                lastSeen: minutesAgo(12),
                machineKey: "mk_9e3d…51c8"
            ),
        ]
    }

    static func providers() -> [ProviderCredential] {
        [
            ProviderCredential(kind: .deepseek, isConnected: true, detail: "sk-live…4f2c"),
            ProviderCredential(kind: .anthropic, isConnected: false, detail: "Not connected"),
            ProviderCredential(kind: .chatgpt, isConnected: true, detail: "you@lorca.app"),
            ProviderCredential(kind: .grok, isConnected: false, detail: "Not connected"),
        ]
    }

    static func plugins() -> [InstalledPlugin] {
        [
            InstalledPlugin(id: "github", name: "GitHub", description: "Issues, pull requests, code search, and repositories on GitHub.", version: "1", icon: "chevron.left.forwardslash.chevron.right", state: .ready, detail: "Ready"),
            InstalledPlugin(id: "linear", name: "Linear", description: "Issues, projects, and cycles in Linear.", version: "1", icon: "line.3.horizontal.decrease.circle", state: .needsAuth, detail: "Sign in"),
        ]
    }

    static func marketplace() -> [MarketplacePlugin] {
        [
            MarketplacePlugin(id: "github", name: "GitHub", description: "Issues, pull requests, code search, and repositories on GitHub.", icon: "chevron.left.forwardslash.chevron.right", homepage: nil, tags: ["git"], signsIn: true, variableNames: ["GITHUB_TOKEN"], installedOn: ["dev-workbench"]),
            MarketplacePlugin(id: "linear", name: "Linear", description: "Issues, projects, and cycles in Linear.", icon: "line.3.horizontal.decrease.circle", homepage: nil, tags: [], signsIn: true, variableNames: [], installedOn: ["dev-workbench"]),
            MarketplacePlugin(id: "notion", name: "Notion", description: "Pages and databases in a Notion workspace.", icon: "doc.richtext", homepage: nil, tags: [], signsIn: true, variableNames: [], installedOn: []),
            MarketplacePlugin(id: "playwright", name: "Browser", description: "Opens web pages in a headless browser on the Runner.", icon: "globe", homepage: nil, tags: [], signsIn: false, variableNames: [], installedOn: []),
        ]
    }

    static func autoReview() -> AutoReview {
        AutoReview(isEnabled: true, rules: [
            AutoReviewRule(id: "ar-1", text: "use GitHub create_issue", behavior: .allow, tool: "github/create_issue"),
            AutoReviewRule(id: "ar-2", text: "comment on a pull request", behavior: .ask),
        ])
    }

    static func routines() -> [Routine] {
        [
            Routine(
                id: "rt-brief", botID: "bot-nova", name: "Morning brief",
                prompt: "Read the overnight messages in every chat you are in and the calendar for today, then post a five-line brief: what needs a decision, what is waiting on someone else, and what you will do first.",
                schedule: "0 9 * * 1-5", scheduleText: "Weekdays at 9:00 AM", isEnabled: true, pausedReason: nil,
                lastRunAt: minutesAgo(190), lastOutcome: "sent",
                nextRunAt: Calendar.current.nextDate(after: Date(), matching: DateComponents(hour: 9, minute: 0), matchingPolicy: .nextTime),
                isRunning: false, createdAt: minutesAgo(60 * 24 * 12)),
            Routine(
                id: "rt-inbox", botID: "bot-nova", name: "Invoice check",
                prompt: "Look for invoices that landed since the last run and say which are flagged, or PASS when none did.",
                schedule: "every 2h", scheduleText: "Every 2 hours", isEnabled: false, pausedReason: nil,
                lastRunAt: minutesAgo(60 * 30), lastOutcome: "pass", nextRunAt: nil, isRunning: false,
                createdAt: minutesAgo(60 * 24 * 3)),
        ]
    }

    static func bots() -> [Bot] {
        [
            Bot(
                id: "bot-nova",
                name: "Nova",
                label: "Generalist",
                description: "Plans the work and delegates it to the team.",
                symbolName: "sparkles",
                accent: .indigo,
                runnerID: "dev-workbench",
                provider: .chatgpt,
                instructions:
                    "You coordinate the other bots. Break work down, hand off with message_bot, and summarize what came back.",
                createdAt: minutesAgo(60 * 24 * 21)
            ),
            Bot(
                id: "bot-patch",
                name: "Patch",
                label: "Rust and Swift",
                description: "Writes the diff, one small change at a time.",
                symbolName: "chevron.left.forwardslash.chevron.right",
                accent: .blue,
                runnerID: "dev-studio",
                provider: .deepseek,
                instructions:
                    "You implement changes. Prefer small diffs, explain the tradeoff in one line, never invent APIs.",
                createdAt: minutesAgo(60 * 24 * 18)
            ),
            Bot(
                id: "bot-scout",
                name: "Scout",
                label: "Research",
                description: "Reads the sources before it answers and cites them.",
                symbolName: "binoculars.fill",
                accent: .teal,
                runnerID: "dev-studio",
                provider: .deepseek,
                instructions: "You gather context and cite where it came from. Say when you are unsure.",
                createdAt: minutesAgo(60 * 24 * 12)
            ),
            Bot(
                id: "bot-quill",
                name: "Quill",
                label: "Writing",
                description: "Docs, copy, and release notes in plain language.",
                symbolName: "pencil.and.scribble",
                accent: .pink,
                runnerID: "dev-workbench",
                provider: .deepseek,
                instructions: "You write plainly. Short sentences. No filler, no exclamation marks.",
                createdAt: minutesAgo(60 * 24 * 9)
            ),
            Bot(
                id: "bot-ember",
                name: "Ember",
                label: "Ops",
                description: "Deploys and watches the relay.",
                symbolName: "bolt.horizontal.fill",
                accent: .orange,
                runnerID: "dev-closet",
                provider: .deepseek,
                instructions: "You handle deploys and incident triage. Always state the blast radius first.",
                createdAt: minutesAgo(60 * 24 * 4)
            ),
        ]
    }

    static func chats() -> [Chat] {
        [
            Chat(
                id: "chat-relay",
                kind: .group,
                customTitle: "Ship the relay",
                botIDs: ["bot-nova", "bot-patch", "bot-scout"],
                messages: relayThread(),
                unreadCount: 0,
                isPinned: true,
                createdAt: minutesAgo(400)
            ),
            Chat(
                id: "chat-nova",
                kind: .dm,
                customTitle: nil,
                botIDs: ["bot-nova"],
                messages: novaThread(),
                unreadCount: 0,
                isPinned: false,
                createdAt: minutesAgo(60 * 30)
            ),
            Chat(
                id: "chat-patch",
                kind: .dm,
                customTitle: nil,
                botIDs: ["bot-patch"],
                messages: patchThread(),
                unreadCount: 2,
                isPinned: false,
                createdAt: minutesAgo(60 * 26)
            ),
            Chat(
                id: "chat-launch",
                kind: .group,
                customTitle: "Launch copy",
                botIDs: ["bot-quill", "bot-nova"],
                messages: launchThread(),
                unreadCount: 0,
                isPinned: false,
                createdAt: minutesAgo(60 * 52)
            ),
            Chat(
                id: "chat-ember",
                kind: .dm,
                customTitle: nil,
                botIDs: ["bot-ember"],
                messages: emberThread(),
                unreadCount: 0,
                isPinned: false,
                createdAt: minutesAgo(60 * 72)
            ),
        ]
    }

    // MARK: - Threads

    private static func relayThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text(
                    "@Nova the relay is still storing plaintext chat titles. Get that fixed before we ship."),
                createdAt: minutesAgo(64)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .text(
                    "Agreed, titles belong inside the roster blob. Let me check who touched that path last."),
                createdAt: minutesAgo(63)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .tool(
                    ToolInvocation(
                        name: "list_teammates",
                        summary: "Listed 4 teammates",
                        detail: """
                            {
                              "teammates": [
                                { "name": "Patch",  "runner": "Studio",      "provider": "DeepSeek" },
                                { "name": "Scout",  "runner": "Studio",      "provider": "DeepSeek" },
                                { "name": "Quill",  "runner": "Workbench",   "provider": "DeepSeek" },
                                { "name": "Ember",  "runner": "Closet mini", "provider": "offline" }
                              ]
                            }
                            """,
                        isRunning: false
                    )),
                createdAt: minutesAgo(63)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .handoff(
                    from: "bot-nova", to: "bot-scout",
                    reason: "Find every place a title leaves the device in the clear"),
                createdAt: minutesAgo(62)
            ),
            Message(
                author: .bot("bot-scout"),
                body: .text(
                    """
                    Two leaks, both on the write path:

                    - `blobs.title` is a real column in the D1 schema, so every PUT writes it in the clear.
                    - The chat list endpoint returns that column so the sidebar can render without decrypting.

                    Nothing reads it anywhere else. The fix is a schema change plus one client change.
                    """),
                createdAt: minutesAgo(61)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .handoff(
                    from: "bot-nova", to: "bot-patch", reason: "Drop the column and move titles into the blob"),
                createdAt: minutesAgo(60)
            ),
            Message(
                author: .bot("bot-patch"),
                body: .text(
                    """
                    Done in two pieces. Drizzle migration first:

                    ```ts
                    export const blobs = sqliteTable("blobs", {
                      id: text("id").primaryKey(),
                      identityPubkey: text("identity_pubkey").notNull(),
                      kind: text("kind", { enum: ["roster", "chat", "job", "machine"] }).notNull(),
                      seq: integer("seq").notNull(),
                      ciphertext: blob("ciphertext").notNull(),
                      nonce: blob("nonce").notNull(),
                    })
                    ```

                    The title moves into the roster blob the CLI already decrypts, so the sidebar keeps working and the relay never sees it. `seq` still orders the blobs, which is all the Worker actually needs.
                    """),
                createdAt: minutesAgo(58)
            ),
            Message(
                author: .you,
                body: .text("Does that break Runners that are still offline?"),
                createdAt: minutesAgo(40)
            ),
            Message(
                author: .bot("bot-patch"),
                body: .text(
                    "No. Old blobs keep their row, the column just stops being written and the CLI falls back to the roster title when it is missing. Closet mini will pick it up whenever it comes back."
                ),
                createdAt: minutesAgo(39)
            ),
        ]
    }

    private static func novaThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text("What is left before the app can talk to a real CLI?"),
                createdAt: minutesAgo(190)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .text(
                    """
                    Three things, in the order they unblock each other:

                    - **Crypto and the blob protocol** — identity, pairing, wrapping DEKs to each machine key.
                    - **The relay Worker** — challenge/response auth, then the blob table.
                    - **The local websocket** — `bootstrap` and `chats.send` are enough to replace the mock snapshot this app runs on.

                    The UI already renders whatever the snapshot contains, so the third one is a swap, not a rewrite.
                    """),
                createdAt: minutesAgo(189)
            ),
            Message(
                author: .you,
                body: .text("Good. Keep the mock data shaped like the real snapshot."),
                createdAt: minutesAgo(180)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .text("That is the rule I am working to. Same field names, same ordering."),
                createdAt: minutesAgo(179)
            ),
            Message(
                author: .you,
                body: .text("File the relay title leak as an issue."),
                createdAt: minutesAgo(3)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .permission(
                    PermissionRequest(
                        pluginID: "github", pluginName: "GitHub", tool: "create_issue",
                        summary: "create_issue · repo: lorca, title: Relay stores plaintext chat titles", decision: .pending,
                        reason: "The issue is public on the repo.")),
                createdAt: minutesAgo(2)
            ),
        ]
    }

    private static func patchThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text("Why is `lorca serve` binding to 4862 and not something memorable?"),
                createdAt: minutesAgo(420)
            ),
            Message(
                author: .bot("bot-patch"),
                body: .text(
                    "It is in the ephemeral-adjacent range that nothing else claims, and it is written down in ARCHITECTURE.md so the app and CLI cannot drift. Changing it means changing both."
                ),
                createdAt: minutesAgo(419)
            ),
            Message(
                author: .bot("bot-patch"),
                body: .text(
                    "One thing worth deciding now: if the port is busy the CLI should fail loudly instead of picking the next free one. A silent second instance holding a second identity is the worst outcome here."
                ),
                createdAt: minutesAgo(30)
            ),
            Message(
                author: .bot("bot-patch"),
                body: .text("Want me to write that check into `doctor` as well?"),
                createdAt: minutesAgo(29)
            ),
        ]
    }

    private static func launchThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text("@Quill one paragraph for the site. No hype."),
                createdAt: minutesAgo(700)
            ),
            Message(
                author: .bot("bot-quill"),
                body: .text(
                    """
                    Lorca runs your bots on machines you own. Create a bot, give it a job, and it works on the Mac you assigned it to — with that machine's API key, not ours. Bots can hand work to each other. Everything that crosses the network is encrypted before it leaves the device, and the relay only ever sees ciphertext.
                    """),
                createdAt: minutesAgo(698)
            ),
            Message(
                author: .you,
                body: .text("Cut the last sentence in half."),
                createdAt: minutesAgo(690)
            ),
            Message(
                author: .bot("bot-quill"),
                body: .text("\"The relay only ever sees ciphertext.\" That is the half that earns its place."),
                createdAt: minutesAgo(689)
            ),
        ]
    }

    private static func emberThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text("Ember, status on the Worker deploy?"),
                createdAt: minutesAgo(60 * 4)
            ),
            Message(
                author: .system,
                body: .notice(
                    "Closet mini went offline. Ember's turn is queued on the relay and will run when that Runner reconnects."
                ),
                createdAt: minutesAgo(60 * 3 + 4)
            ),
        ]
    }

    // MARK: - Identity

    static let backupPhrase: [String] = [
        "k4mq", "7rth", "2bnz", "wq5f",
        "j3xd", "pv82", "ct6m", "9hsa",
        "e7lw", "4knr", "zb3u", "m5yq",
    ]

    static func pairingString() -> String {
        "lorca://pair?relay=https%3A%2F%2Florca.app&id=idk_9f2c41ab&ek=ek_57ca0d3b&n=482913"
    }
}
