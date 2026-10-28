// Bot templates, after the macOS app's TemplateSharing: a bot shared as a link or a file, from
// which anyone makes a bot of their own. The core builds, encrypts, and reads them
// (`templates.*`); these are its answers and what the screens work out from them.

import { t } from "../i18n";

/// Where a link's page lives. Open in Lorca on it is the same address as `lorca://t/…`.
export const TEMPLATE_SITE = "https://lorca.app";

/// A piece of the bot as the template holds it, redacted already, with what a reader should look
/// at before sharing it: `email`, `phone`, `path`, `link`, `credential` (a key it removed).
export interface TemplateItem<T> {
  id: string;
  content: T;
  flags: string[];
}

export interface TemplateProfile {
  name: string;
  description: string;
  symbol_name: string;
  accent: string;
}

export interface TemplateSkill {
  name: string;
  description: string;
}

export interface TemplateRoutine {
  name: string;
  schedule: string;
  prompt: string;
  /// The schedule in words, in an import preview.
  schedule_text?: string | null;
}

/// What a bot has that a template can carry (`templates.contents`).
export interface TemplateContents {
  profile: TemplateItem<TemplateProfile>;
  skills: TemplateItem<TemplateSkill>[];
  memories: TemplateItem<string>[];
  routines: TemplateItem<TemplateRoutine>[];
  /// A plugin on the bot's Runner, by its marketplace service.
  requirements: { service_id: string; name: string }[];
}

/// What goes in a template besides the profile, by the ids `templates.contents` gave.
export interface TemplateSelection {
  profile: boolean;
  skill_ids: string[];
  memory_ids: string[];
  routine_ids: string[];
  requirement_ids: string[];
}

export type TemplateKind = "skill_ids" | "routine_ids" | "requirement_ids" | "memory_ids";

/// A bot the account shares as a link. The roster carries it to every Device, which lists,
/// updates, and revokes it.
export interface SharedLink {
  id: string;
  /// The whole address, the key after its `#`.
  url: string;
  bot_id: string;
  /// The bot's name when it was last shared.
  name: string;
  selection: TemplateSelection;
  created_at: number;
  updated_at: number;
}

/// A template as a file or a link holds it.
export interface TemplateDocument {
  profile?: TemplateProfile | null;
  skills?: TemplateSkill[];
  memories?: string[];
  routines?: TemplateRoutine[];
}

/// One of a Runner's own connections for a plugin the template uses.
export interface TemplateConnection {
  id: string;
  name: string;
  state: string;
}

/// A plugin the template uses, the Runner's connections for it, and the one picked: the user's
/// choice, else the Runner's only ready one.
export interface TemplatePlugin {
  service_id: string;
  name: string;
  candidates: TemplateConnection[];
  selected?: string | null;
}

/// What `templates.import.preview` says about a file or a link for the Runner picked.
export interface TemplateImportPreview {
  template?: TemplateDocument | null;
  digest: string;
  can_import: boolean;
  issues: string[];
  requirements: TemplatePlugin[];
}

/// A link to a shared bot as the core reads it, `https://lorca.app/t/<id>…#<key>`, from the page's
/// address or the apps' `lorca://t/…` (`lorca-dev://` for Lorca Dev); nothing for any other text.
/// A link without the key after `#` is not whole yet.
export function templateAddress(text: string): string | undefined {
  const trimmed = text.trim();
  const app = /^lorca(?:-dev)?:\/\/t\/(.+)$/i.exec(trimmed);
  const address = app ? `${TEMPLATE_SITE}/t/${app[1]}` : /^https?:\/\/[^/?#]+\/t\/[^#]+#./i.test(trimmed) ? trimmed : undefined;
  return address && /\/t\/[^/?#]+[^#]*#./.test(address) ? address : undefined;
}

/// The link the bot was last shared as.
export function sharedLinkFor(links: readonly SharedLink[], botId: string): SharedLink | undefined {
  return links.findLast((link) => link.bot_id === botId);
}

/// Settings' list: the newest change first.
export function newestLinks(links: readonly SharedLink[]): SharedLink[] {
  return [...links].sort((a, b) => b.updated_at - a.updated_at);
}

/// A link's address without the key, for a row: `lorca.app/t/yWUtxVvE…`.
export function linkTitle(url: string): string {
  return url.replace(/^https?:\/\//, "").replace(/[?#].*$/, "");
}

export function emptySelection(): TemplateSelection {
  return { profile: true, skill_ids: [], memory_ids: [], routine_ids: [], requirement_ids: [] };
}

/// The ids of each kind, in the bot's order.
export function templateOrder(contents: TemplateContents): Record<TemplateKind, string[]> {
  return {
    skill_ids: contents.skills.map((item) => item.id),
    routine_ids: contents.routines.map((item) => item.id),
    requirement_ids: contents.requirements.map((item) => item.service_id),
    memory_ids: contents.memories.map((item) => item.id),
  };
}

/// What the sheet starts with: the profile alone, or for a bot shared before what its link
/// holds, less what the bot no longer has.
export function initialSelection(contents: TemplateContents, link?: SharedLink): TemplateSelection {
  const selection = emptySelection();
  if (!link) return selection;
  const order = templateOrder(contents);
  for (const kind of Object.keys(order) as TemplateKind[]) selection[kind] = order[kind].filter((id) => link.selection[kind]?.includes(id));
  return selection;
}

/// Picks or drops one item; the template lists them in the bot's order, whatever order they
/// were picked in.
export function toggleItem(selection: TemplateSelection, kind: TemplateKind, id: string, order: readonly string[]): TemplateSelection {
  const picked = selection[kind].includes(id) ? selection[kind].filter((each) => each !== id) : [...selection[kind], id];
  return { ...selection, [kind]: order.filter((each) => picked.includes(each)) };
}

/// Select All, or Deselect All once everything is picked.
export function toggleAll(selection: TemplateSelection, kind: TemplateKind, order: readonly string[]): TemplateSelection {
  return { ...selection, [kind]: allPicked(selection, kind, order) ? [] : [...order] };
}

export function allPicked(selection: TemplateSelection, kind: TemplateKind, order: readonly string[]): boolean {
  return order.length > 0 && order.every((id) => selection[kind].includes(id));
}

/// How many items a list has before it offers Select All.
export const LONG_LIST = 6;

/// A memory reads as its first line, without Markdown's list or heading marks, over the rest.
export function memoryLines(text: string): { title: string; detail: string } {
  const lines = text
    .split(/\r?\n/)
    .map((line) => line.trim().replace(/^(#+|[-*+]|\d+\.)\s+/, ""))
    .filter(Boolean);
  return { title: lines[0] ?? text, detail: lines.slice(1).join(" ") };
}

/// A routine's schedule in words and what it asks.
export function routineLine(routine: TemplateRoutine): string {
  const schedule = routine.schedule_text || routine.schedule;
  return routine.prompt ? `${schedule} · ${routine.prompt}` : schedule;
}

/// The flags in a word or two, and whether one is personal, which the row tints.
export function flagWords(flags: readonly string[]): { text: string; personal: boolean } | undefined {
  const words = flags.flatMap((flag) => {
    switch (flag) {
      case "email":
        return [t("Email address")];
      case "phone":
        return [t("Phone number")];
      case "path":
        return [t("File path")];
      case "link":
        return [t("Link")];
      case "credential":
        return [t("Key removed")];
      default:
        return [];
    }
  });
  if (!words.length) return undefined;
  return { text: words.slice(0, 2).join(", "), personal: flags.some((flag) => flag !== "credential") };
}

export function isReadyPlugin(plugin: TemplatePlugin): boolean {
  return plugin.candidates.some((candidate) => candidate.id === plugin.selected && candidate.state === "ready");
}

/// What the import still needs, or what it will do, under the form: `warning` until it can go.
export function importNote(preview: TemplateImportPreview, runnerName: string | undefined): { text: string; warning: boolean } | undefined {
  if (runnerName === undefined) return { text: t("Pair a Runner first: a bot runs on a computer."), warning: true };
  if (preview.issues.length) return { text: preview.issues.join("\n"), warning: true };
  const plugin = preview.requirements.find((each) => !isReadyPlugin(each));
  if (plugin) {
    const text = !plugin.candidates.length
      ? t("Add {plugin} to {runner} from the Marketplace first.", { plugin: plugin.name, runner: runnerName })
      : !plugin.selected
        ? t("Choose the {plugin} connection this bot uses.", { plugin: plugin.name })
        : t("Finish setting up {plugin} on {runner} first.", { plugin: plugin.name, runner: runnerName });
    return { text, warning: true };
  }
  return preview.template?.routines?.length ? { text: t("Routines start paused."), warning: false } : undefined;
}

/// A connection in a plugin's menu: its name, and that it needs setup when it does.
export function connectionTitle(connection: TemplateConnection): string {
  return connection.state === "ready" ? connection.name : t("{name} (needs setup)", { name: connection.name });
}
