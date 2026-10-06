// Wire shapes the CLI sends over 127.0.0.1, and their mapping into the app's model, after the
// macOS app's Protocol.swift. Keys are snake_case on the wire.

import { L } from "../l10n";
import * as Format from "./format";
import {
  isAccent,
  isCustomAPI,
  isCustomKind,
  isProviderKind,
  type Attachment,
  type Author,
  type AutoReview,
  type Bot,
  type BotMemory,
  type BotTemplate,
  type Chat,
  type ChatUsage,
  type CommandState,
  type CustomModel,
  type Device,
  type DeviceOS,
  type InstalledPlugin,
  type Marketplace,
  type MarketplacePlugin,
  type Message,
  type PermissionDecision,
  type PluginDetail,
  type PluginState,
  type ProviderCredential,
  type ProviderModel,
  type Routine,
} from "./models";
import type { McpEntry, McpFile, McpServer, ParsedServer } from "./mcp";

/** Why the last try to connect to the relay failed. */
export interface WireRelayProblem {
  /** The error as it came: the relay's answer, or why none came. */
  message: string;
  unknown_machine?: boolean;
}

export interface WireHello {
  version: string;
  has_identity: boolean;
  is_identity_device: boolean;
  device_id?: string;
  relay_url?: string;
  relay_connected: boolean;
  relay_update_required?: boolean;
  relay_error?: WireRelayProblem | null;
}

export interface WirePluginStatus {
  id: string;
  name: string;
  description?: string;
  version?: string;
  icon?: string;
  state: string;
  detail?: string;
  source?: string | null;
}

/** A server of a Runner's mcp.json, as `mcp.list` and `mcp.get` answer. */
export interface WireMcpServer {
  name: string;
  id: string;
  enabled: boolean;
  config: Record<string, unknown> | null;
  problem?: string | null;
  status?: WirePluginStatus | null;
  signs_in?: boolean | null;
  signed_in?: boolean | null;
  tool_count?: number | null;
  tools?: { name: string; title?: string | null; description?: string | null; read_only?: boolean | null; hidden?: boolean | null }[] | null;
}

export interface WireMcpFile {
  path: string;
  error?: string | null;
  servers: WireMcpServer[];
}

/** `mcp.parse`: the servers pasted JSON holds. */
export interface WireParsedServers {
  servers: { name?: string | null; config: Record<string, unknown> | null; problem?: string | null }[];
}

export interface WireDevice {
  id: string;
  name: string;
  model: string;
  os: string;
  os_version: string;
  machine_key: string;
  is_this_device: boolean;
  status: string;
  last_seen: number;
  plugins?: WirePluginStatus[];
  /** The relay lists the machine, but it never sent its `machine` blob: no name, no `os`. */
  unknown?: boolean;
}

export interface WireAttachment {
  id: string;
  name: string;
  mime: string;
  size: number;
  width?: number | null;
  height?: number | null;
}

export interface WireBot {
  id: string;
  name: string;
  description: string;
  symbol_name: string;
  accent: string;
  runner_id: string;
  provider: string;
  model?: string | null;
  thinking?: string | null;
  avatar?: WireAttachment | null;
  created_at: number;
}

export interface WireRun {
  session_id?: string | null;
  command?: string | null;
  state: string;
  prompt?: string | null;
  output?: string | null;
  device?: string | null;
  reason?: string | null;
  rule?: string | null;
  handed_over?: boolean | null;
  background?: boolean | null;
}

export interface WireBody {
  kind: string;
  text?: string | null;
  attachments?: WireAttachment[] | null;
  name?: string | null;
  summary?: string | null;
  detail?: string | null;
  is_running?: boolean | null;
  description?: string | null;
  target_bot_id?: string | null;
  script_command?: string | null;
  from?: string | null;
  to?: string | null;
  reason?: string | null;
  plugin_id?: string | null;
  plugin_name?: string | null;
  tool?: string | null;
  decision?: string | null;
  link?: string | null;
  code?: string | null;
  rule?: string | null;
  command?: string | null;
  run?: WireRun | null;
  reply_to?: { message_id: string; author: WireAuthor; text: string } | null;
}

export interface WireAuthor {
  kind: string;
  bot_id?: string | null;
}

export interface WireMessage {
  id: string;
  chat_id: string;
  author: WireAuthor;
  body: WireBody;
  state: { kind: string; error?: string | null };
  created_at: number;
  queued?: boolean | null;
}

export interface WireChatUsage {
  context_tokens: number;
  context_window: number;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cost_usd: number;
  turns: number;
  model: string;
}

export interface WireChat {
  id: string;
  kind: string;
  title?: string | null;
  bot_ids: string[];
  owner_bot_id?: string | null;
  description?: string | null;
  is_pinned: boolean;
  created_at: number;
  messages?: WireMessage[] | null;
  unread_count?: number | null;
  usage?: WireChatUsage | null;
  has_more?: boolean | null;
}

export interface WireRoutine {
  id: string;
  bot_id: string;
  name: string;
  prompt: string;
  schedule: string;
  schedule_text?: string | null;
  is_enabled: boolean;
  paused_reason?: string | null;
  last_run_at?: number | null;
  last_outcome?: string | null;
  next_run_at?: number | null;
  is_running?: boolean | null;
  check?: string | null;
  created_at: number;
}

export interface WireAutoReview {
  is_enabled: boolean;
  rules?: { id: string; text: string; behavior: string; tool?: string | null }[] | null;
}

export interface WireProvider {
  kind: string;
  is_connected: boolean;
  detail: string;
  base_url?: string | null;
  /** A custom provider's name, wire protocol, and models. Built-in providers have none. */
  name?: string | null;
  api?: string | null;
  models?: WireCustomModel[] | null;
}

/** A custom provider's model, with what its server's model list said of it, and in a status the
 * thinking levels it takes. */
export interface WireCustomModel {
  id: string;
  name?: string | null;
  context_window?: number | null;
  max_output?: number | null;
  images?: boolean | null;
  levels?: string[] | null;
}

/** `providers.list_models`: the chat models a server lists, in its order. `listed` is false when
 * the server publishes no list. */
export interface WireModelList {
  listed: boolean;
  models: WireCustomModel[];
}

export interface WireRunningTurn {
  job_id: string;
  chat_id: string;
  bot_id: string;
  routine_id?: string | null;
}

export interface WireSnapshot {
  version: string;
  has_identity: boolean;
  is_identity_device: boolean;
  identity_id?: string | null;
  this_device_id?: string | null;
  relay_url?: string | null;
  relay_connected: boolean;
  relay_update_required?: boolean | null;
  relay_error?: WireRelayProblem | null;
  devices: WireDevice[];
  bots: WireBot[];
  chats: WireChat[];
  routines?: WireRoutine[] | null;
  auto_review?: WireAutoReview | null;
  providers?: WireProvider[] | null;
  models?: WireModel[] | null;
  running_chat_ids: string[];
  running_turns?: WireRunningTurn[] | null;
}

/** A model the CLI's catalog offers, in the catalog's order: each provider's first is its default. */
export interface WireModel {
  provider: string;
  id: string;
  name: string;
  /** The thinking levels it takes, lowest first. */
  levels: string[];
}

export interface WireRosterChanged {
  devices: WireDevice[];
  bots: WireBot[];
  chats: WireChat[];
  routines?: WireRoutine[] | null;
  auto_review?: WireAutoReview | null;
  providers?: WireProvider[] | null;
  /** The catalog's models again, so a newer catalog the CLI installs reaches the pickers. */
  models?: WireModel[] | null;
}

export interface WireMessagePage {
  messages: WireMessage[];
  has_more: boolean;
}

export interface WireSearchResults {
  chats: { chat_id: string; snippet: string }[];
  messages: { chat_id: string; message_id: string; snippet: string; author: WireAuthor; created_at: number }[];
}

export interface WireMarketplacePlugin {
  id: string;
  name: string;
  description?: string | null;
  icon?: string | null;
  homepage?: string | null;
  author?: string | null;
  category?: string | null;
  featured?: boolean | null;
  tags?: string[] | null;
  servers?: Record<string, { type: string; url?: string | null; command?: string | null; args?: string[] | null; auth?: { type: string } | null }> | null;
  variables?: { name: string; description?: string | null; secret?: boolean | null; required?: boolean | null }[] | null;
  skills?: { name: string; description?: string | null }[] | null;
  installed_on?: string[] | null;
}

export interface WireBotTemplate {
  id: string;
  name: string;
  summary?: string | null;
  description: string;
  symbol_name?: string | null;
  accent?: string | null;
  category?: string | null;
  featured?: boolean | null;
  author?: string | null;
  plugins?: string[] | null;
  routines?: { name: string; schedule: string; schedule_text?: string | null; prompt: string }[] | null;
  memory?: string[] | null;
}

export interface WirePluginDetail {
  manifest: { homepage?: string | null };
  status: WirePluginStatus;
  variables: { name: string; description?: string | null; secret: boolean; required: boolean; is_set: boolean; value?: string | null }[];
  servers: { name: string; kind: string; auth: { url?: string | null; oauth?: boolean | null; signed_in?: boolean | null; code?: string | null; link?: string | null } }[];
  skills: { name: string; description?: string | null }[];
}

export interface WireBotMemory {
  bot_id: string;
  here: boolean;
  runner: string;
  path?: string | null;
  index?: { text: string; hash: string; lines: number; bytes: number; truncated: boolean; max_lines: number; max_bytes: number } | null;
  topics?: string[] | null;
  logs?: string[] | null;
}

export interface WirePairStart {
  nonce: string;
  pairing_string: string;
}

export interface WirePairStatus {
  state: string;
  error?: string | null;
  /** `completed`: the Device that joined. */
  device?: { id: string; name: string } | null;
}

/** `sync.account`: the account's providers once this Device's first pull has its credentials. */
export interface WireSyncAccount {
  providers?: WireProvider[] | null;
}

export interface WireJobEvent {
  chat_id: string;
  bot_id: string;
  job_id: string;
  routine_id?: string | null;
}

export interface WireJobRetry {
  chat_id: string;
  bot_id: string;
  attempt: number;
  max_attempts: number;
  delay_ms: number;
  error: string;
}

export interface WireRelayStatus {
  connected: boolean;
  url?: string | null;
  update_required?: boolean | null;
  error?: WireRelayProblem | null;
}

// MARK: - Mapping

const seconds = (value: number) => value * 1000;
const optional = <T>(value: T | null | undefined): T | undefined => (value === null ? undefined : value);

export function toPlugin(wire: WirePluginStatus): InstalledPlugin {
  const states: PluginState[] = ["ready", "needs_setup", "needs_auth", "connecting", "error"];
  return {
    id: wire.id,
    name: wire.name,
    description: wire.description ?? "",
    version: wire.version ?? "",
    icon: wire.icon ?? "",
    state: (states as string[]).includes(wire.state) ? (wire.state as PluginState) : "unknown",
    detail: wire.detail ?? "",
    source: optional(wire.source),
  };
}

/** An entry as the apps hold it: the strings where strings belong, whatever else the CLI sent. */
export function toMcpEntry(wire: Record<string, unknown> | null | undefined): McpEntry {
  const entry: McpEntry = { ...(wire ?? {}) };
  const text = (value: unknown) => (typeof value === "string" ? value : value === undefined || value === null ? undefined : String(value));
  const map = (value: unknown) =>
    typeof value === "object" && value !== null && !Array.isArray(value)
      ? Object.fromEntries(Object.entries(value as Record<string, unknown>).map(([key, each]) => [key, text(each) ?? ""]))
      : undefined;
  for (const key of ["type", "command", "cwd", "url", "description"] as const) entry[key] = text(entry[key]);
  entry.args = Array.isArray(entry.args) ? entry.args.map((arg) => text(arg) ?? "") : undefined;
  entry.env = map(entry.env);
  entry.headers = map(entry.headers);
  entry.disabled = entry.disabled === true ? true : undefined;
  for (const key of Object.keys(entry)) if (entry[key] === undefined) delete entry[key];
  return entry;
}

export function toMcpServer(wire: WireMcpServer): McpServer {
  return {
    name: wire.name,
    id: wire.id,
    enabled: wire.enabled,
    entry: toMcpEntry(wire.config),
    problem: optional(wire.problem),
    status: wire.status ? toPlugin(wire.status) : undefined,
    signsIn: wire.signs_in ?? false,
    signedIn: wire.signed_in ?? false,
    toolCount: optional(wire.tool_count),
    tools: wire.tools?.map((tool) => ({ name: tool.name, title: optional(tool.title), description: tool.description ?? "", readOnly: tool.read_only ?? false, hidden: tool.hidden ?? false })),
  };
}

export function toMcpFile(wire: WireMcpFile): McpFile {
  return { path: wire.path, error: optional(wire.error), servers: wire.servers.map(toMcpServer) };
}

export function toParsedServers(wire: WireParsedServers): ParsedServer[] {
  return wire.servers.map((server) => ({ name: optional(server.name), entry: server.config ? toMcpEntry(server.config) : undefined, problem: optional(server.problem) }));
}

export function toDevice(wire: WireDevice): Device {
  const oses: DeviceOS[] = ["macos", "linux", "windows", "ios", "ipados", "android"];
  return {
    id: wire.id,
    name: wire.unknown ? L("Unknown Device") : wire.name,
    model: wire.model,
    os: wire.unknown ? "unknown" : (oses as string[]).includes(wire.os) ? (wire.os as DeviceOS) : "linux",
    osVersion: wire.os_version,
    isThisDevice: wire.is_this_device,
    status: wire.status === "online" ? "online" : wire.status === "pairing" ? "pairing" : "offline",
    lastSeen: seconds(wire.last_seen),
    machineKey: wire.machine_key,
    plugins: (wire.plugins ?? []).map(toPlugin),
  };
}

export function toAttachment(wire: WireAttachment): Attachment {
  return {
    id: wire.id,
    name: wire.name,
    mime: wire.mime,
    size: wire.size,
    width: optional(wire.width),
    height: optional(wire.height),
  };
}

export function toBot(wire: WireBot): Bot {
  return {
    id: wire.id,
    name: wire.name,
    description: wire.description,
    symbolName: wire.symbol_name,
    accent: isAccent(wire.accent) ? wire.accent : "indigo",
    runnerID: wire.runner_id,
    provider: isProviderKind(wire.provider) ? wire.provider : "deepseek",
    model: optional(wire.model) || undefined,
    thinking: optional(wire.thinking) || undefined,
    avatar: wire.avatar ? toAttachment(wire.avatar) : undefined,
    createdAt: seconds(wire.created_at),
  };
}

function toAuthor(wire: WireAuthor): Author {
  return wire.kind === "you" ? { kind: "you" } : wire.kind === "bot" ? { kind: "bot", botID: wire.bot_id ?? "" } : { kind: "system" };
}

export function toMessage(wire: WireMessage): Message {
  const author = toAuthor(wire.author);
  const body = wire.body;
  let modelBody: Message["body"];
  switch (body.kind) {
    case "tool": {
      const commandStates: CommandState[] = ["checking", "asking", "running", "waiting", "exited", "failed", "stopped", "denied", "expired", "dismissed"];
      const run = body.run
        ? {
            sessionID: optional(body.run.session_id),
            command: body.run.command ?? "",
            state: (commandStates as string[]).includes(body.run.state) ? (body.run.state as CommandState) : ("stopped" as CommandState),
            prompt: optional(body.run.prompt),
            output: optional(body.run.output),
            device: optional(body.run.device),
            reason: optional(body.run.reason),
            rule: optional(body.run.rule),
            handedOver: body.run.handed_over ?? false,
            background: body.run.background ?? false,
          }
        : undefined;
      modelBody = {
        kind: "tool",
        tool: {
          name: body.name ?? "tool",
          summary: body.summary ?? "",
          detail: body.detail ?? "",
          isRunning: body.is_running ?? false,
          description: optional(body.description),
          targetBotID: optional(body.target_bot_id),
          scriptCommand: optional(body.script_command),
          run,
        },
      };
      break;
    }
    case "handoff":
      modelBody = { kind: "handoff", from: body.from ?? "", to: body.to ?? "", reason: body.reason ?? "" };
      break;
    case "notice":
      modelBody = { kind: "notice", text: body.text ?? "" };
      break;
    case "permission": {
      const decisions: PermissionDecision[] = ["pending", "allowed", "always", "denied", "expired", "dismissed", "connected", "failed"];
      modelBody = {
        kind: "permission",
        request: {
          pluginID: body.plugin_id ?? "",
          pluginName: body.plugin_name ?? "",
          tool: body.tool ?? "",
          summary: body.summary ?? "",
          decision: (decisions as string[]).includes(body.decision ?? "") ? (body.decision as PermissionDecision) : "pending",
          link: optional(body.link),
          code: optional(body.code),
          reason: optional(body.reason),
          rule: optional(body.rule),
          command: optional(body.command),
        },
      };
      break;
    }
    default:
      modelBody = { kind: "text", text: body.text ?? "" };
  }
  const state: Message["state"] =
    wire.state.kind === "thinking"
      ? { kind: "thinking" }
      : wire.state.kind === "streaming"
        ? { kind: "streaming" }
        : wire.state.kind === "failed"
          ? { kind: "failed", error: wire.state.error ?? L("Failed") }
          : { kind: "complete" };
  return {
    id: wire.id,
    author,
    body: modelBody,
    state,
    createdAt: seconds(wire.created_at),
    attachments: (body.attachments ?? []).map(toAttachment),
    replyTo: body.reply_to ? { messageID: body.reply_to.message_id, author: toAuthor(body.reply_to.author), text: body.reply_to.text } : undefined,
    queued: wire.queued ?? undefined,
  };
}

export function toUsage(wire: WireChatUsage): ChatUsage {
  return {
    contextTokens: wire.context_tokens,
    contextWindow: wire.context_window,
    inputTokens: wire.input_tokens,
    outputTokens: wire.output_tokens,
    cacheReadTokens: wire.cache_read_tokens,
    costUSD: wire.cost_usd,
    turns: wire.turns,
    model: wire.model,
  };
}

/** A chat from a snapshot (with its newest messages) or a roster event (without: the chat keeps
 * the messages this app has). */
export function toChat(wire: WireChat, existing?: { messages: Message[]; unreadCount: number; hasMore: boolean }): Chat {
  const kind = wire.kind === "dm" ? "dm" : "group";
  return {
    id: wire.id,
    kind,
    customTitle: kind === "group" ? optional(wire.title) : undefined,
    botIDs: wire.bot_ids,
    messages: wire.messages ? wire.messages.map(toMessage) : (existing?.messages ?? []),
    unreadCount: wire.unread_count ?? existing?.unreadCount ?? 0,
    isPinned: wire.is_pinned,
    createdAt: seconds(wire.created_at),
    usage: wire.usage ? toUsage(wire.usage) : undefined,
    hasMore: wire.has_more ?? existing?.hasMore ?? false,
    ownerBotID: optional(wire.owner_bot_id),
    groupDescription: kind === "group" ? (wire.description ?? "") : "",
  };
}

export function toRoutine(wire: WireRoutine): Routine {
  return {
    id: wire.id,
    botID: wire.bot_id,
    name: wire.name,
    prompt: wire.prompt,
    schedule: wire.schedule,
    scheduleText: Format.schedule(wire.schedule_text ?? wire.schedule),
    isEnabled: wire.is_enabled,
    pausedReason: optional(wire.paused_reason),
    lastRunAt: wire.last_run_at == null ? undefined : seconds(wire.last_run_at),
    lastOutcome: optional(wire.last_outcome),
    nextRunAt: wire.next_run_at == null ? undefined : seconds(wire.next_run_at),
    isRunning: wire.is_running ?? false,
    check: optional(wire.check),
    createdAt: seconds(wire.created_at),
  };
}

export function toAutoReview(wire: WireAutoReview | null | undefined): AutoReview {
  if (!wire) return { isEnabled: true, rules: [] };
  return {
    isEnabled: wire.is_enabled,
    rules: (wire.rules ?? []).map((rule) => ({
      id: rule.id,
      text: rule.text,
      behavior: rule.behavior === "ask" ? "ask" : "allow",
      tool: optional(rule.tool),
    })),
  };
}

/** The built-in providers, then the custom ones in the order they were added. A kind this build
 * does not know is left out. */
export function toProviders(wire: WireProvider[] | null | undefined): ProviderCredential[] {
  return (wire ?? []).flatMap((provider): ProviderCredential[] => {
    if (!isProviderKind(provider.kind)) return [];
    const credential: ProviderCredential = {
      kind: provider.kind,
      isConnected: provider.is_connected,
      detail: provider.detail,
      baseURL: optional(provider.base_url),
    };
    if (isCustomKind(provider.kind)) {
      credential.name = optional(provider.name);
      credential.api = provider.api && isCustomAPI(provider.api) ? provider.api : undefined;
      credential.models = (provider.models ?? []).map(toCustomModel);
    }
    return [credential];
  });
}

export function toCustomModel(wire: WireCustomModel): CustomModel {
  return {
    id: wire.id,
    name: optional(wire.name),
    contextWindow: optional(wire.context_window),
    maxOutput: optional(wire.max_output),
    images: optional(wire.images),
    levels: optional(wire.levels),
  };
}

export function toModels(wire: WireModel[] | null | undefined): ProviderModel[] {
  return (wire ?? []).map((model) => ({ provider: model.provider, id: model.id, label: model.name, levels: model.levels }));
}

export function toMarketplacePlugin(wire: WireMarketplacePlugin): MarketplacePlugin {
  const servers = Object.entries(wire.servers ?? {})
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([name, server]) => ({
      name,
      address: server.url ?? [server.command ?? "", ...(server.args ?? [])].join(" "),
      isRemote: server.type === "http",
      signsIn: server.auth?.type === "oauth",
    }));
  return {
    id: wire.id,
    name: wire.name,
    description: wire.description ?? "",
    icon: wire.icon ?? "",
    homepage: optional(wire.homepage),
    author: wire.author ?? "",
    category: wire.category ?? "",
    isFeatured: wire.featured ?? false,
    tags: wire.tags ?? [],
    servers,
    skills: (wire.skills ?? []).map((skill) => ({ name: skill.name, description: skill.description ?? "" })),
    variables: (wire.variables ?? []).map((variable) => ({
      name: variable.name,
      description: variable.description ?? "",
      secret: variable.secret ?? false,
      required: variable.required ?? false,
    })),
    installedOn: wire.installed_on ?? [],
  };
}

export function toBotTemplate(wire: WireBotTemplate): BotTemplate {
  return {
    id: wire.id,
    name: wire.name,
    summary: wire.summary ?? "",
    description: wire.description,
    symbolName: wire.symbol_name ?? "sparkles",
    accent: wire.accent && isAccent(wire.accent) ? wire.accent : "indigo",
    category: wire.category ?? "",
    isFeatured: wire.featured ?? false,
    author: wire.author ?? "",
    plugins: wire.plugins ?? [],
    routines: (wire.routines ?? []).map((routine) => ({
      name: routine.name,
      scheduleText: routine.schedule_text ?? routine.schedule,
      prompt: routine.prompt,
    })),
    memory: wire.memory ?? [],
  };
}

export function toMarketplace(wire: { plugins: WireMarketplacePlugin[]; bots: WireBotTemplate[] }): Marketplace {
  return { plugins: wire.plugins.map(toMarketplacePlugin), bots: wire.bots.map(toBotTemplate) };
}

export function toPluginDetail(wire: WirePluginDetail): PluginDetail {
  return {
    status: toPlugin(wire.status),
    homepage: optional(wire.manifest.homepage),
    variables: wire.variables.map((variable) => ({
      name: variable.name,
      description: variable.description ?? "",
      secret: variable.secret,
      required: variable.required,
      isSet: variable.is_set,
      value: optional(variable.value),
    })),
    servers: wire.servers.map((server) => ({
      name: server.name,
      kind: server.kind,
      url: optional(server.auth.url),
      oauth: server.auth.oauth ?? false,
      signedIn: server.auth.signed_in ?? false,
      code: optional(server.auth.code),
      link: optional(server.auth.link),
    })),
    skills: wire.skills.map((skill) => ({ name: skill.name, description: skill.description ?? "" })),
  };
}

export function toBotMemory(wire: WireBotMemory): BotMemory {
  return {
    botID: wire.bot_id,
    here: wire.here,
    runner: wire.runner,
    path: wire.path ?? "",
    text: wire.index?.text ?? "",
    hash: wire.index?.hash ?? "",
    lines: wire.index?.lines ?? 0,
    bytes: wire.index?.bytes ?? 0,
    truncated: wire.index?.truncated ?? false,
    maxLines: wire.index?.max_lines ?? 200,
    maxBytes: wire.index?.max_bytes ?? 24_000,
    topics: wire.topics ?? [],
    logs: wire.logs ?? [],
  };
}
