// The domain model as the core reports it over the JSON API (crates/cli/src/model.rs and the
// snapshot in app.rs). Field names are the wire names.

import { t } from "../i18n";

export const MAX_GROUP_BOTS = 6;

export interface ProviderStatus {
  kind: ProviderKind | string;
  is_connected: boolean;
  detail: string;
  base_url?: string;
  /** A custom provider's name, the protocol its server speaks, and its models. Built-in providers have none. */
  name?: string;
  api?: CustomAPI;
  models?: CustomModel[];
  /** The model Auto-review runs on this provider unless the user picks another: the catalog's
   * small, fast review model for a built-in, a custom provider's first. */
  review_model?: string;
}

/// The wire protocol a custom provider's server speaks: a chat protocol, or a decision API
/// (`system-one`, `decisions`) whose models only Auto-review runs.
export type CustomAPI = "chat-completions" | "responses" | "messages" | "system-one" | "decisions";

/// A model a custom provider offers, with what its server's model list said about it. Only the
/// id is sure.
export interface CustomModel {
  id: string;
  name?: string;
  context_window?: number;
  max_output?: number;
  images?: boolean;
  /// The thinking levels the core says it takes, lowest first: in a provider's status only.
  levels?: string[];
}

/// Why this phone's last try to connect to the relay failed.
export interface RelayProblem {
  /// The error as it came: the relay's answer, or why none came.
  message: string;
  /// The relay has no record of this phone: it was reset, or it is not the relay that paired
  /// it. Only pairing again brings the phone back.
  unknown_machine: boolean;
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
  /// The `lorca` that Device runs, as `lorca --version` says it.
  version?: string;
  /// Only on a Runner whose CLI replaces itself (installed with lorca.app's script).
  update?: UpdateStatus | null;
  /// The relay lists the machine, but it never sent its `machine` blob: no name, no `os`.
  unknown?: boolean;
}

/// A self-updating CLI's updates, as its Runner lists them in its `machine` blob.
export interface UpdateStatus {
  /// It installs a newer release by itself and restarts into it once no bot is at work there.
  auto: boolean;
  /// The newest release the last check found, when it is newer than `version`.
  latest?: string;
  /// `installing` while it downloads and swaps the binary; `restarting` once the new one is in
  /// place and it waits for its bots to finish; `installed` when `lorca serve` has to be
  /// restarted by hand to run it.
  state?: "installing" | "restarting" | "installed";
  /// The CLI's words for the last check or install that failed.
  error?: string;
}

/// A plugin as its Runner advertises it: installed, and in what state.
/// The Browser plugin, whose screen lists a bot's browser profiles.
export const BROWSER_PLUGIN_ID = "playwright";

/// One of a bot's browser profiles, as its Runner reports it (`browser.sessions`). Each keeps its
/// own sign-ins there. `revision` goes up with every change of control; Return to Bot sends the
/// one this phone last saw.
export interface BrowserProfile {
  id: string;
  name: string;
  state: "stopped" | "bot" | "taking_over" | "human";
  revision: number;
}

export interface PluginStatus {
  id: string;
  name: string;
  description?: string;
  version?: string;
  /// An SF Symbol name.
  icon?: string;
  state: "ready" | "needs_setup" | "needs_auth" | "insufficient_access" | "connecting" | "error";
  detail?: string;
  /// A named account's marketplace service (gmail) and the user's name for it (Work); its `id`
  /// is the account's own.
  service_id?: string;
  account_name?: string;
}

/// An installed plugin as its Runner details it: here, how each server signs in.
export interface PluginDetail {
  status: PluginStatus;
  servers: { name: string; kind: string; auth: { oauth?: boolean; signed_in?: boolean } }[];
}

/// The connected kinds a bot can run with: built-ins first, then custom providers in the order
/// they were added, as the core lists them. A decision provider is Auto-review's alone.
export function connectedProviders(providers: readonly ProviderStatus[]): string[] {
  return providers.filter((p) => p.is_connected && !isDecisionAPI(p.api)).map((p) => p.kind);
}

/// The providers Auto-review can run a model of: every one the account has connected, decision
/// providers among them, in the order the core lists them.
export function reviewProviders(providers: readonly ProviderStatus[]): string[] {
  return providers.filter((p) => p.is_connected).map((p) => p.kind);
}

export function isRunner(device: Device): boolean {
  return device.os === "macos" || device.os === "linux" || device.os === "windows";
}

/// What a Device goes by: a machine that never said what it is has no name of its own.
export function deviceName(device: Device): string {
  return device.unknown ? t("Unknown Device") : device.name;
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
  /** The user's Access for it (`src/core/access.ts`); none is full access. */
  permissions?: BotPermissions;
  created_at: number;
}

/// A bot's Access, as the CLI keeps it in the encrypted roster: without `connections` every plugin
/// on the Runner, else only those it lists; files `write` and shell on unless they say otherwise.
export interface BotPermissions {
  connections?: Record<string, { capabilities: string[]; tools?: string[] }>;
  filesystem?: "none" | "read" | "write";
  shell?: boolean;
}

/// A recurring task a bot runs on a schedule in its direct chat, as the roster carries it, with
/// the schedule in words, the next run, and the running state resolved by the core.
export interface Routine {
  id: string;
  bot_id: string;
  name: string;
  /** The task, written to the bot, handed to it on every run. */
  prompt: string;
  /** `every 30m`, `every 2h`, `every 1d`, or five cron fields in `timezone`. */
  schedule: string;
  /** The IANA timezone a cron schedule reads in. */
  timezone?: string;
  /** After due times its Runner missed: "coalesce" runs once when it is back, "skip" waits for the next one. */
  missed_run_policy?: "coalesce" | "skip";
  /** How it stands, from the core: "on", "running", "paused", "blocked", "failed", or "waiting_for_runner". */
  state?: string;
  health?: RoutineHealth;
  /** "Weekdays at 9:00 AM" */
  schedule_text: string;
  is_enabled: boolean;
  /** Why Lorca paused it, when it did: "away", or "authentication" after three failed sign-ins in a row. */
  paused_reason?: string;
  last_run_at?: number;
  /** "sent", "pass", or "error". */
  last_outcome?: string;
  next_run_at?: number | null;
  is_running: boolean;
  /** The script the Runner runs at each due time before the bot does; `next_run_at` is then the next check. */
  check?: string | null;
  created_at: number;
  /** When a one-time routine runs; its Runner removes it after that run. */
  once_at?: number | null;
  /** The pull request a watch reads at each due time, until it merges or closes. */
  pull_request?: { repo: string; number: number; title?: string; url?: string } | null;
  /** The calendar events a routine around events runs before or after: so many minutes before they start, or after they end. */
  calendar?: { account?: string; matching?: string | null; minutes?: number | null; after?: boolean | null; next_event?: { title?: string; start?: number; end?: number } | null } | null;
}

/// How a routine's checks and runs have gone, as its Runner records them.
export interface RoutineHealth {
  last_check_at?: number | null;
  last_success_at?: number | null;
  /** How the last check went: "quiet", "ready", "failed", or "blocked". */
  status?: string | null;
  connection_failures?: number;
  authentication_failures?: number;
  /** The runs' own streak with the model provider, which checks do not clear. */
  model?: { status?: string | null; authentication_failures?: number };
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
  | { kind: "text"; text: string; attachments?: Attachment[]; reply_to?: ReplyTo | null }
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
      /** A codemode script's latest command, by its description, while no plugin call came
       * after it: the working row reads "Running command: Run the tests…". */
      script_command?: string;
      /** A bash call's card, from Auto-review's question to how the command ended. */
      run?: CommandRun;
    }
  | { kind: "handoff"; from: string; to: string; reason: string }
  | { kind: "notice"; text: string; routine_id?: string }
  /// The bot asks before a plugin or shell action, or before installing a plugin (`tool` is `install`).
  /// `rule` is the rule Always allow adds, which Auto-review proposed for a shell command (`plugin_id` is `computer`);
  /// `command` is that command in full, where `summary` is its first line.
  | { kind: "permission"; plugin_id: string; plugin_name: string; tool: string; summary: string; decision: "pending" | "allowed" | "always" | "denied" | "expired" | "dismissed" | "connected" | "failed"; reason?: string; rule?: string; command?: string; link?: string; code?: string; secret?: SecretAsk };

/// What a secret request asks for (a permission card with `tool` `secret`): the values the bot
/// names, and where its Runner uses them. The card takes the values; they go sealed to the Runner
/// and never come back.
export interface SecretAsk {
  /** `browser` (typed into a sign-in page of `site` in the bot's Browser), `command` (an environment variable of its commands), or `plugin` (a setting of the card's plugin). */
  use: "browser" | "command" | "plugin";
  site?: string;
  fields: { name: string; label: string }[];
}

/// A secret kept on a Runner for one of its bots, as Settings lists it: never its value.
export interface SavedSecret {
  id: string;
  bot_id: string;
  name: string;
  label: string;
  use: SecretAsk["use"];
  site?: string;
  updated_at: number;
}

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
  /** It runs in the background: the bot started it there, or the user sent it. Stop in the chat leaves it running. */
  background?: boolean;
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

/// A `bash` row whose command runs in its terminal while the bot's call still waits on it: Run in
/// Background sends it there, and the call returns.
export function runsInForeground(message: Message | undefined): boolean {
  if (message?.body.kind !== "tool") return false;
  return message.body.is_running && runsInTerminal(message) && !message.body.run?.background;
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

/// The check on effectful plugin actions and shell commands, shared through the roster. It runs
/// the review model of `provider`, any provider the account has connected, or without one of the
/// bot's own provider. `models` holds the review models the user picked, by provider kind; any
/// other provider runs its status's `review_model`.
export type AutoReview = { is_enabled: boolean; rules: AutoReviewRule[]; provider?: string; models?: Record<string, string> };

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
  /** A message of the user's the bot's turn holds for its next step; Send now has it read now. */
  queued?: boolean;
  /** A file or document link a bot published: this message is one version of it. */
  output?: Output;
}

/// A published output: `id` names it across its versions, the message carrying it is the version.
/// A file is the message's attachment; a link is `url`.
export interface Output {
  id: string;
  name: string;
  mime: string;
  bot_id: string;
  version: number;
  url?: string;
  evidence?: OutputEvidence;
}

/// What the bot says it checked: a test run, a screenshot from before or after a change, or
/// another check, and how it went.
export interface OutputEvidence {
  kind: "test_result" | "before_screenshot" | "after_screenshot" | "verification";
  summary: string;
  status: "passed" | "failed" | "unverified";
  command?: string;
}

export function evidenceTitle(evidence: OutputEvidence): string {
  switch (evidence.kind) {
    case "test_result":
      return t("Test result");
    case "before_screenshot":
      return t("Before screenshot");
    case "after_screenshot":
      return t("After screenshot");
    default:
      return t("Check");
  }
}

export function evidenceStatus(evidence: OutputEvidence): string {
  return evidence.status === "passed" ? t("Passed") : evidence.status === "failed" ? t("Failed") : t("Not verified");
}

/// The document a link output points at; only an https address without credentials opens.
export function documentUrl(output: Output): string | undefined {
  if (!output.url) return undefined;
  try {
    const url = new URL(output.url);
    return url.protocol === "https:" && url.hostname && !url.username && !url.password ? url.href : undefined;
  } catch {
    return undefined;
  }
}

/// The SF Symbol for what the output is.
export function outputSymbol(output: Output): string {
  if (output.url) return "link";
  if (output.mime.startsWith("image/")) return "photo";
  if (output.mime.startsWith("video/")) return "film";
  if (output.mime.startsWith("audio/")) return "waveform";
  if (output.mime === "application/pdf") return "doc.richtext";
  if (output.mime.startsWith("text/") || output.mime === "application/json") return "doc.text";
  return "doc";
}

/// An output and its versions, newest first.
export interface OutputSeries {
  id: string;
  versions: Message[];
}

/// A chat's output messages as one series per output, the latest published first.
export function groupOutputs(messages: Message[]): OutputSeries[] {
  const byId = new Map<string, Message[]>();
  for (const message of messages) {
    if (!message.output) continue;
    const versions = byId.get(message.output.id) ?? [];
    if (!versions.some((m) => m.id === message.id)) versions.push(message);
    byId.set(message.output.id, versions);
  }
  return [...byId.entries()]
    .map(([id, versions]) => ({ id, versions: versions.sort((a, b) => b.output!.version - a.output!.version) }))
    .sort((a, b) => b.versions[0].created_at - a.versions[0].created_at);
}

/// A message quoted by the user's reply: who wrote it and how it opens, as the core keeps it with
/// the reply, so the quote reads the same where the original has not loaded.
export interface ReplyTo {
  message_id: string;
  author: Author;
  text: string;
}

/// A finished text message, the user's or a bot's, which a reply can answer.
export function canBeQuoted(message: Message): boolean {
  return message.body.kind === "text" && message.state.kind === "complete" && message.author.kind !== "system";
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
  /// What a group is for, which every member reads in its system prompt.
  description?: string | null;
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

/// The providers Lorca has built in, in the order the core lists them.
export const PROVIDER_KINDS = ["deepseek", "anthropic", "opencode", "opencode-go", "chatgpt", "grok"] as const;
export type ProviderKind = (typeof PROVIDER_KINDS)[number];

/// A built-in provider's kind. Custom providers' kinds start with `custom:`.
export function isProviderKind(kind: string): kind is ProviderKind {
  return (PROVIDER_KINDS as readonly string[]).includes(kind);
}

/// A provider the user added, any server that speaks OpenAI's or Anthropic's API or a decision
/// API, has the kind `custom:` and a slug of the name it was added with. It lives in the
/// account's credentials.
export const CUSTOM_PROVIDER_PREFIX = "custom:";

export function isCustomProvider(kind: string): boolean {
  return kind.startsWith(CUSTOM_PROVIDER_PREFIX);
}

/// The protocols a custom provider's server can speak: the product's name, the same in every
/// language; the path Lorca adds to the base URL for a model call; and the base URL's example.
/// The decision APIs come last; their example is a root, as the path is added to one.
export const CUSTOM_APIS: readonly { id: CustomAPI; title: string; path: string; placeholder: string }[] = [
  { id: "chat-completions", title: "OpenAI Chat Completions", path: "/chat/completions", placeholder: "https://api.example.com/v1" },
  { id: "responses", title: "OpenAI Responses", path: "/responses", placeholder: "https://api.example.com/v1" },
  { id: "messages", title: "Anthropic Messages", path: "/messages", placeholder: "https://api.example.com/v1" },
  { id: "system-one", title: "System One", path: "/systemone", placeholder: "https://api.example.com/v1" },
  { id: "decisions", title: "OpenAI Decisions", path: "/decisions", placeholder: "https://api.example.com/v1" },
];

/// A decision API, whose models answer typed questions instead of chatting: Auto-review can run
/// them, and no bot can.
export function isDecisionAPI(api: string | undefined): boolean {
  return api === "system-one" || api === "decisions";
}

export function customAPI(id: string | undefined) {
  return CUSTOM_APIS.find((api) => api.id === id) ?? CUSTOM_APIS[0];
}

/// Where a custom provider's model calls go, from the base URL as typed, the way the core
/// stores it: trimmed, without a trailing slash, and cut back to the root when a whole endpoint
/// was pasted. A decision API's URL is its endpoint, since vendors serve one at different paths:
/// one that ends in a decision path stays as it is, and any other gets the API's path. Empty
/// while the base URL is.
export function customRequestURL(api: CustomAPI, baseURL: string): string {
  let root = baseURL.trim().replace(/\/+$/, "");
  if (!root) return "";
  const { path } = customAPI(api);
  if (isDecisionAPI(api)) return /\/(systemone|decisions)$/.test(root) ? root : root + path;
  if (root.endsWith(path)) root = root.slice(0, -path.length);
  // Messages adds the /v1 a root without one lacks (Moonshot's …/anthropic).
  if (api === "messages" && !root.endsWith("/v1")) root += "/v1";
  return root + path;
}

/// The host a base URL names, with its port: "openrouter.ai", "192.168.1.20:11434". Empty when
/// it names none. Parsed by hand: React Native's URL leaves most of its getters unimplemented.
export function urlHost(baseURL: string): string {
  return baseURL.trim().match(/^[a-z][a-z0-9+.-]*:\/\/(?:[^@/?#]*@)?([^/?#]+)/i)?.[1].toLowerCase() ?? "";
}

/// An http(s) URL with a host: what the core can ask for a model list.
export function isHTTPURL(baseURL: string): boolean {
  return /^https?:\/\/[^\s/?#]+/i.test(baseURL.trim());
}

/// This machine by name or address. On a phone that is the phone itself, never a Runner.
export function isLoopbackHost(host: string): boolean {
  const name = host.replace(/:\d+$/, "");
  return name === "localhost" || name.endsWith(".localhost") || name.startsWith("127.") || name === "[::1]" || name === "0.0.0.0";
}

/// A server people often add, to start the form from: its product name, protocol, and base URL.
/// `local` ones run on the user's own computer; those with a decision API serve Auto-review.
export interface CustomPreset {
  name: string;
  api: CustomAPI;
  baseURL: string;
  keyPlaceholder: () => string;
  local?: boolean;
}

export const CUSTOM_PRESETS: readonly CustomPreset[] = [
  { name: "OpenAI", api: "responses", baseURL: "https://api.openai.com/v1", keyPlaceholder: () => t("sk-… from platform.openai.com") },
  { name: "OpenRouter", api: "chat-completions", baseURL: "https://openrouter.ai/api/v1", keyPlaceholder: () => t("sk-or-… from openrouter.ai/keys") },
  { name: "Gemini", api: "chat-completions", baseURL: "https://generativelanguage.googleapis.com/v1beta/openai", keyPlaceholder: () => t("Key from aistudio.google.com") },
  { name: "Groq", api: "chat-completions", baseURL: "https://api.groq.com/openai/v1", keyPlaceholder: () => t("gsk_… from console.groq.com") },
  { name: "Together AI", api: "chat-completions", baseURL: "https://api.together.xyz/v1", keyPlaceholder: () => t("Key from api.together.ai") },
  { name: "Ollama", api: "chat-completions", baseURL: "http://localhost:11434/v1", keyPlaceholder: () => t("Optional for a server on your network"), local: true },
  { name: "LM Studio", api: "chat-completions", baseURL: "http://localhost:1234/v1", keyPlaceholder: () => t("Optional for a server on your network"), local: true },
  { name: "OpenRouter Decisions", api: "system-one", baseURL: "https://openrouter.ai/api/alpha/decisions", keyPlaceholder: () => t("sk-or-… from openrouter.ai/keys") },
  { name: "OpenAI Decisions", api: "decisions", baseURL: "https://api.openai.com/v1/decisions", keyPlaceholder: () => t("sk-… from platform.openai.com") },
  { name: "TypeSafe", api: "system-one", baseURL: "https://api.typesafe.ai/v1/systemone", keyPlaceholder: () => t("Key from typesafe.ai") },
];

/// The group a preset sits in on Add Custom Provider's menu: hosted servers, the ones on the
/// user's own computer, then decision APIs.
export function presetGroup(preset: CustomPreset): "cloud" | "local" | "decisions" {
  return preset.local ? "local" : isDecisionAPI(preset.api) ? "decisions" : "cloud";
}

export function customPreset(name: string | undefined): CustomPreset | undefined {
  const wanted = name?.trim().toLowerCase();
  return wanted ? CUSTOM_PRESETS.find((preset) => preset.name.toLowerCase() === wanted) : undefined;
}

/// The preset whose server a base URL names, by host and port and, among a server's presets, by
/// API: openrouter.ai with System One is OpenRouter Decisions, with Chat Completions OpenRouter.
export function presetForURL(baseURL: string, api?: CustomAPI): CustomPreset | undefined {
  const host = urlHost(baseURL);
  const server = host ? CUSTOM_PRESETS.filter((preset) => urlHost(preset.baseURL) === host) : [];
  return server.find((preset) => preset.api === api) ?? server[0];
}

/// The name a custom provider takes when the user gives it none: the preset's whose server the
/// base URL names, else the URL's host. Empty without a host.
export function defaultProviderName(baseURL: string, api?: CustomAPI): string {
  return presetForURL(baseURL, api)?.name ?? urlHost(baseURL);
}

/// The account's custom provider with this name, which the core keeps unique ignoring case.
export function customProviderNamed(name: string, providers: readonly ProviderStatus[]): ProviderStatus | undefined {
  const wanted = name.trim().toLowerCase();
  return providers.find((p) => isCustomProvider(p.kind) && p.name?.trim().toLowerCase() === wanted);
}

/// A context window the short way model lists write it: "128K", "1M", "1.5M". A power-of-two
/// window counts in 1,024s, so 131,072 is 128K as well.
export function contextWindowLabel(tokens: number): string {
  const unit = tokens % 1000 !== 0 && tokens % 1024 === 0 ? 1024 : 1000;
  if (tokens < unit) return String(tokens);
  const thousands = Math.round(tokens / unit);
  if (thousands < unit) return `${thousands}K`;
  return `${Math.round((tokens / unit / unit) * 10) / 10}M`;
}

/// What a model is called: the name its server gives it, else its id.
export function modelLabel(model: CustomModel): string {
  return model.name?.trim() || model.id;
}

/// A model in the custom provider form, picked or not. A `user` row was saved with the provider
/// or added by hand and stays whatever the server lists; a `server` row came from the server's
/// list and goes with the next list unless it is picked.
export interface ModelRow extends CustomModel {
  selected: boolean;
  source: "user" | "server";
}

/// A saved provider's models as rows: all picked, in their saved order, the default first.
export function savedModelRows(models: readonly CustomModel[]): ModelRow[] {
  return models.map((model) => ({ ...model, selected: true, source: "user" }));
}

/// The rows once the server's list arrives: the user's rows first, with what the list says of
/// them; then rows picked from an earlier list that this one lacks; then the list in its order,
/// keeping what was picked. When nothing is picked and the list is short, all of it is.
export function mergeListedModels(rows: readonly ModelRow[], listed: readonly CustomModel[]): ModelRow[] {
  const byId = new Map<string, CustomModel>();
  for (const model of listed) if (!byId.has(model.id)) byId.set(model.id, model);
  const user = rows.filter((row) => row.source === "user").map((row) => (byId.has(row.id) ? { ...row, ...byId.get(row.id)!, selected: row.selected, source: row.source } : row));
  const userIds = new Set(user.map((row) => row.id));
  const picked = new Set(rows.filter((row) => row.source === "server" && row.selected).map((row) => row.id));
  const kept = rows.filter((row) => row.source === "server" && row.selected && !byId.has(row.id) && !userIds.has(row.id));
  const server: ModelRow[] = [...byId.values()].filter((model) => !userIds.has(model.id)).map((model) => ({ ...model, selected: picked.has(model.id), source: "server" }));
  const merged = [...user, ...kept, ...server];
  if (byId.size > 0 && byId.size <= 8 && !merged.some((row) => row.selected)) return merged.map((row) => (byId.has(row.id) ? { ...row, selected: true } : row));
  return merged;
}

/// The id the picker's search offers to add: the text, trimmed, unless a row has that id already.
export function modelIdToAdd(rows: readonly ModelRow[], query: string): string | null {
  const id = query.trim();
  return id && !rows.some((row) => row.id === id) ? id : null;
}

/// An id the user typed, picked, at the top.
export function addModelRow(rows: readonly ModelRow[], id: string): ModelRow[] {
  return [{ id, selected: true, source: "user" }, ...rows.filter((row) => row.id !== id)];
}

export function toggleModelRow(rows: readonly ModelRow[], id: string): ModelRow[] {
  return rows.map((row) => (row.id === id ? { ...row, selected: !row.selected } : row));
}

/// The rows whose id or name holds the search, ignoring case.
export function filterModelRows(rows: readonly ModelRow[], query: string): ModelRow[] {
  const wanted = query.trim().toLowerCase();
  if (!wanted) return [...rows];
  return rows.filter((row) => row.id.toLowerCase().includes(wanted) || !!row.name?.toLowerCase().includes(wanted));
}

/// The default model: the one the user chose while it is picked, else the first picked.
export function defaultModelId(rows: readonly ModelRow[], chosen?: string): string | undefined {
  const picked = rows.filter((row) => row.selected);
  return picked.find((row) => row.id === chosen)?.id ?? picked[0]?.id;
}

/// The picked ids as `providers.connect_custom` takes them: the default first, then the rest in
/// the list's order.
export function selectedModelIds(rows: readonly ModelRow[], chosen?: string): string[] {
  const first = defaultModelId(rows, chosen);
  const rest = rows.filter((row) => row.selected && row.id !== first).map((row) => row.id);
  return first ? [first, ...rest] : rest;
}

/// Where asking the server for its models stands: no URL to ask yet, asking, answered with a list
/// (`listed`) or without one (`unlisted`), or failed.
export type ModelListing = { state: "none" } | { state: "loading" } | { state: "listed" } | { state: "unlisted" } | { state: "error"; message: string };

/// What the form and the picker say about it; nothing once the models are in.
export function modelListingNote(listing: ModelListing): string | undefined {
  switch (listing.state) {
    case "none":
      return t("Enter the base URL to load the server’s models.");
    case "loading":
      return t("Loading models…");
    case "unlisted":
      return t("This server doesn’t list its models. Add model IDs in Models.");
    case "error": {
      // The core's message, as a sentence, before the way on.
      const message = listing.message.trim();
      return `${/[.!?。！？]$/.test(message) ? message : `${message}.`} ${t("You can still add model IDs in Models.")}`;
    }
    case "listed":
      return undefined;
  }
}

/// Every provider a bot can run with: the built-ins, then the account's custom providers in the
/// order they were added, but those with a decision API.
export function providerKinds(providers: readonly ProviderStatus[]): string[] {
  return [...PROVIDER_KINDS, ...providers.filter((p) => isCustomProvider(p.kind) && !isDecisionAPI(p.api)).map((p) => p.kind)];
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

/// The name people know a provider by: a built-in's, or the one the user gave a custom
/// provider, which is its slug once the provider is gone.
export function providerLabel(kind: string, providers: readonly ProviderStatus[]): string {
  if (isCustomProvider(kind)) return providers.find((p) => p.kind === kind)?.name || kind.slice(CUSTOM_PROVIDER_PREFIX.length);
  return PROVIDER_LABELS[kind] ?? kind;
}

/** A model the core's catalog offers, as the snapshot carries it: its provider, id, and name,
 * the thinking levels it takes, lowest first, and whether it is a decision model, which answers
 * typed questions instead of chatting: Auto-review can run it, and no bot can. */
export interface ProviderModel {
  provider: string;
  id: string;
  name: string;
  levels: string[];
  decides?: boolean;
}

/** The models a bot of a provider can run, in the catalog's order; the first is the default the
 * CLI uses. Decision models are Auto-review's alone. */
export function providerModels(models: ProviderModel[], provider: string): ProviderModel[] {
  return models.filter((model) => model.provider === provider && !model.decides);
}

/** Every model of a provider Auto-review can run, in the catalog's order: the ones bots can, and
 * decision models. */
export function reviewModels(models: ProviderModel[], provider: string): ProviderModel[] {
  return models.filter((model) => model.provider === provider);
}

/// What Auto-review's Reviews with offers and shows: every connected provider, and the one
/// picked, none for the bot's own (as when the picked one is no longer connected, which the core
/// then reviews with too).
export function reviewPicker(review: AutoReview, providers: readonly ProviderStatus[]): { providers: string[]; provider?: string } {
  const kinds = reviewProviders(providers);
  const provider = review.provider && kinds.includes(review.provider) ? review.provider : undefined;
  return { providers: kinds, provider };
}

/// One row of Settings' Review Models: a connected provider, the name of the model it reviews
/// with by default (none when its status names none), all its models with decision models among
/// them and the picked one added, by its id, when the list lacks it, and the model the user
/// picked, none for the default.
export interface ReviewModelRow {
  kind: string;
  defaultName?: string;
  models: ProviderModel[];
  picked?: string;
}

/// Auto-review with a provider's review model picked, or its default put back (`undefined`), as
/// an `auto_review.set` patch of `models` leaves it.
export function withReviewModel(review: AutoReview, kind: string, model: string | undefined): AutoReview {
  const { [kind]: _, ...others } = review.models ?? {};
  const models = model ? { ...others, [kind]: model } : others;
  return { ...review, models: Object.keys(models).length ? models : undefined };
}

/// A row for every connected provider, in the order the core lists them. `catalog` is the core's
/// with the custom providers' models (`withCustomModels`).
export function reviewModelRows(review: AutoReview, providers: readonly ProviderStatus[], catalog: ProviderModel[]): ReviewModelRow[] {
  return providers
    .filter((status) => status.is_connected)
    .map((status) => {
      const kind = status.kind;
      const offered = reviewModels(catalog, kind);
      const picked = review.models?.[kind] || undefined;
      const models = picked && !offered.some((model) => model.id === picked) ? [...offered, { provider: kind, id: picked, name: picked, levels: [] }] : offered;
      const fallback = status.review_model;
      const defaultName = fallback ? (offered.find((model) => model.id === fallback)?.name ?? fallback) : undefined;
      return { kind, defaultName, models, picked };
    });
}

/** Every thinking level, lowest first. */
const ALL_THINKING_LEVELS = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/** The thinking levels `model` takes, lowest first: the provider's default model's when it is
 * undefined, and every level the provider's models take for a model the catalog does not have. */
export function thinkingLevels(models: ProviderModel[], provider: string, model: string | undefined): string[] {
  const offered = providerModels(models, provider);
  const known = offered.find((each) => each.id === (model ?? offered[0]?.id));
  return known?.levels ?? ALL_THINKING_LEVELS.filter((level) => offered.some((each) => each.levels.includes(level)));
}

/// The catalog with each custom provider's saved models after it, so the pickers offer them as
/// they do the catalog's: named as the provider's server names them, with the thinking levels the
/// core says each takes. Every model of a decision API decides.
export function withCustomModels(models: ProviderModel[], providers: readonly ProviderStatus[]): ProviderModel[] {
  const custom = providers.flatMap((provider) =>
    isCustomProvider(provider.kind)
      ? (provider.models ?? []).map((model) => ({ provider: provider.kind, id: model.id, name: modelLabel(model), levels: model.levels ?? [], decides: isDecisionAPI(provider.api) }))
      : [],
  );
  return [...models, ...custom];
}

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

/// Durable work a bot keeps across turns, as the core's `tasks` methods carry it: an owning bot
/// (and so its Runner), what done looks like, the next step, the tasks it waits for, and, once a
/// run ends, a result with what supports it. Not the Running tasks of `app/tasks`, which are a
/// chat's terminal commands.
/// A draft or an exact call a bot left for the user to approve, as its Runner keeps it: what it
/// would do (`payload`), where (`target`), why (`rationale`), and how it ended. Only that Runner
/// changes it, and a decision names the `version` the user saw.
export interface ReviewItem {
  id: string;
  runner_id: string;
  bot_id: string;
  origin: { chat_id: string; message_id?: string | null; routine_id?: string | null; task_id?: string | null };
  target: { account: string; resource: string };
  rationale: string;
  payload: ReviewPayload;
  version: number;
  revision: number;
  preconditions: { workdir: string; files: { path: string; hash?: string | null }[] };
  state: ReviewState;
  outcome?: { summary: string; result?: { text?: string } | null; message_id: string } | null;
  created_at: number;
}

export type ReviewPayload =
  | { kind: "draft"; text: string }
  | { kind: "shell"; arguments: { command?: string } }
  | { kind: "plugin"; plugin_id: string; server_name: string; tool: string; arguments: unknown };

export type ReviewState = "pending" | "approved" | "executing" | "succeeded" | "failed" | "rejected" | "cancelled" | "uncertain";

/// Waiting for the user, or approved and about to run.
export function reviewIsOpen(item: ReviewItem): boolean {
  return item.state === "pending" || item.state === "approved" || item.state === "executing";
}

export interface DurableTask {
  id: string;
  revision: number;
  authority_runner_id: string;
  owner_bot_id: string;
  runner_id: string;
  goal: string;
  acceptance_criteria: string[];
  dependencies: string[];
  next_action: string;
  chat_ids: string[];
  links: { label: string; url: string }[];
  state: DurableTaskState;
  reason?: string | null;
  result?: string | null;
  evidence: TaskEvidence[];
  active_run?: { id: string; bot_id: string; runner_id: string; chat_id: string; started_at: number } | null;
  created_at: number;
  updated_at: number;
}

export type DurableTaskState = "queued" | "working" | "blocked" | "awaiting_review" | "completed" | "cancelled";

/// What supports a task's result: a message, an output version, a file, a review, or a link.
export interface TaskEvidence {
  kind: "message" | "output" | "file" | "review" | "url";
  label: string;
  chat_id?: string | null;
  message_id?: string | null;
  attachment_id?: string | null;
  url?: string | null;
  output_id?: string | null;
  version?: number | null;
  review_id?: string | null;
}

/// Why a task the user cancelled in the app stopped, for its bot to read; the apps show the state
/// alone for it.
export const CANCELLED_BY_USER = "Cancelled by the user.";

export const taskIsFinished = (state: DurableTaskState) => state === "completed" || state === "cancelled";

/// A run can start: queued or blocked, and not running.
export const taskCanStart = (task: DurableTask) => (task.state === "queued" || task.state === "blocked") && !task.active_run;

/// The SF Symbol for a state, the same as the desktop apps'.
export function taskSymbol(state: DurableTaskState): string {
  switch (state) {
    case "working":
      return "arrow.triangle.2.circlepath";
    case "blocked":
      return "exclamationmark.circle.fill";
    case "awaiting_review":
      return "eye";
    case "completed":
      return "checkmark.circle";
    case "cancelled":
      return "xmark.circle";
    default:
      return "circle";
  }
}

/// What waits on the user first, then open work, then finished tasks.
export function taskOrder(state: DurableTaskState): number {
  return { blocked: 0, awaiting_review: 1, working: 2, queued: 3, completed: 4, cancelled: 4 }[state];
}

/// What a DM's turns, a task's runs, or a routine's runs may use on the bot's Runner. A missing
/// limit is none; zero allows nothing.
export interface BudgetLimits {
  max_usd?: number | null;
  max_tokens?: number | null;
  max_runtime_secs?: number | null;
  max_retries?: number | null;
  max_connector_calls?: number | null;
}

/// One allowance as its Runner keeps it. `chat` holds the limits each new turn in a DM starts
/// with; `job` is one turn; `task` and `routine` all of their runs.
export interface BudgetState {
  kind: "chat" | "job" | "task" | "routine";
  id: string;
  runner_id: string;
  chat_id: string;
  limits: BudgetLimits;
  usage: {
    tokens: number;
    api_cost_usd: number;
    subscription_estimate_usd: number;
    unknown_price_calls: number;
    runtime_secs: number;
    retries: number;
    connector_calls: number;
  };
  state: "ready" | "running" | "complete" | "budget_exhausted" | "interrupted";
  /// The limit it stopped at: usd, tokens, runtime, retries, connector_calls, or unknown_price.
  reached?: string | null;
  updated_at: number;
}

/// A plugin account's call limit on its Runner, shared by every bot there.
export interface CallLimits {
  limits: { max_calls: number; window_secs: number; max_concurrency: number };
  /// When the service asked the calls to wait until, in seconds.
  retry_at?: number | null;
  plugin_id: string;
  service_id: string;
}

/// A group's shared project context, after the desktop apps' Project section: the briefs, goals,
/// constraints, decisions, facts, links (`document`), and files (`asset`) every bot in the group
/// can read. The core owns scope, revisions, source checks, and sync.
export type ProjectKind = "brief" | "goal" | "constraint" | "decision" | "fact" | "document" | "asset";

/// The order the Project section lists the kinds in, and the + menu offers them.
export const PROJECT_KINDS: ProjectKind[] = ["brief", "goal", "constraint", "decision", "fact", "document", "asset"];

/// Where an entry came from, kept whole so a correction carries a cited output along.
export interface ProjectSource {
  kind: "user" | "bot" | "url" | "message" | "output";
  label: string;
  url?: string | null;
  message_id?: string | null;
  output?: { chat_id: string; message_id: string; output_id: string; version: number } | null;
}

export interface ProjectEntry {
  id: string;
  kind: ProjectKind;
  title: string;
  text: string;
  source: ProjectSource;
  verification: string;
  freshness: string;
  updated_at: number;
  verified_at?: number | null;
  fetched_at?: number | null;
  asset?: Attachment | null;
  refresh_error?: string | null;
  supersedes?: string[];
  current?: boolean;
  removed?: boolean;
}

/// A group's current entries in list order, and the entries that are two versions of one (two
/// Devices changed it at once).
export interface ProjectContext {
  entries: ProjectEntry[];
  conflicts: string[][];
}

/// Briefs first and files last, newest first within a kind.
export function orderProjectEntries(entries: ProjectEntry[]): ProjectEntry[] {
  return [...entries].sort((a, b) => PROJECT_KINDS.indexOf(a.kind) - PROJECT_KINDS.indexOf(b.kind) || b.updated_at - a.updated_at || a.id.localeCompare(b.id));
}

/// The other current versions of an entry two Devices changed at once.
export function otherVersions(context: ProjectContext | undefined, id: string): string[] {
  return context?.conflicts.find((versions) => versions.includes(id))?.filter((other) => other !== id) ?? [];
}

/// A bot's proposal, waiting for the user to accept it.
export function isSuggestion(entry: ProjectEntry): boolean {
  return entry.verification === "unverified" && entry.source.kind === "bot";
}

/// A bot reads the link again before it relies on it; an agreed decision is the user's to change.
export function canCheckLink(entry: ProjectEntry): boolean {
  return !!entry.source.url && !(entry.kind === "decision" && entry.verification === "agreed");
}

/// The last time the link was read or confirmed, in seconds.
export function linkChecked(entry: ProjectEntry): number | undefined {
  const times = [entry.fetched_at, entry.verified_at].filter((at): at is number => typeof at === "number");
  return times.length ? Math.max(...times) : undefined;
}

/// The host of the entry's link, for its row.
export function projectHost(entry: ProjectEntry): string | undefined {
  if (!entry.source.url) return undefined;
  try {
    return new URL(entry.source.url).hostname.replace(/^www\./, "");
  } catch {
    return undefined;
  }
}

export function projectSymbol(kind: ProjectKind): string {
  return { brief: "doc.text", goal: "flag", constraint: "hand.raised", decision: "checkmark.seal", fact: "info.circle", document: "link", asset: "paperclip" }[kind];
}

/// What the core answers when the entry being saved changed on another Device first.
export const STALE_PROJECT_ENTRY = "This entry changed on another Device.";

/// Whose skill it is: one bot's, in every chat it is in (`bot`), or a group's, for the bots in
/// that group (`project`).
export interface PlaybookScope {
  kind: "bot" | "project";
  id: string;
}

/// A reference or a script the skill carries, as text: `references/checklist.md`.
export interface PlaybookFile {
  path: string;
  text: string;
}

export interface PlaybookContent {
  name: string;
  description: string;
  instructions: string;
  examples: string;
  references: PlaybookFile[];
  scripts: PlaybookFile[];
}

/// A skill as the roster lists it, without its body.
export interface PlaybookSummary {
  id: string;
  scope: PlaybookScope;
  name: string;
  description: string;
  status: "draft" | "saved" | "deleted";
  revision: number;
  hash: string;
  updated_at: number;
}

/// One step of a skill's history: who changed it where, and what it said then.
export interface PlaybookRevision {
  id: string;
  revision: number;
  status: "draft" | "saved" | "deleted";
  content: PlaybookContent | null;
  provenance: { kind: string; chat_id?: string | null; message_ids: string[] };
  device_id: string;
  created_at: number;
}

/// A skill with its body and history, fetched when it opens.
export interface PlaybookRecord {
  id: string;
  scope: PlaybookScope;
  status: "draft" | "saved" | "deleted";
  revision: number;
  hash: string;
  content: PlaybookContent | null;
  revisions: PlaybookRevision[];
}

/// Whose skills a chat shows: the bot's in a DM, the group's in a group.
export function skillScopeOf(chat: ChatMeta): PlaybookScope | undefined {
  if (chat.kind === "group") return { kind: "project", id: chat.id };
  return chat.bot_ids[0] ? { kind: "bot", id: chat.bot_ids[0] } : undefined;
}

/// What the core answers when a skill being saved changed on another Device first.
export function isStaleSkill(error: string): boolean {
  return error.includes("changed since");
}
