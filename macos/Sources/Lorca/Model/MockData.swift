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
            ProviderCredential(kind: .anthropic, isConnected: true, detail: "sk-ant…8d1a"),
            ProviderCredential(kind: .opencode, isConnected: false, detail: "Not connected"),
            ProviderCredential(kind: .opencodeGo, isConnected: false, detail: "Not connected"),
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
                prompt: "Read the recent messages in every chat you are in and the launch checklist in the workspace. Post a short brief: what changed, what needs a decision, and what the team will do first.",
                schedule: "0 9 * * 1-5", scheduleText: "Weekdays at 9:00 AM", isEnabled: true, pausedReason: nil,
                lastRunAt: minutesAgo(190), lastOutcome: "sent",
                nextRunAt: Calendar.current.nextDate(after: Date(), matching: DateComponents(hour: 9, minute: 0), matchingPolicy: .nextTime),
                isRunning: false, createdAt: minutesAgo(60 * 24 * 12)),
            Routine(
                id: "rt-checklist", botID: "bot-nova", name: "Launch checklist",
                prompt: "Review the launch checklist in the workspace and the latest team replies. Report new blockers or completed milestones, or PASS when nothing changed.",
                schedule: "every 2h", scheduleText: "Every 2 hours", isEnabled: false, pausedReason: nil,
                lastRunAt: minutesAgo(60 * 30), lastOutcome: "pass", nextRunAt: nil, isRunning: false,
                createdAt: minutesAgo(60 * 24 * 3)),
        ]
    }

    static func bots() -> [Bot] {
        [
            Bot(
                id: "bot-nova",
                name: "Project Manager",
                description: "Plans the work and delegates it to the team. Breaks work down, hands it off with message_bot, and summarizes what came back.",
                symbolName: "list.bullet.clipboard.fill",
                accent: .indigo,
                runnerID: "dev-workbench",
                provider: .chatgpt,
                createdAt: minutesAgo(60 * 24 * 21)
            ),
            Bot(
                id: "bot-patch",
                name: "Developer",
                description: "Implements changes in small diffs, explains the tradeoff in one line, and never invents APIs.",
                symbolName: "chevron.left.forwardslash.chevron.right",
                accent: .blue,
                runnerID: "dev-studio",
                provider: .deepseek,
                createdAt: minutesAgo(60 * 24 * 18)
            ),
            Bot(
                id: "bot-scout",
                name: "Researcher",
                description: "Gathers context, reads the sources before answering, cites them, and says when it is unsure.",
                symbolName: "magnifyingglass",
                accent: .teal,
                runnerID: "dev-studio",
                provider: .deepseek,
                createdAt: minutesAgo(60 * 24 * 12)
            ),
            Bot(
                id: "bot-quill",
                name: "Writer",
                description: "Writes docs, copy, and release notes in plain language: short sentences, no filler, and no exclamation marks.",
                symbolName: "pencil.and.scribble",
                accent: .pink,
                runnerID: "dev-workbench",
                provider: .anthropic,
                createdAt: minutesAgo(60 * 24 * 9)
            ),
            Bot(
                id: "bot-ember",
                name: "DevOps",
                description: "Handles deploys and incident triage, watches the relay, and always states the blast radius first.",
                symbolName: "server.rack",
                accent: .orange,
                runnerID: "dev-closet",
                provider: .deepseek,
                createdAt: minutesAgo(60 * 24 * 4)
            ),
        ]
    }

    static func chats() -> [Chat] {
        [
            Chat(
                id: "chat-relay",
                kind: .group,
                customTitle: "Launch room",
                botIDs: ["bot-nova", "bot-patch", "bot-scout"],
                messages: launchRoomThread(),
                unreadCount: 0,
                isPinned: true,
                createdAt: minutesAgo(400)
            ),
            Chat(
                id: "chat-nova",
                kind: .dm,
                customTitle: nil,
                botIDs: ["bot-nova"],
                messages: managerThread(),
                unreadCount: 0,
                isPinned: false,
                createdAt: minutesAgo(60 * 30)
            ),
            Chat(
                id: "chat-patch",
                kind: .dm,
                customTitle: nil,
                botIDs: ["bot-patch"],
                messages: developerThread(),
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
                id: "chat-scout",
                kind: .dm,
                customTitle: nil,
                botIDs: ["bot-scout"],
                messages: researcherThread(),
                unreadCount: 1,
                isPinned: false,
                createdAt: minutesAgo(60 * 24 * 12)
            ),
            Chat(
                id: "chat-quill",
                kind: .dm,
                customTitle: nil,
                botIDs: ["bot-quill"],
                messages: writerThread(),
                unreadCount: 0,
                isPinned: false,
                createdAt: minutesAgo(60 * 24 * 9)
            ),
            Chat(
                id: "chat-ember",
                kind: .dm,
                customTitle: nil,
                botIDs: ["bot-ember"],
                messages: devopsThread(),
                unreadCount: 0,
                isPinned: false,
                createdAt: minutesAgo(60 * 72)
            ),
        ]
    }

    // MARK: - Threads

    private static func launchRoomThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text("Launch review: @Researcher check onboarding, @Developer check the site. @Project Manager pull it together."),
                createdAt: minutesAgo(14)
            ),
            Message(
                author: .bot("bot-scout"),
                body: .text("Walked through setup on Mac and iPhone. Pairing is clear. One gap: the guide needs to explain that your Mac runs the bots while you chat from your phone."),
                createdAt: minutesAgo(12)
            ),
            Message(
                author: .bot("bot-patch"),
                body: .text("Updated the getting-started guide and checked every download link. The site builds cleanly. The changes are ready to review."),
                createdAt: minutesAgo(9)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .text("The launch checklist is ready:\n\n- **Onboarding** — reviewed on Mac and iPhone\n- **Website** — guide updated, links checked\n- **Launch copy** — Writer's draft is ready\n\nOnly your final review is left."),
                createdAt: minutesAgo(7)
            ),
            Message(
                author: .you,
                body: .text("Great. Keep the announcement as a draft until I've reviewed it."),
                createdAt: minutesAgo(5)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .text("Saved in `launch/announcement.md`. I'll include the checklist in your morning brief."),
                createdAt: minutesAgo(4)
            ),
        ]
    }

    private static func managerThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text("Give me a short launch brief every weekday at 9. Focus on blockers and decisions."),
                createdAt: minutesAgo(192)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .text("Your **Morning brief** runs weekdays at 9:00 AM on Workbench. I'll read our chats and the launch checklist, then post what changed and what needs you."),
                createdAt: minutesAgo(191)
            ),
            Message(
                author: .system,
                body: .notice("Routine · Morning brief"),
                createdAt: minutesAgo(190)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .text("**Today's focus: the launch.**\n\n- Researcher is reviewing the setup guide.\n- Developer is checking the website and download links.\n- Writer has a first draft of the announcement.\n\nI'll bring their updates together in Launch room."),
                createdAt: minutesAgo(190)
            ),
            Message(
                author: .you,
                body: .text("Ask Writer to keep the announcement short and lead with what people can do."),
                createdAt: minutesAgo(36)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .handoff(from: "bot-nova", to: "bot-quill", reason: "Draft a short launch announcement that leads with what people can do."),
                createdAt: minutesAgo(35)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .text("Writer has the brief. I'll keep the final draft with the launch checklist for your review."),
                createdAt: minutesAgo(34)
            ),
        ]
    }

    private static func developerThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text("Check the getting-started page and make sure every download link works."),
                createdAt: minutesAgo(55)
            ),
            Message(
                author: .bot("bot-patch"),
                body: .text("The Mac download and CLI install links work. I also checked the docs links in both languages."),
                createdAt: minutesAgo(28)
            ),
            Message(
                author: .bot("bot-patch"),
                body: .text("The website build passes. I've left the changes ready for review."),
                createdAt: minutesAgo(27)
            ),
        ]
    }

    private static func researcherThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text("Read the setup guide as a new user. What would you want explained sooner?"),
                createdAt: minutesAgo(80)
            ),
            Message(
                author: .bot("bot-scout"),
                body: .text("I'd explain the Device roles right after pairing: your Mac runs the bots, and your phone lets you chat with them. I added that note to `research/onboarding.md`."),
                createdAt: minutesAgo(45)
            ),
        ]
    }

    private static func writerThread() -> [Message] {
        [
            Message(
                author: .bot("bot-nova"),
                body: .handoff(from: "bot-nova", to: "bot-quill", reason: "Draft a short launch announcement that leads with what people can do."),
                createdAt: minutesAgo(35)
            ),
            Message(
                author: .bot("bot-quill"),
                body: .text("Create a team of bots for your everyday work. Give each one a role, bring them into a group chat, and pick up the conversation from your phone. Lorca runs the bots on your computers and encrypts your chats before they sync.\n\nDraft saved to `launch/announcement.md`."),
                createdAt: minutesAgo(24)
            ),
        ]
    }

    private static func launchThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text("@Writer write a short welcome for the setup guide. @Project Manager check that it covers the first steps."),
                createdAt: minutesAgo(110)
            ),
            Message(
                author: .bot("bot-quill"),
                body: .text("Meet your first bot. Give it a name and a job, connect your model provider, and send a message. Add more bots when you need a team, or pair your phone to take the conversation with you."),
                createdAt: minutesAgo(108)
            ),
            Message(
                author: .bot("bot-nova"),
                body: .text("That covers the first session. The pairing guide follows it with a Mac-and-phone walkthrough."),
                createdAt: minutesAgo(106)
            ),
        ]
    }

    private static func devopsThread() -> [Message] {
        [
            Message(
                author: .you,
                body: .text("Check the relay health and disk usage when you're back online."),
                createdAt: minutesAgo(60 * 4)
            ),
            Message(
                author: .system,
                body: .notice("Closet mini is offline. DevOps's turn is queued on the relay and will run when that Runner reconnects."),
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
