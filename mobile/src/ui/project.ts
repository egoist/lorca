// A group's project context in its details, after the desktop apps' Project section: the words
// for each kind, who saved an entry, and what a row says needs the user.

import { isSuggestion, otherVersions, projectHost, type ProjectContext, type ProjectEntry, type ProjectKind } from "../core/model";
import { t } from "../i18n";
import { monthDay } from "./format";

/// The words a kind goes by; the core calls a link `document` and a file `asset`.
export function projectKindTitle(kind: ProjectKind): string {
  return { brief: t("Brief"), goal: t("Goal"), constraint: t("Constraint"), decision: t("Decision"), fact: t("Fact"), document: t("Link"), asset: t("File") }[kind];
}

export function projectNewTitle(kind: ProjectKind): string {
  return { brief: t("New Brief"), goal: t("New Goal"), constraint: t("New Constraint"), decision: t("New Decision"), fact: t("New Fact"), document: t("New Link"), asset: t("New File") }[kind];
}

/// Who wrote the entry, or where it came from.
export function projectProvenance(entry: ProjectEntry): string {
  const day = monthDay(new Date(entry.updated_at * 1000));
  switch (entry.source.kind) {
    case "user":
      return t("Saved by you on {date}", { date: day });
    case "bot":
      return isSuggestion(entry) ? t("Suggested by {name} on {date}", { name: entry.source.label, date: day }) : t("Added by {name} on {date}", { name: entry.source.label, date: day });
    case "url":
      return t("From {source}", { source: projectHost(entry) ?? entry.source.label });
    default:
      return t("From {source} on {date}", { source: entry.source.label, date: day });
  }
}

/// What needs the user about an entry, in a word or two: two versions of it, or a link that could
/// not be opened.
export function projectProblem(context: ProjectContext | undefined, entry: ProjectEntry): string | undefined {
  if (otherVersions(context, entry.id).length > 0) return t("Two versions");
  if (entry.freshness === "unavailable") return t("Unavailable");
  return undefined;
}

/// A row's second line: its kind, then what needs the user, who suggested it, or its link's host.
export function projectRowDetail(context: ProjectContext | undefined, entry: ProjectEntry): string {
  const detail = projectProblem(context, entry) ?? (isSuggestion(entry) ? t("Suggested by {name}", { name: entry.source.label }) : projectHost(entry));
  return detail ? `${projectKindTitle(entry.kind)} · ${detail}` : projectKindTitle(entry.kind);
}
