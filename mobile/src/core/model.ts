// The domain model as the core reports it over the JSON API (crates/cli/src/model.rs and the
// snapshot in app.rs). Field names are the wire names.

import { t } from "../i18n";

export const MAX_GROUP_BOTS = 6;

export interface ProviderStatus {
  kind: ProviderKind | string;
  is_connected: boolean;
  detail: string;
  base_url?: string;
}

/// A Device as the core's snapshot describes it: presence already resolved.
export interface Device {
  /// The machine signing public key, base64url.
  id: string;
  name: string;
  model: string;
  os: string;
  os_version: string;
  machine_key: string;
  is_this_device: boolean;
  status: "online" | "offline";
  /// Unix seconds.
  last_seen: number;
  /// Plugins installed on that Runner, with their setup state.
  plugins?: PluginStatus[];
}

/// A plugin as its Runner advertises it: installed, and in what state.
export interface PluginStatus {
  id: string;
  name: string;
  description?: string;
  version?: string;
  /// An SF Symbol name.
  icon?: string;
  state: "ready" | "needs_setup" | "needs_auth" | "connecting" | "error";
  detail?: string;
}

/// The kinds the account has connected.
export function connectedProviders(providers: ProviderStatus[]): string[] {
  return providers.filter((p) => p.is_connected).map((p) => p.kind);
}

export function isRunner(device: Device): boolean {
  return device.os === "macos" || device.os === "linux" || device.os === "windows";
}

export type Accent = "indigo" | "blue" | "teal" | "green" | "orange" | "pink" | "purple" | "red";

export interface Bot {
  id: string;
  name: string;
  /** What the bot does and how it should work. */
  description: string;
  /** SF Symbol on the accent gradient: the look when there is no image. */
  symbol_name: string;
  accent: Accent | string;
  /** A custom profile image, a `file` blob like a message attachment; shown in place of the symbol and accent. */
  avatar?: Attachment;
  runner_id: string;
  provider: string;
  model?: string;
  /** How much the model thinks: off, minimal, low, medium, high, xhigh, max. */
  thinking?: string;
  workdir?: string;
  created_at: number;
}

/// A recurring task a bot runs on a schedule in its direct chat, as the roster carries it, with
/// the schedule in words, the next run, and the running state resolved by the core.
export interface Routine {
  id: string;
  bot_id: string;
  name: string;
  /** The task, written to the bot, handed to it on every run. */
  prompt: string;
  /** `every 30m`, `every 2h`, `every 1d`, or five cron fields in the Runner's local time. */
  schedule: string;
  /** "Weekdays at 9:00 AM" */
  schedule_text: string;
  is_enabled: boolean;
  /** Why Lorca paused it, when it did: "away". */
  paused_reason?: string;
  last_run_at?: number;
  /** "sent", "pass", or "error". */
  last_outcome?: string;
  next_run_at?: number | null;
  is_running: boolean;
  created_at: number;
}

export type Author = { kind: "you" } | { kind: "bot"; bot_id: string } | { kind: "system" };

export interface ChatSearchResults {
  chats: { chat_id: string; snippet: string }[];
  messages: { chat_id: string; message_id: string; snippet: string; author: Author; created_at: number }[];
}

/// A file sent with a message. Its bytes travel as a `file` blob under this id, encrypted with
/// the account key; the phone keeps a copy in its documents directory once it has them.
export interface Attachment {
  id: string;
  name: string;
  mime: string;
  size: number;
  width?: number;
  height?: number;
}

export function isImage(attachment: Attachment): boolean {
  return attachment.mime.startsWith("image/");
}

/// "Photo", "3 photos", "report.pdf", "2 files": the preview of a message with no text.
export function attachmentSummary(attachments: Attachment[]): string {
  const first = attachments[0];
  if (!first) return "";
  if (attachments.length === 1) return isImage(first) ? t("Photo") : first.name;
  return attachments.every(isImage) ? t("{count} photos", { count: attachments.length }) : t("{count} files", { count: attachments.length });
}

export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export const MAX_ATTACHMENT_BYTES = 100 * 1024 * 1024;
export const MAX_ATTACHMENTS = 10;

export type Body =
  | { kind: "text"; text: string; attachments?: Attachment[] }
  | {
      kind: "tool";
      name: string;
      summary: string;
      detail: string;
      is_running: boolean;
      call_id?: string;
      arguments?: unknown;
      result?: string | null;
      is_error?: boolean;
      /** What the call does, in the bot's words: a shell command's description. */
      description?: string;
      /** The bot a message_bot call goes to. */
      target_bot_id?: string;
      /** A bash call's card, from Auto-review's question to how the command ended. */
      run?: CommandRun;
    }
  | { kind: "handoff"; from: string; to: string; reason: string }
  | { kind: "notice"; text: string; routine_id?: string }
  /// The bot asks before a plugin or shell action, or before installing a plugin (`tool` is `install`).
  /// `rule` is the rule Always allow adds, which Auto-review proposed for a shell command (`plugin_id` is `computer`);
  /// `command` is that command in full, where `summary` is its first line.
  | { kind: "permission"; plugin_id: string; plugin_name: string; tool: string; summary: string; decision: "pending" | "allowed" | "always" | "denied" | "expired" | "dismissed" | "connected" | "failed"; reason?: string; rule?: string; command?: string; link?: string; code?: string };

/// Where a bash call's command stands: Auto-review checking it, the question it asks, the command
/// running in its terminal, what the command asks, and that it ended. While it asks, its card takes
/// the answer to the question (`chats.permission`); once the bot handed the running command over,
/// the user's answer to it (`bash.stdin`) and a Stop (`bash.stop`). The transcript shows the card
/// only while the command needs the user (`showsCard`).
export interface CommandRun {
  /** The terminal session running it, once one does; none before it starts, or on a Windows Runner, where it takes no answers. */
  session_id?: string;
  /** The command, its first 8,000 characters. */
  command: string;
  /**
   * `checking` (Auto-review is judging it), `asking` (for the user's permission), `running` (printing, or quiet), or
   * `waiting` (at a question the user can read); once it ended, `exited`, `failed`, `stopped`, `denied` (never run),
   * `expired` (nobody answered in time), or `dismissed` (the user sent a new message instead of answering, so it never ran).
   */
  state: "checking" | "asking" | "running" | "waiting" | "exited" | "failed" | "stopped" | "denied" | "expired" | "dismissed";
  /** The line it asks with while it waits: "[sudo] password for ana:". */
  prompt?: string;
  /** The bot left the command to the user: its turn ended with the command still running, or it waits on the command at a question. */
  handed_over?: boolean;
  /** Its last lines, as the bottom of a terminal shows them. Never what was typed. */
  output?: string;
  /** The Runner it runs on, for the question: "Workbench". */
  device?: string;
  /** Why Auto-review asked. */
  reason?: string;
  /** The rule Always allow adds. */
  rule?: string;
}

export function isLive(run: CommandRun): boolean {
  return run.state === "waiting" || run.state === "running";
}

/// It ran and ended: by itself, with a nonzero code, or stopped.
export function hasEnded(run: CommandRun): boolean {
  return run.state === "exited" || run.state === "failed" || run.state === "stopped";
}

/// A `bash` row whose command runs in its terminal, here or on its Runner: it takes answers and a
/// Stop. A command Auto-review is still judging has no terminal yet.
export function runsInTerminal(message: Message | undefined): boolean {
  const run = message?.body.kind === "tool" ? message.body.run : undefined;
  return !!run && isLive(run) && !!run.session_id;
}

/// Whether a tool row shows as a command's card: while the command needs the user. That is while
/// Auto-review asks to run it, and once the bot handed the running command over (its turn ended, or
/// it waits on the command at a question) until it ends. Before that the bot deals with it, and the
/// command shows only as the working row's activity.
export function showsCard(tool: Extract<Body, { kind: "tool" }>): boolean {
  const run = tool.run;
  return !!run && (run.state === "asking" || (isLive(run) && !!run.handed_over));
}

/// One Auto-review rule: what a bot wants to do, in the user's words, and whether that runs on
/// its own or asks first. Always allow on a shell command adds the rule Auto-review proposed; on
/// a plugin tool it adds a rule for that exact tool (`tool`).
export type AutoReviewRule = { id: string; text: string; behavior: "allow" | "ask"; tool?: string };

/// The check on effectful plugin actions and shell commands, shared through the roster.
export type AutoReview = { is_enabled: boolean; rules: AutoReviewRule[] };

export type MessageState =
  | { kind: "thinking" }
  | { kind: "streaming" }
  | { kind: "complete" }
  | { kind: "failed"; error: string };

export interface Message {
  id: string;
  chat_id: string;
  author: Author;
  body: Body;
  state: MessageState;
  /// Unix seconds.
  created_at: number;
  /** Later model-context position when this message steered work already in flight. */
  promoted_at?: number;
}

export function isComplete(message: Message): boolean {
  return message.state.kind === "complete";
}

/// A finished message_bot call: the one tool the transcript shows, as "Messaged ◉ Name".
export function isSentMessage(body: Body): body is Extract<Body, { kind: "tool" }> {
  return body.kind === "tool" && body.name === "message_bot" && !body.is_running && body.summary.startsWith("Messaged ");
}

export interface ChatMeta {
  id: string;
  /// `dm` or `group`.
  kind: "dm" | "group";
  title?: string | null;
  bot_ids: string[];
  owner_bot_id?: string;
  is_pinned: boolean;
  created_at: number;
}

/// The tokens and money the turns in a chat used, from its Runner.
export interface ChatUsage {
  context_tokens: number;
  context_window: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cost_usd: number;
  turns: number;
  model: string;
  updated_at: number;
}

export interface Chat extends ChatMeta {
  messages: Message[];
  /// The core holds messages older than these; the transcript asks for them by page.
  has_more?: boolean;
  unread_count: number;
  usage?: ChatUsage;
}

export const PROVIDER_LABELS: Record<string, string> = {
  deepseek: "DeepSeek",
  anthropic: "Anthropic",
  opencode: "OpenCode Zen",
  "opencode-go": "OpenCode Go",
  chatgpt: "ChatGPT",
  grok: "Grok",
};

export const PROVIDER_KINDS = ["deepseek", "anthropic", "opencode", "opencode-go", "chatgpt", "grok"] as const;
export type ProviderKind = (typeof PROVIDER_KINDS)[number];

export function isProviderKind(kind: string): kind is ProviderKind {
  return (PROVIDER_KINDS as readonly string[]).includes(kind);
}

export function providerUsesAPIKey(kind: ProviderKind): boolean {
  return kind !== "chatgpt" && kind !== "grok";
}

export function providerConnectMethod(kind: ProviderKind): string {
  return `providers.connect_${kind === "opencode-go" ? "opencode_go" : kind}`;
}

export function providerDefaultBaseURL(kind: ProviderKind): string {
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

export function providerKeyPlaceholder(kind: ProviderKind): string {
  switch (kind) {
    case "deepseek":
      return t("sk-… from platform.deepseek.com");
    case "anthropic":
      return t("sk-ant-… from console.anthropic.com");
    case "opencode":
    case "opencode-go":
      return t("API key from opencode.ai/auth");
    default:
      return "";
  }
}

export function providerSignInRequirement(kind: ProviderKind): string {
  if (kind === "chatgpt") return t("It needs a ChatGPT subscription.");
  if (kind === "grok") return t("It needs a SuperGrok or X Premium+ subscription.");
  return "";
}

export function providerLabel(kind: string): string {
  return PROVIDER_LABELS[kind] ?? kind;
}

/** Models offered for each provider, in the same order as the desktop app. */
export const PROVIDER_MODELS: Record<string, { id: string; label: string }[]> = {
  deepseek: [
    { id: "deepseek-flash", label: "V4.1 Flash" },
    { id: "deepseek-v4-pro", label: "V4 Pro (reasoning)" },
  ],
  anthropic: [
    { id: "claude-opus-5", label: "Opus 5" },
    { id: "claude-opus-5-5", label: "Opus 5.5" },
    { id: "claude-sonnet-5", label: "Sonnet 5" },
    { id: "claude-fable-5-1", label: "Fable 5.1" },
    { id: "claude-opus-4-8", label: "Opus 4.8" },
    { id: "claude-haiku-4-5", label: "Haiku 4.5" },
  ],
  opencode: [
    { id: "deepseek-v4.1-flash", label: "DeepSeek V4.1 Flash" },
    { id: "claude-sonnet-5", label: "Claude Sonnet 5" },
    { id: "gpt-5.6-terra", label: "GPT-5.6 Terra" },
    { id: "grok-4.6", label: "Grok 4.6" },
    { id: "kimi-k3", label: "Kimi K3" },
    { id: "big-pickle", label: "Big Pickle (free)" },
  ],
  "opencode-go": [
    { id: "glm-5.3-flash", label: "GLM-5.3 Flash" },
    { id: "deepseek-v4.1-flash", label: "DeepSeek V4.1 Flash" },
    { id: "gpt-5.6-luna", label: "GPT-5.6 Luna" },
    { id: "grok-4.6", label: "Grok 4.6" },
    { id: "kimi-k3", label: "Kimi K3" },
    { id: "qwen3.8-flash", label: "Qwen3.8 Flash" },
    { id: "minimax-m3", label: "MiniMax M3" },
  ],
  chatgpt: [
    { id: "gpt-6-sol", label: "GPT-6 Sol" },
    { id: "gpt-6-astra", label: "GPT-6 Astra" },
    { id: "gpt-6-luna", label: "GPT-6 Luna" },
  ],
  grok: [
    { id: "grok-4.7", label: "Grok 4.7" },
    { id: "grok-4.6", label: "Grok 4.6" },
  ],
};

/** The thinking levels a provider's models take, lowest first. */
export const THINKING_LEVELS: Record<string, string[]> = {
  deepseek: ["off", "low", "medium", "high", "xhigh", "max"],
  anthropic: ["off", "minimal", "low", "medium", "high", "xhigh", "max"],
  opencode: ["off", "low", "medium", "high", "xhigh", "max"],
  "opencode-go": ["off", "low", "medium", "high", "xhigh", "max"],
  chatgpt: ["low", "medium", "high", "xhigh", "max"],
  grok: ["low", "medium", "high", "xhigh"],
};

export function thinkingLabel(level: string): string {
  const labels: Record<string, string> = {
    off: t("Off"),
    minimal: t("Minimal"),
    low: t("Low"),
    medium: t("Medium"),
    high: t("High"),
    xhigh: t("Extra high"),
    max: t("Max"),
  };
  return labels[level] ?? level.charAt(0).toUpperCase() + level.slice(1);
}
