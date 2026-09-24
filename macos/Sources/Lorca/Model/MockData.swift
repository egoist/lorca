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

    /// The bundled index's plugins and bots, as the CLI serves them.
    static func marketplace() -> Marketplace {
        Marketplace(plugins: marketplacePlugins(), bots: marketplaceBots())
    }

    private static func marketplacePlugins() -> [MarketplacePlugin] {
        [
            MarketplacePlugin(
                id: "github", name: "GitHub", description: "Issues, pull requests, code search, and repositories on GitHub.",
                icon: "chevron.left.forwardslash.chevron.right", homepage: "https://github.com/github/github-mcp-server", author: "GitHub",
                category: "code", isFeatured: true, tags: [],
                servers: [.init(name: "github", address: "https://api.githubcopilot.com/mcp/", isRemote: true, signsIn: true)],
                skills: [],
                variables: [.init(name: "GITHUB_TOKEN", description: "A personal access token (github.com/settings/tokens), the quickest way in.", secret: true, required: false), .init(name: "GITHUB_CLIENT_ID", description: "Normally not needed: the client id of your own OAuth app (github.com/settings/developers, Device Flow enabled), to sign in with it instead of Lorca's.", secret: false, required: false), .init(name: "GITHUB_CLIENT_SECRET", description: "Only for a browser sign-in with your own app: its client secret.", secret: true, required: false)],
                installedOn: ["dev-workbench"]),
            MarketplacePlugin(
                id: "linear", name: "Linear", description: "Issues, projects, and cycles in Linear.",
                icon: "line.3.horizontal.decrease.circle", homepage: "https://linear.app/docs/mcp", author: "Linear",
                category: "productivity", isFeatured: true, tags: [],
                servers: [.init(name: "linear", address: "https://mcp.linear.app/mcp", isRemote: true, signsIn: true)],
                skills: [],
                variables: [],
                installedOn: ["dev-workbench"]),
            MarketplacePlugin(
                id: "notion", name: "Notion", description: "Pages and databases in a Notion workspace.",
                icon: "doc.richtext", homepage: "https://developers.notion.com/docs/mcp", author: "Notion",
                category: "productivity", isFeatured: true, tags: [],
                servers: [.init(name: "notion", address: "https://mcp.notion.com/mcp", isRemote: true, signsIn: true)],
                skills: [],
                variables: [],
                installedOn: []),
            MarketplacePlugin(
                id: "sentry", name: "Sentry", description: "Errors, issues, and releases in Sentry.",
                icon: "exclamationmark.triangle", homepage: "https://mcp.sentry.dev", author: "Sentry",
                category: "code", isFeatured: false, tags: [],
                servers: [.init(name: "sentry", address: "https://mcp.sentry.dev/mcp", isRemote: true, signsIn: true)],
                skills: [],
                variables: [],
                installedOn: []),
            MarketplacePlugin(
                id: "context7", name: "Context7", description: "Up-to-date documentation and code examples for libraries and frameworks.",
                icon: "book.closed", homepage: "https://context7.com", author: "Upstash",
                category: "research", isFeatured: false, tags: [],
                servers: [.init(name: "context7", address: "https://mcp.context7.com/mcp", isRemote: true, signsIn: false)],
                skills: [],
                variables: [.init(name: "CONTEXT7_API_KEY", description: "Optional API key from context7.com for higher limits.", secret: true, required: false)],
                installedOn: []),
            MarketplacePlugin(
                id: "playwright", name: "Browser", description: "Opens web pages in a headless browser on the Runner to read them, fill forms, and take screenshots.",
                icon: "globe", homepage: "https://github.com/microsoft/playwright-mcp", author: "Microsoft",
                category: "research", isFeatured: true, tags: [],
                servers: [.init(name: "browser", address: "npx -y @playwright/mcp@latest --headless", isRemote: false, signsIn: false)],
                skills: [.init(name: "Reading a page", description: "How to read a page without filling the context.")],
                variables: [],
                installedOn: []),
        ]
    }

    private static func marketplaceBots() -> [BotTemplate] {
        [
            BotTemplate(
                id: "morning-briefing", name: "Morning Briefing",
                summary: "Preps a short morning briefing from your issues, errors, and pull requests",
                description: "Each morning you prepare a short briefing for the user from the services they connected: new and assigned issues, pull requests waiting on them, and fresh errors. Lead with the three things that need them today, then list the rest one line each, with a link to every item. Skip what has not changed since the last briefing. When a service is not connected, say so once and work with the others.",
                symbolName: "sun.max.fill", accent: .orange, category: "productivity",
                isFeatured: true, author: "Lorca", plugins: ["github", "linear", "sentry"],
                routines: [.init(name: "Morning briefing", scheduleText: "Weekdays at 8:30 AM", prompt: "Prepare today's briefing: what needs the user today first, then the rest one line each, with links.")],
                memory: ["The user wants the briefing short: the top three items first, then everything else one line each."]),
            BotTemplate(
                id: "researcher", name: "Researcher",
                summary: "Digs into any question across the web and your docs, then writes up what it found",
                description: "You research questions for the user. Restate the question and what a good answer needs, and ask one clarifying question only when the scope is truly unclear. Search the web, read the pages that matter in the browser, and check library documentation with Context7 when the question is technical. Cite every claim with its link, say how sure you are, and keep facts apart from your reading of them. Write anything longer than a few paragraphs to a Markdown file in your working directory and give the user the short version in chat.",
                symbolName: "binoculars.fill", accent: .teal, category: "research",
                isFeatured: true, author: "Lorca", plugins: ["context7", "playwright"],
                routines: [],
                memory: []),
            BotTemplate(
                id: "pr-reviewer", name: "PR Reviewer",
                summary: "Reviews new pull requests on GitHub and leaves clear, specific notes",
                description: "You review pull requests in the user's repositories on GitHub. Read the whole change and the code around it before you comment. Look for bugs first, then risky edge cases, missing tests, and unclear names; skip nits a formatter would catch. Name the file and line for every note and suggest the concrete fix. Post nothing to GitHub unless the user asks: give them your review in chat, most important first, and say plainly when a change looks good.",
                symbolName: "checklist", accent: .blue, category: "code",
                isFeatured: true, author: "Lorca", plugins: ["github"],
                routines: [.init(name: "Review new pull requests", scheduleText: "Weekdays at 9:00 AM", prompt: "Review the pull requests opened or updated since your last run in the repositories the user named.")],
                memory: ["The user wants review notes that name the file and line and suggest a concrete change."]),
            BotTemplate(
                id: "lookout", name: "Lookout",
                summary: "Watches the pages you name and tells you when they change",
                description: "You watch web pages for the user: prices, release notes, job posts, status pages, whatever they name. Keep the list of watched pages, and what counts as a change on each, in your memory, and ask for it when it is empty. On each check, open every page in the browser, compare it with what you saw last time, and report only real changes: what changed, the old and the new value, and the link.",
                symbolName: "eye.fill", accent: .purple, category: "research",
                isFeatured: true, author: "Lorca", plugins: ["playwright"],
                routines: [.init(name: "Check watched pages", scheduleText: "Every 2 hours", prompt: "Check every watched page for changes since the last check and report only what changed.")],
                memory: []),
            BotTemplate(
                id: "issue-triager", name: "Issue Triager",
                summary: "Sorts new Linear issues, fills in missing details, and flags what needs you",
                description: "You triage new issues in Linear for the user. For each one, check it is not a duplicate, add the details you can find (steps, the affected area, links), and suggest a priority, an owner, and labels. Change nothing in Linear until the user says which changes you may make on your own, then keep to those. Report what you triaged as a short list, with anything urgent at the top.",
                symbolName: "tray.full.fill", accent: .indigo, category: "productivity",
                isFeatured: false, author: "Lorca", plugins: ["linear"],
                routines: [.init(name: "Triage new issues", scheduleText: "Weekdays at 10:00 AM and 4:00 PM", prompt: "Triage the Linear issues created since your last run, anything urgent first.")],
                memory: []),
            BotTemplate(
                id: "error-watch", name: "Error Watch",
                summary: "Watches Sentry for new errors, finds the cause in the code, and drafts a fix",
                description: "You watch Sentry for new and regressed errors in the user's projects. For each one, read the stack trace and the events, find the cause in the code, and explain it in two or three sentences with the file and line. When the cause is clear, draft a fix as a patch in your working directory and say how confident you are. Group repeats of one problem, and never resolve or ignore anything in Sentry yourself.",
                symbolName: "exclamationmark.triangle.fill", accent: .red, category: "code",
                isFeatured: false, author: "Lorca", plugins: ["sentry", "github"],
                routines: [.init(name: "Check new errors", scheduleText: "Weekdays at 9:00 AM, 1:00 PM, and 5:00 PM", prompt: "Look at the errors that are new or regressed since your last run and report each one with its likely cause.")],
                memory: []),
            BotTemplate(
                id: "competitor-watcher", name: "Competitor Watcher",
                summary: "Tracks competitors' pricing and launches, and briefs you every week",
                description: "You track the user's competitors: their pricing pages, changelogs, blogs, and launch posts. Keep the list of competitors and the pages that matter in your memory, and ask for it when it is empty. Each week, compare what you find with last week's notes and brief the user on what actually changed and why it might matter, with links. Keep a running log so later briefings can point back to earlier changes.",
                symbolName: "chart.line.uptrend.xyaxis", accent: .green, category: "research",
                isFeatured: false, author: "Lorca", plugins: ["playwright"],
                routines: [.init(name: "Weekly competitor briefing", scheduleText: "Mondays at 9:00 AM", prompt: "Check each competitor's pages and brief the user on what changed since last week.")],
                memory: []),
            BotTemplate(
                id: "prototyper", name: "Prototyper",
                summary: "Turns your ideas into working prototypes on its computer",
                description: "You build quick, working prototypes of the user's ideas on your Runner: small web apps, scripts, and tools. Ask at most one question before you start, then build the smallest version that shows the idea working, run it, and fix what breaks. Tell the user where the files are and how to run or open it, and list what you left out. Prefer plain, well-known tools that run without setup.",
                symbolName: "hammer.fill", accent: .pink, category: "code",
                isFeatured: false, author: "Lorca", plugins: [],
                routines: [],
                memory: []),
            BotTemplate(
                id: "docs-keeper", name: "Docs Keeper",
                summary: "Keeps your Notion docs in step with what shipped on GitHub",
                description: "You keep the user's documentation in Notion in step with their code. Each week, read what shipped on GitHub (merged pull requests and releases), find the Notion pages that describe those parts, and draft the updates: what to change, where, and the new wording. Edit a page only after the user approves the draft, and say which pages you could not find.",
                symbolName: "doc.text.fill", accent: .blue, category: "productivity",
                isFeatured: false, author: "Lorca", plugins: ["notion", "github"],
                routines: [.init(name: "Weekly docs check", scheduleText: "Fridays at 3:00 PM", prompt: "Compare what shipped this week with the Notion docs and draft the updates for the user to approve.")],
                memory: []),
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
