// The app's model, after the macOS app's Models.swift. Everything comes from the CLI; the wire
// shapes are mapped into these in wire.ts. Values are immutable: a change makes a new object, so
// the views can tell what changed by identity.

import { L, Lc } from "../l10n";
import * as Format from "./format";
import { markdownBlocks, plainText } from "./markdown";

// MARK: - Providers

/** The providers Lorca has built in. */
export type BuiltInProviderKind = "deepseek" | "anthropic" | "opencode" | "opencode-go" | "chatgpt" | "grok";

/** A provider the user added: any server that speaks OpenAI's or Anthropic's API. Its kind is
 * `custom:` and a slug of the name it was added with. */
export type CustomProviderKind = `custom:${string}`;

export type ProviderKind = BuiltInProviderKind | CustomProviderKind;

/** The built-in kinds, in the order Settings lists them. The account's custom providers follow
 * them, in the order they were added (`store.providerKinds`). */
export const providerKinds: BuiltInProviderKind[] = ["deepseek", "anthropic", "opencode", "opencode-go", "chatgpt", "grok"];

const customPrefix = "custom:";

export function isCustomKind(kind: string): kind is CustomProviderKind {
  return kind.startsWith(customPrefix);
}

/** A built-in kind or a custom one. Any other kind is one this build does not know. */
export function isProviderKind(value: string): value is ProviderKind {
  return (providerKinds as string[]).includes(value) || isCustomKind(value);
}

/** The name people know the provider by: "DeepSeek", "OpenCode Zen". A custom provider's is the one
 * the user gave it, from the account's `providers`, and its slug once it is deleted. */
export function providerName(kind: BuiltInProviderKind): string;
export function providerName(kind: ProviderKind, providers: readonly ProviderCredential[]): string;
export function providerName(kind: ProviderKind, providers: readonly ProviderCredential[] = []): string {
  if (isCustomKind(kind)) return providers.find((provider) => provider.kind === kind)?.name ?? kind.slice(customPrefix.length);
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

/** Connects with a pasted API key; ChatGPT and Grok sign in through the browser instead, and a
 * custom provider is set up in its own sheet. */
export function usesAPIKey(kind: ProviderKind): boolean {
  return kind === "deepseek" || kind === "anthropic" || kind === "opencode" || kind === "opencode-go";
}

export function providerSymbol(kind: ProviderKind): string {
  if (isCustomKind(kind)) return "server.rack";
  return usesAPIKey(kind) ? "key.fill" : "person.badge.key.fill";
}

export function providerSubtitle(kind: ProviderKind): string {
  if (isCustomKind(kind)) return L("Custom");
  return usesAPIKey(kind) ? L("API key") : L("Subscription");
}

/** What the sign-in needs, for the subscription providers. */
export function signInRequirement(kind: BuiltInProviderKind): string {
  if (kind === "chatgpt") return L("It needs a ChatGPT subscription.");
  if (kind === "grok") return L("It needs a SuperGrok or X Premium+ subscription.");
  return "";
}

/** The API root the CLI calls unless the credential names another. */
export function defaultBaseURL(kind: BuiltInProviderKind): string {
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
export function keyPlaceholder(kind: BuiltInProviderKind): string {
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
export function connectMethod(kind: BuiltInProviderKind): string {
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

/** A model the CLI's catalog offers, as the snapshot names it: its provider, id, and name, and
 * the thinking levels it takes, lowest first. */
export interface ProviderModel {
  provider: string;
  id: string;
  label: string;
  levels: string[];
}

/** The models a provider offers, in the catalog's order; the first is the default the CLI uses. */
export function providerModels(models: ProviderModel[], kind: ProviderKind): ProviderModel[] {
  return models.filter((model) => model.provider === kind);
}

/** Every thinking level, lowest first. */
const allThinkingLevels = ["off", "minimal", "low", "medium", "high", "xhigh", "max"];

/** The thinking levels `model` takes, lowest first: the provider's default model's when it is
 * undefined, and every level the provider's models take for a model the catalog does not have.
 * None on a bot means the model's default. */
export function thinkingLevels(models: ProviderModel[], kind: ProviderKind, model: string | undefined): { id: string; label: string }[] {
  const offered = providerModels(models, kind);
  const known = offered.find((each) => each.id === (model ?? offered[0]?.id));
  const ids = known?.levels ?? allThinkingLevels.filter((id) => offered.some((each) => each.levels.includes(id)));
  return ids.map((id) => ({ id, label: thinkingLabel(id) }));
}

/** The catalog with each custom provider's saved models after it, so the pickers offer them as
 * they do the catalog's: named as the provider's server names them, with the thinking levels the
 * CLI says each takes. */
export function withCustomModels(models: ProviderModel[], providers: readonly ProviderCredential[]): ProviderModel[] {
  const custom = providers.flatMap((provider) =>
    isCustomKind(provider.kind)
      ? (provider.models ?? []).map((model) => ({ provider: provider.kind, id: model.id, label: model.name ?? model.id, levels: model.levels ?? [] }))
      : [],
  );
  return [...models, ...custom];
}

/** One of the account's provider credentials, as statuses: never a key. */
export interface ProviderCredential {
  kind: ProviderKind;
  isConnected: boolean;
  detail: string;
  /** A custom API root, when the account credential has one. */
  baseURL?: string;
  /** A custom provider's name, the protocol its server speaks, and the models it offers. Built-in
   * providers have none. */
  name?: string;
  api?: CustomAPI;
  models?: CustomModel[];
}

/** The wire protocol a custom provider's server speaks. */
export type CustomAPI = "chat-completions" | "responses" | "messages";

/** Every protocol, in the order the custom provider sheet offers them. */
export const customAPIs: CustomAPI[] = ["chat-completions", "responses", "messages"];

export function isCustomAPI(value: string): value is CustomAPI {
  return (customAPIs as string[]).includes(value);
}

/** Product names, the same in every language. */
export function customAPITitle(api: CustomAPI): string {
  switch (api) {
    case "chat-completions":
      return "OpenAI Chat Completions";
    case "responses":
      return "OpenAI Responses";
    case "messages":
      return "Anthropic Messages";
  }
}

/** What the CLI adds to the base URL for a model call. */
export function customAPIPath(api: CustomAPI): string {
  switch (api) {
    case "chat-completions":
      return "/chat/completions";
    case "responses":
      return "/responses";
    case "messages":
      return "/v1/messages";
  }
}

export function customBaseURLPlaceholder(api: CustomAPI): string {
  return api === "messages" ? "https://api.example.com" : "https://api.example.com/v1";
}

/** The URL the CLI calls for a base URL as typed: a pasted endpoint is cut back to its root first,
 * as the CLI does (trailing slashes, then `/chat/completions`, `/responses`, or for Messages
 * `/v1/messages` or `/v1`), then the API's path goes on. */
export function customEndpoint(api: CustomAPI, baseURL: string): string {
  const root = baseURL.trim().replace(/\/+$/, "");
  const pasted = (api === "messages" ? ["/v1/messages", "/v1"] : [customAPIPath(api)]).find((path) => root.endsWith(path));
  return (pasted ? root.slice(0, -pasted.length) : root) + customAPIPath(api);
}

/** The note under the base URL field, naming the URL the CLI calls, so a base URL with a missing or
 * doubled path shows there. */
export function customEndpointNote(api: CustomAPI, baseURL: string): string {
  return baseURL.trim() === "" ? L("Lorca adds %@ to it.", customAPIPath(api)) : L("Requests go to %@.", customEndpoint(api, baseURL));
}

/** The base URL's host: "api.example.com". Empty until the base URL has one. */
export function customHost(baseURL: string): string {
  try {
    return new URL(baseURL.trim()).hostname;
  } catch {
    return "";
  }
}

/** A base URL the sheet can ask a server at for its models: http or https, with a host. */
export function isUsableBaseURL(baseURL: string): boolean {
  const url = baseURL.trim();
  return (url.startsWith("http://") || url.startsWith("https://")) && customHost(url) !== "";
}

/** The preset for a base URL as typed, by its host and port, so a URL pasted into an empty sheet
 * still finds the server's name and key hint. */
export function matchingPreset(baseURL: string): CustomPreset | undefined {
  let url: URL;
  try {
    url = new URL(baseURL.trim());
  } catch {
    return undefined;
  }
  if (url.hostname === "") return undefined;
  return customPresets.find((preset) => {
    const known = new URL(preset.baseURL);
    return known.hostname === url.hostname && known.port === url.port;
  });
}

/** What a provider left unnamed is saved as: the known server's name, else the base URL's host.
 * Empty without either. */
export function suggestedProviderName(baseURL: string): string {
  return matchingPreset(baseURL)?.name ?? customHost(baseURL);
}

/** A server people often add, with what the sheet fills in for it. */
export interface CustomPreset {
  /** A product name, the same in every language. */
  name: string;
  api: CustomAPI;
  baseURL: string;
  /** A model server on the user's own network, which takes no key. */
  local: boolean;
  /** The key field's placeholder, naming where the key comes from. Called while a view is built. */
  keyPlaceholder: () => string;
}

/** What Add Provider… offers, in its menu's order: hosted APIs, then servers on the user's network. */
export const customPresets: CustomPreset[] = [
  { name: "OpenAI", api: "responses", baseURL: "https://api.openai.com/v1", local: false, keyPlaceholder: () => L("sk-… from platform.openai.com") },
  { name: "OpenRouter", api: "chat-completions", baseURL: "https://openrouter.ai/api/v1", local: false, keyPlaceholder: () => L("sk-or-… from openrouter.ai/keys") },
  {
    name: "Gemini",
    api: "chat-completions",
    baseURL: "https://generativelanguage.googleapis.com/v1beta/openai",
    local: false,
    keyPlaceholder: () => L("Key from aistudio.google.com"),
  },
  { name: "Groq", api: "chat-completions", baseURL: "https://api.groq.com/openai/v1", local: false, keyPlaceholder: () => L("gsk_… from console.groq.com") },
  { name: "Together AI", api: "chat-completions", baseURL: "https://api.together.xyz/v1", local: false, keyPlaceholder: () => L("Key from api.together.ai") },
  { name: "Ollama", api: "chat-completions", baseURL: "http://localhost:11434/v1", local: true, keyPlaceholder: () => L("Optional for a server on your network") },
  { name: "LM Studio", api: "chat-completions", baseURL: "http://localhost:1234/v1", local: true, keyPlaceholder: () => L("Optional for a server on your network") },
];

/** The custom provider the account has under a preset's name, in any case, which the preset's menu
 * item opens instead of adding another. */
export function presetProvider(preset: CustomPreset, providers: readonly ProviderCredential[]): ProviderCredential | undefined {
  const name = preset.name.toLowerCase();
  return providers.find((provider) => isCustomKind(provider.kind) && provider.name?.toLowerCase() === name);
}

/** A model a custom provider offers, with what its server's model list says of it. */
export interface CustomModel {
  id: string;
  name?: string;
  contextWindow?: number;
  maxOutput?: number;
  /** Whether it takes images. */
  images?: boolean;
  /** The thinking levels the CLI says it takes, lowest first: in a provider's status only. */
  levels?: string[];
}

/** The name a model goes by in the list: what its server calls it, else its id. */
export function modelDisplayName(model: CustomModel): string {
  return model.name ?? model.id;
}

/** The custom provider sheet's model list: every model it knows in list order (saved, added by
 * hand, or listed by the server), the ones bots can pick, the ids the user typed in, and the
 * default, which is one of the picked ones or none. */
export interface ModelChecklist {
  models: readonly CustomModel[];
  selected: ReadonlySet<string>;
  added: ReadonlySet<string>;
  defaultID?: string;
}

/** A provider's saved models, all picked, the first the default; or an empty list. */
export function savedChecklist(models: readonly CustomModel[] = []): ModelChecklist {
  return { models, selected: new Set(models.map((model) => model.id)), added: new Set(), defaultID: models[0]?.id };
}

/** Takes a new listing: the models to keep (picked or added by hand) stay where they are with the
 * listing's facts, the rest of the old listing goes, and the new one follows. With nothing picked
 * yet, a list of eight models or fewer, as a model server on the user's network has, starts picked,
 * its first the default. An empty listing, a server with none, keeps only the models to keep. */
export function takeListing(checklist: ModelChecklist, listed: readonly CustomModel[]): ModelChecklist {
  const facts = new Map<string, CustomModel>();
  for (const model of listed) if (!facts.has(model.id)) facts.set(model.id, model);
  const keep = (id: string) => checklist.selected.has(id) || checklist.added.has(id);
  const models = checklist.models.filter((model) => keep(model.id)).map((model) => facts.get(model.id) ?? model);
  const seen = new Set(models.map((model) => model.id));
  for (const model of listed) {
    if (seen.has(model.id)) continue;
    seen.add(model.id);
    models.push(model);
  }
  if (checklist.selected.size > 0 || listed.length === 0 || listed.length > 8) return { ...checklist, models };
  return { ...checklist, models, selected: new Set(listed.map((model) => model.id)), defaultID: listed[0]?.id };
}

/** Picks or unpicks a model. The first one picked becomes the default; unpicking the default makes
 * the first picked one in the list the default. */
export function toggleModel(checklist: ModelChecklist, id: string): ModelChecklist {
  const selected = new Set(checklist.selected);
  let defaultID = checklist.defaultID;
  if (selected.delete(id)) {
    if (defaultID === id) defaultID = checklist.models.find((model) => selected.has(model.id))?.id;
  } else {
    selected.add(id);
    defaultID ??= id;
  }
  return { ...checklist, selected, defaultID };
}

/** A model the server does not list, picked, at the top of the list. It stays when another
 * listing replaces the server's. */
export function addModel(checklist: ModelChecklist, id: string): ModelChecklist {
  return {
    models: checklist.models.some((model) => model.id === id) ? checklist.models : [{ id }, ...checklist.models],
    selected: new Set(checklist.selected).add(id),
    added: new Set(checklist.added).add(id),
    defaultID: checklist.defaultID ?? id,
  };
}

/** The models whose name or id holds the search text, in any case. */
export function filterModels(models: readonly CustomModel[], query: string): readonly CustomModel[] {
  const text = query.trim().toLowerCase();
  if (text === "") return models;
  return models.filter((model) => model.id.toLowerCase().includes(text) || (model.name?.toLowerCase().includes(text) ?? false));
}

/** The id an Add row offers for the search text: one no model has yet. */
export function addCandidate(query: string, models: readonly CustomModel[]): string | undefined {
  const id = query.trim();
  return id === "" || models.some((model) => model.id === id) ? undefined : id;
}

/** The picked ids as the CLI keeps them: the default first, then the others in list order. */
export function orderedModelIDs(checklist: ModelChecklist): string[] {
  const picked = checklist.models.map((model) => model.id).filter((id) => checklist.selected.has(id));
  const defaultID = checklist.defaultID;
  return defaultID !== undefined && picked.includes(defaultID) ? [defaultID, ...picked.filter((id) => id !== defaultID)] : picked;
}

// MARK: - Device

/** `unknown` is a machine the relay lists that never said what it is: never a Runner. */
export type DeviceOS = "macos" | "linux" | "windows" | "ios" | "ipados" | "android" | "unknown";

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
    case "unknown":
      return "";
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
  /** The `lorca` this Device runs; empty when its CLI has not said. */
  version: string;
  /** How a CLI that replaces itself keeps current: one installed with the install script. Unset
   * where an app updates the CLI it carries, and on phones. */
  update?: DeviceUpdate;
}

/** A self-updating CLI's updates, as its `machine` blob tells every Device. */
export interface DeviceUpdate {
  /** It installs a newer release by itself and restarts into it once no bot is at work there. */
  auto: boolean;
  /** The newest release, when it is newer than `version`. */
  latest?: string;
  /** `installing` while it downloads and swaps the binary, `restarting` while it waits for its
   * bots to finish, `installed` when `lorca serve` must be restarted by hand to run it. */
  state?: "installing" | "restarting" | "installed";
  /** Why the last check or install failed, in the CLI's words. */
  error?: string;
}

/** Derived from `os` alone: a desktop Device is a Runner and can be assigned bots. */
export function isRunner(device: Device): boolean {
  return isDesktopOS(device.os);
}

export function roleLabel(device: Device): string {
  return isRunner(device) ? L("Runner") : L("Device");
}

/** What the Device panes say about a machine that never said what it is. */
export function unknownDeviceNote(): string {
  return L("This machine is paired to your account but has not sent its name or system. If you don't recognize it, unpair it.");
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
    case "unknown":
      return "questionmark.circle";
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
  /** `mcp.json` for one of the Runner's own MCP servers, which the server sheet edits. */
  source?: string;
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
  /** It runs in the background: the bot started it there, or the user sent it. Stop in the chat
   * leaves it running. */
  background: boolean;
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
  /** A codemode script's latest command, by its description, while no plugin call came after
   * it: the status line reads "Running command: Run the tests". */
  scriptCommand?: string;
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
  /** The message the user answers with this one, quoted. */
  replyTo?: ReplyQuote;
  /** A message of the user's the bot's turn holds for its next step; Send now has it read now. */
  queued?: boolean;
}

/** A message quoted by the user's reply: who wrote it and how it opens, as the CLI keeps it with the
 * reply, so the quote reads the same where the original has not loaded. */
export interface ReplyQuote {
  messageID: string;
  author: Author;
  text: string;
}

/** A finished text message, the user's or a bot's, which a reply can answer. */
export function canBeQuoted(message: Message): boolean {
  return message.body.kind === "text" && message.state.kind === "complete" && message.author.kind !== "system";
}

/** The quote of `message` the CLI makes, for a reply it has not confirmed yet: its words without the
 * Markdown, on one line, or the names of its files. */
export function quoteOf(message: Message): ReplyQuote | undefined {
  if (!canBeQuoted(message) || message.body.kind !== "text") return undefined;
  let line = plainText(markdownBlocks(message.body.text)).split(/\s+/).filter(Boolean).join(" ");
  if (line === "") line = message.attachments.map((attachment) => attachment.name).join(", ");
  if ([...line].length > 280) line = `${[...line].slice(0, 280).join("").trimEnd()}…`;
  return { messageID: message.id, author: message.author, text: line };
}

export function newMessageID(): string {
  return `msg-${crypto.randomUUID().toLowerCase()}`;
}

/** A `bash` row's command. */
export function commandRunOf(message: Message): CommandRun | undefined {
  return message.body.kind === "tool" ? message.body.tool.run : undefined;
}

/** A command running in its terminal that the bot's call still waits on: Run in Background sends
 * it there, and the call returns. */
export function runsInForeground(message: Message): boolean {
  if (message.body.kind !== "tool") return false;
  const { isRunning, run } = message.body.tool;
  return isRunning && !!run && takesInput(run) && !run.background;
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
  /** The group member holding the work, as the CLI last said. */
  ownerBotID?: string;
  /** What a group is for, which every member reads in its system prompt; empty for none. */
  groupDescription: string;
}

export const isGroup = (chat: Chat) => chat.kind === "group";
export const isDM = (chat: Chat) => chat.kind === "dm";
/** Whether another bot may join. Only groups grow, and never past the cap. */
export const canAddBot = (chat: Chat) => isGroup(chat) && chat.botIDs.length < maxGroupBots;
/** Whether a bot may leave. Groups keep at least one bot; DMs never change. */
export const canRemoveBot = (chat: Chat) => isGroup(chat) && chat.botIDs.length > 1;
/** A group's owner: the one set, else the first member, as the CLI picks. */
export function chatOwner(chat: Chat): string | undefined {
  if (!isGroup(chat)) return undefined;
  return chat.ownerBotID && chat.botIDs.includes(chat.ownerBotID) ? chat.ownerBotID : chat.botIDs[0];
}

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
