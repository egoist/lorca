// A bot's Access, after the desktop apps' Access sheet: how far it may use each plugin on its
// Runner, down to single tools, and whether it reads or changes files and runs shell commands
// there. Only the user changes it; here each change saves at once, as iOS Settings does. The
// plugins and the tools each offered come from the Runner (`bots.permissions`, a sealed request
// through the core); the policy itself is in the roster.

import { create } from "zustand";
import * as core from "../../modules/lorca-core";
import { t } from "../i18n";
import type { BotPermissions } from "./model";
import { useStore } from "./store";

/// How far a grant reaches, each level taking in the ones below it: a plugin's read, draft, and
/// write capabilities, or files read or changed.
export type AccessLevel = "write" | "draft" | "read" | "none";

export const LEVELS: AccessLevel[] = ["write", "draft", "read", "none"];

/// Files have no drafts: read and write, read only, or none.
export type FilesLevel = Exclude<AccessLevel, "draft">;

export interface AccessTool {
  name: string;
  title?: string | null;
  description?: string | null;
  /** What it does, as its Runner last read it: read, draft, or write. */
  capability?: string | null;
}

export interface AccessPlugin {
  id: string;
  name: string;
  tools: AccessTool[];
}

export function capabilities(level: AccessLevel): string[] {
  return { write: ["read", "draft", "write"], draft: ["read", "draft"], read: ["read"], none: [] }[level];
}

export function levelOf(caps: string[] | undefined): AccessLevel {
  if (caps?.includes("write")) return "write";
  if (caps?.includes("draft")) return "draft";
  if (caps?.includes("read")) return "read";
  return "none";
}

/// What the bot may do with a plugin; a plugin a policy's map leaves out is off.
export function pluginLevel(policy: BotPermissions | undefined, pluginId: string): AccessLevel {
  return policy?.connections ? levelOf(policy.connections[pluginId]?.capabilities) : "write";
}

export function filesLevel(policy: BotPermissions | undefined): FilesLevel {
  return policy?.filesystem ?? "write";
}

export function shellOn(policy: BotPermissions | undefined): boolean {
  return policy?.shell ?? true;
}

/// Whether the bot's emails and Slack messages wait in the chat as drafts.
export function draftsOn(policy: BotPermissions | undefined): boolean {
  return policy?.drafts ?? true;
}

/// The tools chosen for a plugin; none is all of them, one it adds later included.
export function chosenTools(policy: BotPermissions | undefined, pluginId: string): string[] | undefined {
  return policy?.connections?.[pluginId]?.tools;
}

export function isFullAccess(policy: BotPermissions | undefined): boolean {
  return !policy?.connections && filesLevel(policy) === "write" && shellOn(policy);
}

export function reaches(level: AccessLevel, capability: string | null | undefined): boolean {
  return capabilities(level).includes(capability || "write");
}

/// The policy with `pluginId` at `level` and with `tools`, the other listed plugins as they are.
/// Every plugin at Read and write with all its tools is the Runner's every plugin, one installed
/// later included; otherwise only the plugins given a level are listed.
export function withPlugin(policy: BotPermissions | undefined, plugins: AccessPlugin[], pluginId: string, level: AccessLevel, tools: string[] | undefined): BotPermissions {
  const grants = plugins.map((plugin) =>
    plugin.id === pluginId ? { id: plugin.id, level, tools } : { id: plugin.id, level: pluginLevel(policy, plugin.id), tools: chosenTools(policy, plugin.id) },
  );
  const open = grants.every((grant) => grant.level === "write" && !grant.tools);
  const connections: BotPermissions["connections"] = {};
  for (const grant of grants) {
    if (grant.level === "none") continue;
    connections[grant.id] = { capabilities: capabilities(grant.level), ...(grant.tools ? { tools: [...grant.tools].sort() } : {}) };
  }
  return { ...(open ? {} : { connections }), filesystem: filesLevel(policy), shell: shellOn(policy) };
}

/// The policy with one tool of `plugin` chosen or left out. Every tool chosen is all of them.
export function withTool(policy: BotPermissions | undefined, plugins: AccessPlugin[], plugin: AccessPlugin, name: string, on: boolean): BotPermissions {
  const all = plugin.tools.map((tool) => tool.name);
  const chosen = new Set(chosenTools(policy, plugin.id) ?? all);
  if (on) chosen.add(name);
  else chosen.delete(name);
  const tools = all.every((each) => chosen.has(each)) ? undefined : [...chosen];
  return withPlugin(policy, plugins, plugin.id, pluginLevel(policy, plugin.id), tools);
}

// MARK: - Words

export function accessSummary(policy: BotPermissions | undefined): string {
  return isFullAccess(policy) ? t("Full access") : t("Limited");
}

export function levelTitle(level: AccessLevel): string {
  return { write: t("Read and write"), draft: t("Read and draft"), read: t("Read only"), none: t("No access") }[level];
}

/// What a tool does, in a word.
export function toolDoes(capability: string | null | undefined): string {
  return capability === "read" ? t("Reads") : capability === "draft" ? t("Drafts") : t("Changes");
}

/// "All tools", or how many of them the bot may use. Nothing before the plugin has connected
/// once, or when it is off.
export function toolsSummary(policy: BotPermissions | undefined, plugin: AccessPlugin): string | undefined {
  if (plugin.tools.length === 0 || pluginLevel(policy, plugin.id) === "none") return undefined;
  const chosen = chosenTools(policy, plugin.id);
  if (!chosen) return t("All tools");
  return t("{count} of {total} tools", { count: plugin.tools.filter((tool) => chosen.includes(tool.name)).length, total: plugin.tools.length });
}

// MARK: - The Runner's plugins, and saving

/// Each bot's plugins and their tools as its Runner last answered; `failed` when it could not.
export const useAccessStore = create<{ byBot: Record<string, AccessPlugin[] | "failed"> }>(() => ({ byBot: {} }));

/// Asks the bot's Runner for its plugins and their tools. One that cannot be asked keeps what was
/// shown, or marks the list as failed when there was none.
export async function loadAccessCatalog(botId: string): Promise<void> {
  try {
    const { connections } = await core.request<{ connections: AccessPlugin[] }>("bots.permissions", { id: botId });
    useAccessStore.setState((s) => ({ byBot: { ...s.byBot, [botId]: connections.map((plugin) => ({ ...plugin, tools: plugin.tools ?? [] })) } }));
  } catch (error) {
    console.warn("loading the bot's plugins", error instanceof Error ? error.message : error);
    useAccessStore.setState((s) => (Array.isArray(s.byBot[botId]) ? s : { byBot: { ...s.byBot, [botId]: "failed" } }));
  }
}

/// Saves the bot's Access, shown at once; the CLI also dismisses the access requests the bot
/// left. A save that fails puts the old Access back and says why.
export async function setBotPermissions(botId: string, policy: BotPermissions): Promise<void> {
  const before = useStore.getState().bots.find((bot) => bot.id === botId)?.permissions;
  const put = (permissions: BotPermissions | undefined) =>
    useStore.setState((s) => ({ bots: s.bots.map((bot) => (bot.id === botId ? { ...bot, permissions } : bot)) }));
  put(policy);
  try {
    await core.request("bots.update", { id: botId, permissions: policy });
  } catch (error) {
    put(before);
    throw error;
  }
}
