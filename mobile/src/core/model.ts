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
}

/// The wire protocol a custom provider's server speaks.
export type CustomAPI = "chat-completions" | "responses" | "messages";

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
  /// The relay lists the machine, but it never sent its `machine` blob: no name, no `os`.
  unknown?: boolean;
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

/// The kinds the account has connected: built-ins first, then custom providers in the order
/// they were added, as the core lists them.
export function connectedProviders(providers: readonly ProviderStatus[]): string[] {
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
  /** The script the Runner runs at each due time before the bot does; `next_run_at` is then the next check. */
  check?: string | null;
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

/// The providers Lorca has built in, in the order the core lists them.
export const PROVIDER_KINDS = ["deepseek", "anthropic", "opencode", "opencode-go", "chatgpt", "grok"] as const;
export type ProviderKind = (typeof PROVIDER_KINDS)[number];

/// A built-in provider's kind. Custom providers' kinds start with `custom:`.
export function isProviderKind(kind: string): kind is ProviderKind {
  return (PROVIDER_KINDS as readonly string[]).includes(kind);
}

/// A provider the user added, any server that speaks OpenAI's or Anthropic's API, has the kind
/// `custom:` and a slug of the name it was added with. It lives in the account's credentials.
export const CUSTOM_PROVIDER_PREFIX = "custom:";

export function isCustomProvider(kind: string): boolean {
  return kind.startsWith(CUSTOM_PROVIDER_PREFIX);
}

/// The protocols a custom provider's server can speak: the product's name, the same in every
/// language; the path Lorca adds to the base URL for a model call; and the base URL's example.
export const CUSTOM_APIS: readonly { id: CustomAPI; title: string; path: string; placeholder: string }[] = [
  { id: "chat-completions", title: "OpenAI Chat Completions", path: "/chat/completions", placeholder: "https://api.example.com/v1" },
  { id: "responses", title: "OpenAI Responses", path: "/responses", placeholder: "https://api.example.com/v1" },
  { id: "messages", title: "Anthropic Messages", path: "/v1/messages", placeholder: "https://api.example.com" },
];

export function customAPI(id: string | undefined) {
  return CUSTOM_APIS.find((api) => api.id === id) ?? CUSTOM_APIS[0];
}

/// Where a custom provider's model calls go, from the base URL as typed, the way the core
/// stores it: trimmed, without a trailing slash, and cut back to the root when a whole endpoint
/// was pasted. Empty while the base URL is.
export function customRequestURL(api: CustomAPI, baseURL: string): string {
  let root = baseURL.trim().replace(/\/+$/, "");
  if (!root) return "";
  const { path } = customAPI(api);
  const pasted = api === "messages" ? [path, "/v1"] : [path];
  const endpoint = pasted.find((suffix) => root.endsWith(suffix));
  if (endpoint) root = root.slice(0, -endpoint.length);
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
/// `local` ones run on the user's own computer.
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
];

export function customPreset(name: string | undefined): CustomPreset | undefined {
  const wanted = name?.trim().toLowerCase();
  return wanted ? CUSTOM_PRESETS.find((preset) => preset.name.toLowerCase() === wanted) : undefined;
}

/// The preset whose server a base URL names, by host and port.
export function presetForURL(baseURL: string): CustomPreset | undefined {
  const host = urlHost(baseURL);
  return host ? CUSTOM_PRESETS.find((preset) => urlHost(preset.baseURL) === host) : undefined;
}

/// The name a custom provider takes when the user gives it none: the preset's whose server the
/// base URL names, else the URL's host. Empty without a host.
export function defaultProviderName(baseURL: string): string {
  return presetForURL(baseURL)?.name ?? urlHost(baseURL);
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
/// order they were added.
export function providerKinds(providers: readonly ProviderStatus[]): string[] {
  return [...PROVIDER_KINDS, ...providers.filter((p) => isCustomProvider(p.kind)).map((p) => p.kind)];
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
 * and the thinking levels it takes, lowest first. */
export interface ProviderModel {
  provider: string;
  id: string;
  name: string;
  levels: string[];
}

/** The models a provider offers, in the catalog's order; the first is the default the CLI uses. */
export function providerModels(models: ProviderModel[], provider: string): ProviderModel[] {
  return models.filter((model) => model.provider === provider);
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
/// core says each takes.
export function withCustomModels(models: ProviderModel[], providers: readonly ProviderStatus[]): ProviderModel[] {
  const custom = providers.flatMap((provider) =>
    isCustomProvider(provider.kind)
      ? (provider.models ?? []).map((model) => ({ provider: provider.kind, id: model.id, name: modelLabel(model), levels: model.levels ?? [] }))
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
