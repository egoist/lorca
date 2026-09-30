// The seeded demo (`LORCA_MOCK=1`), after the macOS app's MockData: the snapshot the CLI would send,
// for screenshots and for working on the views without a CLI. The marketplace is the CLI's own
// bundled index.

import { newMessageID, type AutoReview, type Bot, type Chat, type Device, type InstalledPlugin, type Marketplace, type Message, type ProviderCredential, type Routine } from "./models";
import { toMarketplace, type WireBotTemplate, type WireMarketplacePlugin } from "./wire";

const minutesAgo = (minutes: number) => Date.now() - minutes * 60_000;

function message(author: Message["author"], body: Message["body"], createdAt: number): Message {
  return { id: newMessageID(), author, body, state: { kind: "complete" }, createdAt, attachments: [] };
}

const you: Message["author"] = { kind: "you" };
const bot = (botID: string): Message["author"] => ({ kind: "bot", botID });
const text = (value: string): Message["body"] => ({ kind: "text", text: value });

export function plugins(): InstalledPlugin[] {
  return [
    { id: "github", name: "GitHub", description: "Issues, pull requests, code search, and repositories on GitHub.", version: "1", icon: "chevron.left.forwardslash.chevron.right", state: "ready", detail: "Ready" },
    { id: "linear", name: "Linear", description: "Issues, projects, and cycles in Linear.", version: "1", icon: "line.3.horizontal.decrease.circle", state: "needs_auth", detail: "Sign in" },
  ];
}

export function devices(): Device[] {
  return [
    { id: "dev-workbench", name: "Workbench", model: "ThinkPad X1 Carbon Gen 13", os: "linux", osVersion: "Ubuntu 26.04", isThisDevice: true, status: "online", lastSeen: Date.now(), machineKey: "mk_7c41…a09f", plugins: plugins() },
    { id: "dev-studio", name: "Studio", model: "Mac Studio (M3 Ultra)", os: "macos", osVersion: "macOS 27.0", isThisDevice: false, status: "online", lastSeen: minutesAgo(1), machineKey: "mk_1f88…23bd", plugins: [] },
    { id: "dev-closet", name: "Closet PC", model: "Desktop", os: "windows", osVersion: "Windows 11 25H2", isThisDevice: false, status: "offline", lastSeen: minutesAgo(184), machineKey: "mk_c052…77e1", plugins: [] },
    { id: "dev-phone", name: "iPhone", model: "iPhone 17 Pro", os: "ios", osVersion: "iOS 27.0", isThisDevice: false, status: "online", lastSeen: minutesAgo(12), machineKey: "mk_9e3d…51c8", plugins: [] },
  ];
}

export function providers(): ProviderCredential[] {
  return [
    { kind: "deepseek", isConnected: true, detail: "sk-live…4f2c" },
    { kind: "anthropic", isConnected: true, detail: "sk-ant…8d1a" },
    { kind: "opencode", isConnected: false, detail: "Not connected" },
    { kind: "opencode-go", isConnected: false, detail: "Not connected" },
    { kind: "chatgpt", isConnected: true, detail: "you@lorca.app" },
    { kind: "grok", isConnected: false, detail: "Not connected" },
  ];
}

/** Each template's routines in words, as the CLI says them. */
const scheduleTexts: Record<string, string[]> = {
  "morning-briefing": ["Weekdays at 8:30 AM"],
  "pr-reviewer": ["Weekdays at 9:00 AM"],
  lookout: ["Every 2 hours"],
  "issue-triager": ["Weekdays at 10:00 AM and 4:00 PM"],
  "error-watch": ["Weekdays at 9:00 AM, 1:00 PM, and 5:00 PM"],
  "competitor-watcher": ["Mondays at 9:00 AM"],
  "docs-keeper": ["Fridays at 3:00 PM"],
};

/** The bundled index's plugins and bots, as the CLI serves them. */
export async function mockMarketplace(): Promise<Marketplace> {
  const index = (await import("../../../crates/cli/marketplace/index.json")).default as unknown as {
    plugins: WireMarketplacePlugin[];
    bots: WireBotTemplate[];
  };
  return toMarketplace({
    plugins: index.plugins.map((plugin) => ({ ...plugin, installed_on: plugin.id === "github" || plugin.id === "linear" ? ["dev-workbench"] : [] })),
    bots: index.bots.map((template) => ({
      ...template,
      routines: (template.routines ?? []).map((routine, index) => ({ ...routine, schedule_text: scheduleTexts[template.id]?.[index] ?? routine.schedule })),
    })),
  });
}

export function autoReview(): AutoReview {
  return {
    isEnabled: true,
    rules: [
      { id: "ar-1", text: "use GitHub create_issue", behavior: "allow", tool: "github/create_issue" },
      { id: "ar-2", text: "comment on a pull request", behavior: "ask" },
    ],
  };
}

function nextNineAM(): number {
  const next = new Date();
  next.setHours(9, 0, 0, 0);
  if (next.getTime() <= Date.now()) next.setDate(next.getDate() + 1);
  return next.getTime();
}

export function routines(): Routine[] {
  return [
    {
      id: "rt-brief",
      botID: "bot-nova",
      name: "Morning brief",
      prompt: "Read the recent messages in every chat you are in and the launch checklist in the workspace. Post a short brief: what changed, what needs a decision, and what the team will do first.",
      schedule: "0 9 * * 1-5",
      scheduleText: "Weekdays at 9:00 AM",
      isEnabled: true,
      lastRunAt: minutesAgo(190),
      lastOutcome: "sent",
      nextRunAt: nextNineAM(),
      isRunning: false,
      createdAt: minutesAgo(60 * 24 * 12),
    },
    {
      id: "rt-checklist",
      botID: "bot-nova",
      name: "Launch checklist",
      prompt: "Review the launch checklist in the workspace and the latest team replies. Report new blockers or completed milestones, or PASS when nothing changed.",
      schedule: "every 2h",
      scheduleText: "Every 2 hours",
      isEnabled: false,
      lastRunAt: minutesAgo(60 * 30),
      lastOutcome: "pass",
      isRunning: false,
      createdAt: minutesAgo(60 * 24 * 3),
    },
  ];
}

export function bots(): Bot[] {
  return [
    {
      id: "bot-nova",
      name: "Project Manager",
      description: "Plans the work and delegates it to the team. Breaks work down, hands it off with message_bot, and summarizes what came back.",
      symbolName: "list.bullet.clipboard.fill",
      accent: "indigo",
      runnerID: "dev-workbench",
      provider: "chatgpt",
      createdAt: minutesAgo(60 * 24 * 21),
    },
    {
      id: "bot-patch",
      name: "Developer",
      description: "Implements changes in small diffs, explains the tradeoff in one line, and never invents APIs.",
      symbolName: "chevron.left.forwardslash.chevron.right",
      accent: "blue",
      runnerID: "dev-studio",
      provider: "deepseek",
      createdAt: minutesAgo(60 * 24 * 18),
    },
    {
      id: "bot-scout",
      name: "Researcher",
      description: "Gathers context, reads the sources before answering, cites them, and says when it is unsure.",
      symbolName: "magnifyingglass",
      accent: "teal",
      runnerID: "dev-studio",
      provider: "deepseek",
      createdAt: minutesAgo(60 * 24 * 12),
    },
    {
      id: "bot-quill",
      name: "Writer",
      description: "Writes docs, copy, and release notes in plain language: short sentences, no filler, and no exclamation marks.",
      symbolName: "pencil.and.scribble",
      accent: "pink",
      runnerID: "dev-workbench",
      provider: "anthropic",
      createdAt: minutesAgo(60 * 24 * 9),
    },
    {
      id: "bot-ember",
      name: "DevOps",
      description: "Handles deploys and incident triage, watches the relay, and always states the blast radius first.",
      symbolName: "server.rack",
      accent: "orange",
      runnerID: "dev-closet",
      provider: "deepseek",
      createdAt: minutesAgo(60 * 24 * 4),
    },
  ];
}

function chat(id: string, kind: Chat["kind"], botIDs: string[], messages: Message[], extra: Partial<Chat>): Chat {
  return { id, kind, botIDs, messages, unreadCount: 0, isPinned: false, createdAt: Date.now(), hasMore: false, ...extra };
}

export function chats(): Chat[] {
  return [
    chat("chat-relay", "group", ["bot-nova", "bot-patch", "bot-scout"], launchRoomThread(), { customTitle: "Launch room", isPinned: true, createdAt: minutesAgo(400) }),
    chat("chat-nova", "dm", ["bot-nova"], managerThread(), { createdAt: minutesAgo(60 * 30) }),
    chat("chat-patch", "dm", ["bot-patch"], developerThread(), { unreadCount: 2, createdAt: minutesAgo(60 * 26) }),
    chat("chat-launch", "group", ["bot-quill", "bot-nova"], launchThread(), { customTitle: "Launch copy", createdAt: minutesAgo(60 * 52) }),
    chat("chat-scout", "dm", ["bot-scout"], researcherThread(), { unreadCount: 1, createdAt: minutesAgo(60 * 24 * 12) }),
    chat("chat-quill", "dm", ["bot-quill"], writerThread(), { createdAt: minutesAgo(60 * 24 * 9) }),
    chat("chat-ember", "dm", ["bot-ember"], devopsThread(), { createdAt: minutesAgo(60 * 72) }),
  ];
}

function launchRoomThread(): Message[] {
  return [
    message(you, text("Launch review: @Researcher check onboarding, @Developer check the site. @Project Manager pull it together."), minutesAgo(14)),
    message(
      bot("bot-scout"),
      text("Walked through setup on Linux, Windows, and iPhone. Pairing is clear. One gap: the guide needs to explain that your computer runs the bots while you chat from your phone."),
      minutesAgo(12),
    ),
    message(bot("bot-patch"), text("Updated the getting-started guide and checked every download link. The site builds cleanly. The changes are ready to review."), minutesAgo(9)),
    message(
      bot("bot-nova"),
      text("The launch checklist is ready:\n\n- **Onboarding** — reviewed on Linux, Windows, and iPhone\n- **Website** — guide updated, links checked\n- **Launch copy** — Writer's draft is ready\n\nOnly your final review is left."),
      minutesAgo(7),
    ),
    message(you, text("Great. Keep the announcement as a draft until I've reviewed it."), minutesAgo(5)),
    message(bot("bot-nova"), text("Saved in `launch/announcement.md`. I'll include the checklist in your morning brief."), minutesAgo(4)),
  ];
}

function managerThread(): Message[] {
  return [
    message(you, text("Give me a short launch brief every weekday at 9. Focus on blockers and decisions."), minutesAgo(192)),
    message(
      bot("bot-nova"),
      text("Your **Morning brief** runs weekdays at 9:00 AM on Workbench. I'll read our chats and the launch checklist, then post what changed and what needs you."),
      minutesAgo(191),
    ),
    message({ kind: "system" }, { kind: "notice", text: "Routine · Morning brief" }, minutesAgo(190)),
    message(
      bot("bot-nova"),
      text("**Today's focus: the launch.**\n\n- Researcher is reviewing the setup guide.\n- Developer is checking the website and download links.\n- Writer has a first draft of the announcement.\n\nI'll bring their updates together in Launch room."),
      minutesAgo(190),
    ),
    message(you, text("Ask Writer to keep the announcement short and lead with what people can do."), minutesAgo(36)),
    message(bot("bot-nova"), { kind: "handoff", from: "bot-nova", to: "bot-quill", reason: "Draft a short launch announcement that leads with what people can do." }, minutesAgo(35)),
    message(bot("bot-nova"), text("Writer has the brief. I'll keep the final draft with the launch checklist for your review."), minutesAgo(34)),
  ];
}

function developerThread(): Message[] {
  return [
    message(you, text("Check the getting-started page and make sure every download link works."), minutesAgo(55)),
    message(bot("bot-patch"), text("The Windows and Linux downloads and the CLI install links work. I also checked the docs links in both languages."), minutesAgo(28)),
    message(bot("bot-patch"), text("The website build passes. I've left the changes ready for review."), minutesAgo(27)),
  ];
}

function researcherThread(): Message[] {
  return [
    message(you, text("Read the setup guide as a new user. What would you want explained sooner?"), minutesAgo(80)),
    message(
      bot("bot-scout"),
      text("I'd explain the Device roles right after pairing: your computer runs the bots, and your phone lets you chat with them. I added that note to `research/onboarding.md`."),
      minutesAgo(45),
    ),
  ];
}

function writerThread(): Message[] {
  return [
    message(bot("bot-nova"), { kind: "handoff", from: "bot-nova", to: "bot-quill", reason: "Draft a short launch announcement that leads with what people can do." }, minutesAgo(35)),
    message(
      bot("bot-quill"),
      text("Create a team of bots for your everyday work. Give each one a role, bring them into a group chat, and pick up the conversation from your phone. Lorca runs the bots on your computers and encrypts your chats before they sync.\n\nDraft saved to `launch/announcement.md`."),
      minutesAgo(24),
    ),
  ];
}

function launchThread(): Message[] {
  return [
    message(you, text("@Writer write a short welcome for the setup guide. @Project Manager check that it covers the first steps."), minutesAgo(110)),
    message(
      bot("bot-quill"),
      text("Meet your first bot. Give it a name and a job, connect your model provider, and send a message. Add more bots when you need a team, or pair your phone to take the conversation with you."),
      minutesAgo(108),
    ),
    message(bot("bot-nova"), text("That covers the first session. The pairing guide follows it with a computer-and-phone walkthrough."), minutesAgo(106)),
  ];
}

function devopsThread(): Message[] {
  return [
    message(you, text("Check the relay health and disk usage when you're back online."), minutesAgo(60 * 4)),
    message({ kind: "system" }, { kind: "notice", text: "Closet PC is offline. DevOps's turn is queued on the relay and will run when that Runner reconnects." }, minutesAgo(60 * 3 + 4)),
  ];
}

export const backupPhrase = ["k4mq", "7rth", "2bnz", "wq5f", "j3xd", "pv82", "ct6m", "9hsa", "e7lw", "4knr", "zb3u", "m5yq"];
