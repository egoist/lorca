// The domain model as the core reports it over the JSON API (crates/cli/src/model.rs and the
// snapshot in app.rs). Field names are the wire names.

export const MAX_GROUP_BOTS = 6;
export const ONLINE_WINDOW_SECS = 150;

export interface ProviderStatus {
  kind: string;
  is_connected: boolean;
  detail: string;
  base_url?: string;
}

/// A Device as the core's snapshot describes it: presence and providers already resolved.
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
  providers: ProviderStatus[];
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

export function connectedProviders(device: Device): string[] {
  return device.providers.filter((p) => p.is_connected).map((p) => p.kind);
}

export function isRunner(device: Device): boolean {
  return device.os === "macos" || device.os === "linux" || device.os === "windows";
}

export type Accent = "indigo" | "blue" | "teal" | "green" | "orange" | "pink" | "purple" | "red";

export interface Bot {
  id: string;
  name: string;
  /** One short line under the name: what the bot is for. */
  label: string;
  /** A sentence or two about the bot. */
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
  instructions: string;
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
  /** Why Tinybot paused it, when it did: "away". */
  paused_reason?: string;
  last_run_at?: number;
  /** "sent", "pass", or "error". */
  last_outcome?: string;
  next_run_at?: number | null;
  is_running: boolean;
  created_at: number;
}

export type Author = { kind: "you" } | { kind: "bot"; bot_id: string } | { kind: "system" };

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
  if (attachments.length === 1) return isImage(first) ? "Photo" : first.name;
  return attachments.every(isImage) ? `${attachments.length} photos` : `${attachments.length} files`;
}

export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export const MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024;
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
    }
  | { kind: "handoff"; from: string; to: string; reason: string }
  | { kind: "notice"; text: string; routine_id?: string }
  /// The bot asks before a plugin tool runs, or before installing a plugin (`tool` is `install`).
  | { kind: "permission"; plugin_id: string; plugin_name: string; tool: string; summary: string; decision: "pending" | "allowed" | "always" | "denied" | "expired" | "connected" | "failed"; reason?: string; link?: string; code?: string };

/// One Auto-review rule: what a bot wants to do, in the user's words, and whether that runs on
/// its own or asks first. A rule from a card's Always allow also names the exact `plugin/tool`.
export type AutoReviewRule = { id: string; text: string; behavior: "allow" | "ask"; tool?: string };

/// The check on plugin actions that change something, shared through the roster.
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
}

export function isComplete(message: Message): boolean {
  return message.state.kind === "complete";
}

/// A finished message_bot call: the one tool the transcript shows, as "Messaged ◉ Name".
export function isSentMessage(body: Body): body is Extract<Body, { kind: "tool" }> {
  return body.kind === "tool" && body.name === "message_bot" && !body.is_running && body.summary.startsWith("Messaged ");
}

export function recipientName(body: Extract<Body, { kind: "tool" }>): string {
  return body.summary.slice("Messaged ".length);
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
  unread_count: number;
  usage?: ChatUsage;
}

export const PROVIDER_LABELS: Record<string, string> = {
  deepseek: "DeepSeek",
  anthropic: "Anthropic",
  chatgpt: "ChatGPT",
};

export function providerLabel(kind: string): string {
  return PROVIDER_LABELS[kind] ?? kind;
}

/** The thinking levels a provider's models take, lowest first. */
export const THINKING_LEVELS: Record<string, string[]> = {
  deepseek: ["off", "low", "medium", "high", "xhigh", "max"],
  anthropic: ["off", "minimal", "low", "medium", "high", "xhigh", "max"],
  chatgpt: ["low", "medium", "high", "xhigh"],
};

export function thinkingLabel(level: string): string {
  return level === "xhigh" ? "Extra high" : level.charAt(0).toUpperCase() + level.slice(1);
}
