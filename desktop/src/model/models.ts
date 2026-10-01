// The app's model, after the macOS app's Models.swift. Everything comes from the CLI; the wire
// shapes are mapped into these in wire.ts. Values are immutable: a change makes a new object, so
// the views can tell what changed by identity.

import { L, Lc } from "../l10n";
import * as Format from "./format";

// MARK: - Providers

export type ProviderKind = "deepseek" | "anthropic" | "opencode" | "opencode-go" | "chatgpt" | "grok";

/** Every kind, in the order Settings lists them. */
export const providerKinds: ProviderKind[] = ["deepseek", "anthropic", "opencode", "opencode-go", "chatgpt", "grok"];

export function isProviderKind(value: string): value is ProviderKind {
  return (providerKinds as string[]).includes(value);
}

/** The provider's name: "DeepSeek", "OpenCode Zen". */
export function providerName(kind: ProviderKind): string {
  switch (kind) {
    case "deepseek":
      return "DeepSeek";
    case "anthropic":
      return "Anthropic";
    case "opencode":
      return "OpenCode Zen";
    case "opencode-go":
      return "OpenCode Go";
    case "chatgpt":
      return "ChatGPT";
    case "grok":
      return "Grok";
  }
}

/** Connects with a pasted API key; ChatGPT and Grok sign in through the browser instead. */
export function usesAPIKey(kind: ProviderKind): boolean {
  return kind !== "chatgpt" && kind !== "grok";
}

export function providerSymbol(kind: ProviderKind): string {
  return usesAPIKey(kind) ? "key.fill" : "person.badge.key.fill";
}

export function providerSubtitle(kind: ProviderKind): string {
  return usesAPIKey(kind) ? L("API key") : L("Subscription");
}

/** What the sign-in needs, for the subscription providers. */
export function signInRequirement(kind: ProviderKind): string {
  if (kind === "chatgpt") return L("It needs a ChatGPT subscription.");
  if (kind === "grok") return L("It needs a SuperGrok or X Premium+ subscription.");
  return "";
}

/** The API root the CLI calls unless the credential names another. */
export function defaultBaseURL(kind: ProviderKind): string {
  switch (kind) {
    case "deepseek":
      return "https://api.deepseek.com";
    case "anthropic":
      return "https://api.anthropic.com";
    case "opencode":
      return "https://opencode.ai/zen";
    case "opencode-go":
      return "https://opencode.ai/zen/go";
    default:
      return "";
  }
}

/** Placeholder for the key field, naming where the key comes from. */
export function keyPlaceholder(kind: ProviderKind): string {
  switch (kind) {
    case "deepseek":
      return L("sk-… from platform.deepseek.com");
    case "anthropic":
      return L("sk-ant-… from console.anthropic.com");
    case "opencode":
    case "opencode-go":
      return L("API key from opencode.ai/auth");
    default:
      return "";
  }
}

/** Provider connect methods use underscores even when the stored provider id has a hyphen. */
export function connectMethod(kind: ProviderKind): string {
  return `providers.connect_${kind === "opencode-go" ? "opencode_go" : kind}`;
}

export function thinkingLabel(level: string): string {
  switch (level) {
    case "off":
      return L("Off");
    case "minimal":
      return L("Minimal");
    case "low":
      return L("Low");
    case "medium":
      return L("Medium");
    case "high":
      return L("High");
    case "xhigh":
      return L("Extra high");
    case "max":
      return L("Max");
    default:
      return level.charAt(0).toUpperCase() + level.slice(1);
  }
}

/** The thinking levels this provider's models take, lowest first. None on a bot means the
 * provider's default. */
export function thinkingLevels(kind: ProviderKind): { id: string; label: string }[] {
  const ids: Record<ProviderKind, string[]> = {
    deepseek: ["off", "low", "medium", "high", "xhigh", "max"],
    anthropic: ["off", "minimal", "low", "medium", "high", "xhigh", "max"],
    opencode: ["off", "low", "medium", "high", "xhigh", "max"],
    "opencode-go": ["off", "low", "medium", "high", "xhigh", "max"],
    chatgpt: ["low", "medium", "high", "xhigh", "max"],
    grok: ["low", "medium", "high", "xhigh"],
  };
  return ids[kind].map((id) => ({ id, label: thinkingLabel(id) }));
}

/** Model ids this provider accepts; the first is the default the CLI uses. */
export function providerModels(kind: ProviderKind): { id: string; label: string }[] {
  switch (kind) {
    case "deepseek":
      return [
        { id: "deepseek-flash", label: "V4.1 Flash" },
        { id: "deepseek-v4-pro", label: "V4 Pro (reasoning)" },
      ];
    case "anthropic":
      return [
        { id: "claude-opus-5", label: "Opus 5" },
        { id: "claude-opus-5-5", label: "Opus 5.5" },
        { id: "claude-sonnet-5", label: "Sonnet 5" },
        { id: "claude-fable-5-1", label: "Fable 5.1" },
        { id: "claude-opus-4-8", label: "Opus 4.8" },
        { id: "claude-haiku-4-5", label: "Haiku 4.5" },
      ];
    case "opencode":
      return [
        { id: "deepseek-v4.1-flash", label: "DeepSeek V4.1 Flash" },
        { id: "claude-sonnet-5", label: "Claude Sonnet 5" },
        { id: "gpt-5.6-terra", label: "GPT-5.6 Terra" },
        { id: "grok-4.6", label: "Grok 4.6" },
        { id: "kimi-k3", label: "Kimi K3" },
        { id: "big-pickle", label: "Big Pickle (free)" },
      ];
    case "opencode-go":
      return [
        { id: "glm-5.3-flash", label: "GLM-5.3 Flash" },
        { id: "deepseek-v4.1-flash", label: "DeepSeek V4.1 Flash" },
        { id: "gpt-5.6-luna", label: "GPT-5.6 Luna" },
        { id: "grok-4.6", label: "Grok 4.6" },
        { id: "kimi-k3", label: "Kimi K3" },
        { id: "qwen3.8-flash", label: "Qwen3.8 Flash" },
        { id: "minimax-m3", label: "MiniMax M3" },
      ];
    case "chatgpt":
      return [
        { id: "gpt-6-sol", label: "GPT-6 Sol" },
        { id: "gpt-6-astra", label: "GPT-6 Astra" },
        { id: "gpt-6-luna", label: "GPT-6 Luna" },
      ];
    case "grok":
      return [
        { id: "grok-4.7", label: "Grok 4.7" },
        { id: "grok-4.6", label: "Grok 4.6" },
      ];
  }
}

/** One of the account's provider credentials, as statuses: never a key. */
export interface ProviderCredential {
  kind: ProviderKind;
  isConnected: boolean;
  detail: string;
  /** A custom API root, when the account credential has one. */
  baseURL?: string;
}

// MARK: - Device

export type DeviceOS = "macos" | "linux" | "windows" | "ios" | "ipados" | "android";

export function osDisplayName(os: DeviceOS): string {
  switch (os) {
    case "macos":
      return "macOS";
    case "linux":
      return "Linux";
    case "windows":
      return "Windows";
    case "ios":
      return "iOS";
    case "ipados":
      return "iPadOS";
    case "android":
      return "Android";
  }
}

/** Desktop systems run the agent loop. Phones and tablets never do. */
export function isDesktopOS(os: DeviceOS): boolean {
  return os === "macos" || os === "linux" || os === "windows";
}

export type DeviceStatus = "online" | "offline" | "pairing";

export function deviceStatusLabel(status: DeviceStatus): string {
  switch (status) {
    case "online":
      return L("Online");
    case "offline":
      return L("Offline");
    case "pairing":
      return Lc("Pairing", "device state");
  }
}

/** A paired machine or phone. Its `os` decides whether it is a Runner: only desktop systems run
 * the CLI and get bots assigned. */
export interface Device {
  id: string;
  name: string;
  model: string;
  os: DeviceOS;
  osVersion: string;
  isThisDevice: boolean;
  status: DeviceStatus;
  /** Milliseconds since the epoch. */
  lastSeen: number;
  machineKey: string;
  /** Plugins installed on this Runner, as it advertises them. Secrets stay on the Runner. */
  plugins: InstalledPlugin[];
}

/** Derived from `os` alone: a desktop Device is a Runner and can be assigned bots. */
export function isRunner(device: Device): boolean {
  return isDesktopOS(device.os);
}

export function roleLabel(device: Device): string {
  return isRunner(device) ? L("Runner") : L("Device");
}

/** The symbol for a Device: its kind of machine. */
export function deviceSymbol(device: Device): string {
  switch (device.os) {
    case "macos":
      if (device.model.includes("MacBook")) return "laptopcomputer";
      if (device.model.includes("Studio")) return "macstudio";
      if (device.model.includes("mini")) return "macmini";
      return "desktopcomputer";
    case "linux":
      return "server.rack";
    case "windows":
      return "pc";
    case "ios":
      return "iphone";
    case "ipados":
      return "ipad";
    case "android":
      return "smartphone";
  }
}

// MARK: - Bot

export type Accent = "indigo" | "blue" | "teal" | "green" | "orange" | "pink" | "purple" | "red";

export const accents: Accent[] = ["indigo", "blue", "teal", "green", "orange", "pink", "purple", "red"];

export function isAccent(value: string): value is Accent {
  return (accents as string[]).includes(value);
}

export interface Bot {
  id: string;
  name: string;
  /** What the bot does and how it should work. */
  description: string;
  symbolName: string;
  accent: Accent;
  runnerID: string;
  provider: ProviderKind;
  /** None means the provider's default model. */
  model?: string;
  /** How much the model thinks; none means the provider's default. */
  thinking?: string;
  /** A custom profile image, kept as a `file` blob like a message attachment. Shown in place of
   * the symbol and accent once this computer has the bytes. */
  avatar?: Attachment;
  createdAt: number;
}

// MARK: - Auto-review

/** One Auto-review rule: what a bot wants to do, in the user's words, and whether that runs on
 * its own or asks first. Always allow on a shell command adds the rule Auto-review proposed; on a
 * plugin tool it adds a rule for that exact tool. */
export interface AutoReviewRule {
  id: string;
  text: string;
  behavior: "allow" | "ask";
  /** The exact plugin tool (`github/create_issue`) Always allow saved this rule for. */
  tool?: string;
}

export function behaviorTitle(behavior: AutoReviewRule["behavior"]): string {
  return behavior === "allow" ? L("Allow automatically") : L("Ask first");
}

/** The check on effectful plugin actions and shell commands, shared by every Device through the
 * roster: on, a small model on the bot's provider asks only when needed; off, each one asks. */
export interface AutoReview {
  isEnabled: boolean;
  rules: AutoReviewRule[];
}

// MARK: - Plugins

export type PluginState = "ready" | "needs_setup" | "needs_auth" | "connecting" | "error" | "unknown";

/** A plugin as its Runner advertises it: installed, and in what state. */
export interface InstalledPlugin {
  id: string;
  name: string;
  description: string;
  version: string;
  icon: string;
  state: PluginState;
  detail: string;
}

export function pluginSymbol(plugin: { icon: string }): string {
  return plugin.icon || "puzzlepiece.extension";
}

/** The color a plugin's state takes, as a CSS color token. */
export function pluginStateColor(state: PluginState): string {
  switch (state) {
    case "ready":
      return "var(--green)";
    case "connecting":
      return "var(--accent)";
    case "error":
      return "var(--red)";
    default:
      return "var(--orange)";
  }
}

export interface MarketplaceServer {
  name: string;
  /** The URL of a remote server, or the command a local one runs on the Runner. */
  address: string;
  isRemote: boolean;
  signsIn: boolean;
}

/** A marketplace plugin, with the Runners that already have it. */
export interface MarketplacePlugin {
  id: string;
  name: string;
  description: string;
  icon: string;
  homepage?: string;
  /** Who makes it. */
  author: string;
  category: string;
  isFeatured: boolean;
  tags: string[];
  servers: MarketplaceServer[];
  skills: { name: string; description: string }[];
  variables: { name: string; description: string; secret: boolean; required: boolean }[];
  installedOn: string[];
}

/** At least one server signs in with OAuth on the Runner. */
export function signsIn(plugin: MarketplacePlugin): boolean {
  return plugin.servers.some((server) => server.signsIn);
}

/** A bot to add from the marketplace: the profile it starts with, the plugins it works with, the
 * routines it brings (added paused), and facts it starts out knowing. */
export interface BotTemplate {
  id: string;
  name: string;
  /** One line for the marketplace's rows. */
  summary: string;
  /** What the bot does and how it should work: the new bot's description. */
  description: string;
  symbolName: string;
  accent: Accent;
  category: string;
  isFeatured: boolean;
  author: string;
  plugins: string[];
  routines: { name: string; scheduleText: string; prompt: string }[];
  memory: string[];
}

/** What the marketplace offers, in the index's order. */
export interface Marketplace {
  plugins: MarketplacePlugin[];
  bots: BotTemplate[];
}

/** One installed plugin in full, as its Runner reports it: never a secret's value. */
export interface PluginDetail {
  status: InstalledPlugin;
  homepage?: string;
  variables: { name: string; description: string; secret: boolean; required: boolean; isSet: boolean; value?: string }[];
  servers: {
    name: string;
    kind: string;
    url?: string;
    oauth: boolean;
    signedIn: boolean;
    /** While a device-flow sign-in waits: the code to enter, and the page to enter it on. */
    code?: string;
    link?: string;
  }[];
  skills: { name: string; description: string }[];
}

// MARK: - Permission

export type PermissionDecision = "pending" | "allowed" | "always" | "denied" | "expired" | "dismissed" | "connected" | "failed";

/** A bot asking before a plugin or shell action runs, or before a plugin is installed. */
export interface PermissionRequest {
  pluginID: string;
  pluginName: string;
  tool: string;
  summary: string;
  decision: PermissionDecision;
  /** A sign-in mid-flow: where to go and the code to enter there. */
  link?: string;
  code?: string;
  /** Why Auto-review paused the action, when it did. */
  reason?: string;
  /** The rule Always allow adds, which Auto-review proposed for a shell command. */
  rule?: string;
  /** A shell card's whole command, where `summary` is its first line. */
  command?: string;
}

/** The command as the card and its sheet show it, without the summary's `$ ` prompt. */
export function fullCommand(request: PermissionRequest): string {
  return request.command ?? (request.summary.startsWith("$ ") ? request.summary.slice(2) : request.summary);
}

export const isPending = (request: PermissionRequest) => request.decision === "pending";
export const isInstall = (request: PermissionRequest) => request.tool === "install";
/** A shell command on the bot's Runner. */
export const isShell = (request: PermissionRequest) => request.pluginID === "computer";
/** A sign-in card: Sign in starts the OAuth flow on the Runner. */
export const isConnect = (request: PermissionRequest) => request.tool === "connect";

/** "wants to use GitHub" / "wants to install GitHub" / "needs a sign-in to GitHub" /
 * "wants to run a command on Workbench" */
export function verbPhrase(request: PermissionRequest): string {
  if (isConnect(request)) return L("needs a sign-in to %@", request.pluginName);
  if (isShell(request)) return L("wants to run a command on %@", request.pluginName);
  return isInstall(request) ? L("wants to install %@", request.pluginName) : L("wants to use %@", request.pluginName);
}

export function decisionText(request: PermissionRequest): string {
  switch (request.decision) {
    case "pending":
      return L("Waiting for you");
    case "allowed":
      return isConnect(request) ? L("Signing in") : L("Allowed once");
    case "always":
      return L("Always allowed");
    case "denied":
      return isConnect(request) ? L("Not now") : L("Denied");
    case "expired":
      return L("No answer in time");
    case "dismissed":
      return L("Dismissed");
    case "connected":
      return L("Signed in");
    case "failed":
      return L("Sign-in failed");
  }
}

/** The buttons a pending card offers: [title, decision]. A shell command offers Always allow only
 * with a rule to add. */
export function permissionChoices(request: PermissionRequest): [string, string][] {
  if (isConnect(request)) return [[L("Sign in"), "allow"], [L("Not now"), "deny"]];
  if (isInstall(request)) return [[L("Allow"), "allow"], [L("Deny"), "deny"]];
  if (isShell(request) && request.rule === undefined) return [[L("Allow once"), "allow"], [L("Deny"), "deny"]];
  return [
    [L("Allow once"), "allow"],
    [L("Always allow"), "always"],
    [L("Deny"), "deny"],
  ];
}

// MARK: - Memory

/** A bot's memory as its Runner reports it: the curated index with its load budget, and the other
 * files by name. `here` is false when the bot runs elsewhere and only `runner` is known. */
export interface BotMemory {
  botID: string;
  here: boolean;
  runner: string;
  path: string;
  text: string;
  hash: string;
  lines: number;
  bytes: number;
  truncated: boolean;
  maxLines: number;
  maxBytes: number;
  topics: string[];
  logs: string[];
}

/** "12 lines · 1.2 KB of 24 KB", or "Empty". */
export function memoryBudgetSummary(memory: BotMemory): string {
  if (memory.lines <= 0) return L("Empty");
  return memory.lines === 1
    ? L("%d line · %@ of %@", memory.lines, Format.kilobytes(memory.bytes), Format.kilobytes(memory.maxBytes))
    : L("%d lines · %@ of %@", memory.lines, Format.kilobytes(memory.bytes), Format.kilobytes(memory.maxBytes));
}

/** "2 topics · 5 days of logs" */
export function memoryFilesSummary(memory: BotMemory): string {
  const parts: string[] = [];
  if (memory.topics.length > 0) parts.push(memory.topics.length === 1 ? L("%d topic", 1) : L("%d topics", memory.topics.length));
  if (memory.logs.length > 0) parts.push(memory.logs.length === 1 ? L("%d day of logs", 1) : L("%d days of logs", memory.logs.length));
  return parts.length === 0 ? L("No other notes yet") : parts.join(" · ");
}

// MARK: - Routine

/** A recurring task a bot runs on a schedule in its direct chat, as the roster carries it. The
 * schedule's words, the next run, and the running state come from the CLI. */
export interface Routine {
  id: string;
  botID: string;
  name: string;
  /** The task, written to the bot, handed to it on every run. */
  prompt: string;
  /** `every 30m`, `every 2h`, `every 1d`, or five cron fields in the Runner's local time. */
  schedule: string;
  /** The schedule in words: "Weekdays at 9:00 AM". */
  scheduleText: string;
  isEnabled: boolean;
  /** Why Lorca paused it, when it did: "away". */
  pausedReason?: string;
  lastRunAt?: number;
  /** How the last run ended: "sent", "pass", or "error". */
  lastOutcome?: string;
  nextRunAt?: number;
  isRunning: boolean;
  /** The script the Runner runs at each due time before the bot does; the bot runs only when it
   * finds something. `nextRunAt` is then the next check. */
  check?: string;
  createdAt: number;
}

/** The line under the name in the inspector: the schedule, then what is going on. */
export function routineDetail(routine: Routine): string {
  if (routine.isRunning) return L("%@ · Running…", routine.scheduleText);
  if (!routine.isEnabled) {
    return routine.pausedReason === "away" ? L("%@ · Paused while you were away", routine.scheduleText) : L("%@ · Paused", routine.scheduleText);
  }
  if (routine.nextRunAt !== undefined) {
    const next = Format.upcoming(routine.nextRunAt);
    return routine.check === undefined ? L("%@ · Next %@", routine.scheduleText, next) : L("%@ · Next check %@", routine.scheduleText, next);
  }
  return routine.scheduleText;
}

/** "Today 9:00 AM · replied", "Never", "Yesterday 6:00 PM · nothing to report". */
export function lastRunSummary(routine: Routine): string {
  if (routine.lastRunAt === undefined) return L("Never");
  const when = Format.daySeparator(routine.lastRunAt);
  switch (routine.lastOutcome) {
    case "sent":
      return L("%@ · replied", when);
    case "pass":
      return L("%@ · nothing to report", when);
    case "error":
      return L("%@ · failed", when);
    default:
      return when;
  }
}

// MARK: - Message

export type CommandState = "checking" | "asking" | "running" | "waiting" | "exited" | "failed" | "stopped" | "denied" | "expired" | "dismissed";

/** Where a shell command stands: Auto-review checking it, the question it asks, the command running
 * in its terminal, what the command asks, and that it ended. While it asks, its card takes the
 * answer to the question (`chats.permission`); once the bot handed the running command over, the
 * user's answer to it (`bash.stdin`) and a Stop (`bash.stop`). */
export interface CommandRun {
  /** The terminal session running it, once one does. None before it starts, and on a Windows
   * Runner, where a command runs on pipes and takes no answers. */
  sessionID?: string;
  /** The command, its first 8,000 characters. */
  command: string;
  state: CommandState;
  /** The line it asks with: "[sudo] password for ana:". */
  prompt?: string;
  /** Its last lines, as the bottom of a terminal shows them. Never what was typed. */
  output?: string;
  /** The Runner it runs on, for the question: "Workbench". */
  device?: string;
  /** Why Auto-review asked. */
  reason?: string;
  /** The rule Always allow adds. */
  rule?: string;
  /** The bot left the command to the user: its turn ended with the command still running, or it
   * waits on the command at a question. */
  handedOver: boolean;
}

export const isLive = (run: CommandRun) => run.state === "waiting" || run.state === "running";
/** The command runs in a session here or on its Runner: it takes answers and a Stop. */
export const takesInput = (run: CommandRun) => isLive(run) && run.sessionID !== undefined;
/** It ran and ended: by itself, with a nonzero code, or stopped. */
export const hasEnded = (run: CommandRun) => run.state === "exited" || run.state === "failed" || run.state === "stopped";

/** The command's first line with anything on it. */
export function firstLine(run: { command: string }): string {
  return (
    run.command
      .split("\n")
      .map((line) => line.trim())
      .find((line) => line.length > 0) ?? run.command
  );
}

/** Whether what the user types should show as they type it: a yes-or-no question. Anything else
 * may be a secret. */
export function asksYesOrNo(run: CommandRun): boolean {
  const prompt = run.prompt?.toLowerCase();
  if (!prompt) return false;
  return prompt.includes("[y/n]") || prompt.includes("(y/n)") || prompt.includes("(yes/no");
}

/** The buttons the question offers: [title, decision]. Always allow only with a rule to add. */
export function commandChoices(run: CommandRun): [string, string][] {
  return run.rule === undefined
    ? [
        [L("Allow once"), "allow"],
        [L("Deny"), "deny"],
      ]
    : [
        [L("Allow once"), "allow"],
        [L("Always allow"), "always"],
        [L("Deny"), "deny"],
      ];
}

export interface ToolInvocation {
  name: string;
  summary: string;
  detail: string;
  isRunning: boolean;
  /** What the call does, in the bot's words: a shell command's "Install dependencies". */
  description?: string;
  /** The bot a message_bot call goes to. */
  targetBotID?: string;
  /** A shell command's card, which the transcript shows only while the command needs the user
   * (`isShown`). Every `bash` row has one. */
  run?: CommandRun;
}

/** A finished message_bot call: the one tool the transcript shows, as "Messaged ◉ Name". */
export function isSentMessage(tool: ToolInvocation): boolean {
  return tool.name === "message_bot" && !tool.isRunning && tool.summary.startsWith("Messaged ");
}

/** Whether the transcript shows the row: a sent message's marker, or a command's card while the
 * command needs the user. That is while Auto-review asks to run it, and once the bot handed the
 * running command over (its turn ended, or it waits on the command at a question) until it ends.
 * Before that the bot deals with it, and the command shows only as the working row's activity. */
export function isShown(tool: ToolInvocation): boolean {
  const run = tool.run;
  if (!run) return isSentMessage(tool);
  return run.state === "asking" || (isLive(run) && run.handedOver);
}

/** A file sent with a message. The bytes live under `~/.lorca/files/<id>` once this Device has
 * them; `width` and `height` size an image's thumbnail before the file arrives. */
export interface Attachment {
  id: string;
  name: string;
  mime: string;
  size: number;
  width?: number;
  height?: number;
}

export const isImage = (attachment: Attachment) => attachment.mime.startsWith("image/");

/** "Photo", "3 photos", "report.pdf", "2 files": the preview of a message with no text. */
export function attachmentSummary(attachments: Attachment[]): string {
  const first = attachments[0];
  if (!first) return "";
  if (attachments.length === 1) return isImage(first) ? L("Photo") : first.name;
  return attachments.every(isImage) ? L("%d photos", attachments.length) : L("%d files", attachments.length);
}

export function sizeText(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export type Author = { kind: "you" } | { kind: "bot"; botID: string } | { kind: "system" };

export const you: Author = { kind: "you" };
export const system: Author = { kind: "system" };

export function authorBotID(author: Author): string | undefined {
  return author.kind === "bot" ? author.botID : undefined;
}

export function sameAuthor(a: Author | undefined, b: Author | undefined): boolean {
  if (!a || !b || a.kind !== b.kind) return false;
  return a.kind !== "bot" || a.botID === (b as { botID: string }).botID;
}

export type Body =
  | { kind: "text"; text: string }
  | { kind: "tool"; tool: ToolInvocation }
  | { kind: "handoff"; from: string; to: string; reason: string }
  | { kind: "notice"; text: string }
  | { kind: "permission"; request: PermissionRequest };

export type MessageState = { kind: "thinking" } | { kind: "streaming" } | { kind: "complete" } | { kind: "failed"; error: string };

export interface Message {
  id: string;
  author: Author;
  body: Body;
  state: MessageState;
  createdAt: number;
  /** Files sent with a text body; other bodies carry none. */
  attachments: Attachment[];
}

export function newMessageID(): string {
  return `msg-${crypto.randomUUID().toLowerCase()}`;
}

/** A `bash` row's command. */
export function commandRunOf(message: Message): CommandRun | undefined {
  return message.body.kind === "tool" ? message.body.tool.run : undefined;
}

export function messageText(message: Message): string {
  const body = message.body;
  switch (body.kind) {
    case "text":
      return body.text;
    case "tool":
      return body.tool.summary;
    case "handoff":
      return body.reason;
    case "notice":
      return body.text;
    case "permission":
      return body.request.summary;
  }
}

// MARK: - Chat

export const maxGroupBots = 6;

/** A DM is a fixed one-to-one thread with a single bot. A group holds one to six bots and can gain
 * or lose members after it is created. */
export interface Chat {
  id: string;
  kind: "dm" | "group";
  customTitle?: string;
  botIDs: string[];
  messages: Message[];
  unreadCount: number;
  isPinned: boolean;
  createdAt: number;
  /** What the turns run in this chat used, from the Runner that ran them. */
  usage?: ChatUsage;
  /** The CLI holds messages older than the ones here; the transcript asks for them by page. */
  hasMore: boolean;
}

export const isGroup = (chat: Chat) => chat.kind === "group";
export const isDM = (chat: Chat) => chat.kind === "dm";
/** Whether another bot may join. Only groups grow, and never past the cap. */
export const canAddBot = (chat: Chat) => isGroup(chat) && chat.botIDs.length < maxGroupBots;
/** Whether a bot may leave. Groups keep at least one bot; DMs never change. */
export const canRemoveBot = (chat: Chat) => isGroup(chat) && chat.botIDs.length > 1;

export function lastActivity(chat: Chat): number {
  return chat.messages[chat.messages.length - 1]?.createdAt ?? chat.createdAt;
}

// MARK: - Usage

/** Tokens and money the turns in a chat used. `contextTokens` and `contextWindow` are the last
 * turn's; the rest accumulate. */
export interface ChatUsage {
  contextTokens: number;
  contextWindow: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  costUSD: number;
  turns: number;
  model: string;
}

/** "128k of 1M · 13%", or "128k" when the window is unknown. */
export function contextSummary(usage: ChatUsage): string {
  if (usage.contextWindow <= 0) return Format.tokens(usage.contextTokens);
  const percent = Math.round((usage.contextTokens / usage.contextWindow) * 100);
  return L("%@ of %@ · %d%%", Format.tokens(usage.contextTokens), Format.tokens(usage.contextWindow), percent);
}

/** "$0.42 · 18 turns" */
export function spendSummary(usage: ChatUsage): string {
  const dollars = usage.costUSD < 0.01 && usage.costUSD > 0 ? "<$0.01" : `$${usage.costUSD.toFixed(2)}`;
  return usage.turns === 1 ? L("%@ · %d turn", dollars, usage.turns) : L("%@ · %d turns", dollars, usage.turns);
}

// MARK: - Settings

export type SettingsPane = "general" | "providers" | "auto-review" | "plugins" | "bots" | "device" | "advanced";

export const settingsPanes: SettingsPane[] = ["general", "providers", "auto-review", "plugins", "bots", "device", "advanced"];

/** Bots and plugins live on a Runner, so these panes show one Device, picked at the top of the
 * page. The others hold this computer's settings and the account's. */
export function isDeviceScoped(pane: SettingsPane): boolean {
  return pane === "bots" || pane === "plugins" || pane === "device";
}

export function paneTitle(pane: SettingsPane): string {
  switch (pane) {
    case "general":
      return L("General");
    case "auto-review":
      return L("Auto-review");
    case "advanced":
      return L("Advanced");
    case "bots":
      return L("Bots");
    case "providers":
      return L("Providers");
    case "plugins":
      return L("Plugins");
    case "device":
      return L("Devices");
  }
}

export function paneSymbol(pane: SettingsPane): string {
  switch (pane) {
    case "general":
      return "gearshape";
    case "auto-review":
      return "checkmark.shield";
    case "advanced":
      return "slider.horizontal.3";
    case "bots":
      return "person.2";
    case "providers":
      return "key";
    case "plugins":
      return "puzzlepiece.extension";
    case "device":
      return "desktopcomputer";
  }
}

export type Selection = { kind: "chat"; id: string } | { kind: "settings"; pane: SettingsPane };

export function encodeSelection(selection: Selection | null): string {
  if (!selection) return "";
  return selection.kind === "chat" ? `chat:${selection.id}` : `settings:${selection.pane}`;
}

export function decodeSelection(raw: string): Selection | null {
  const at = raw.indexOf(":");
  if (at < 0) return null;
  const kind = raw.slice(0, at);
  const value = raw.slice(at + 1);
  if (kind === "chat" && value) return { kind: "chat", id: value };
  if (kind === "settings" && (settingsPanes as string[]).includes(value)) return { kind: "settings", pane: value as SettingsPane };
  return null;
}

export function sameSelection(a: Selection | null, b: Selection | null): boolean {
  return encodeSelection(a) === encodeSelection(b);
}
