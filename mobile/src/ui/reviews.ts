// What a bot left for review, in the words the desktop apps use: the permission card's title, the
// row's line, and how it ended.

import type { Bot, Device, ReviewItem } from "../core/model";
import { t } from "../i18n";

/// The installed plugin, or account of one, a call goes to, by the name its Runner gives it
/// ("GitHub", "Gmail · Work"); the call's own account words once it is gone.
export function reviewPluginName(item: ReviewItem, runner: Device | undefined): string {
  const id = item.payload.kind === "plugin" ? item.payload.plugin_id : undefined;
  return runner?.plugins?.find((plugin) => plugin.id === id)?.name ?? item.target.account;
}

/// "Chef wants to run a command on Workbench", as the permission card says it.
export function reviewTitle(item: ReviewItem, bot: Bot | undefined, runner: Device | undefined): string {
  const who = bot?.name ?? t("The bot");
  switch (item.payload.kind) {
    case "draft":
      return t("{who} wrote a draft", { who });
    case "shell":
      return t("{who} wants to run a command on {plugin}", { who, plugin: runner?.name ?? t("its Runner") });
    default:
      return t("{who} wants to use {plugin}", { who, plugin: reviewPluginName(item, runner) });
  }
}

/// The command's first line, the plugin a call goes to, or what a draft is for.
export function reviewHeadline(item: ReviewItem, runner: Device | undefined): string {
  switch (item.payload.kind) {
    case "shell":
      return firstLine(item.payload.arguments.command ?? "");
    case "draft":
      return item.target.resource;
    default:
      return reviewPluginName(item, runner);
  }
}

export function reviewSymbol(item: ReviewItem, runner: Device | undefined): string {
  switch (item.payload.kind) {
    case "shell":
      return "terminal";
    case "draft":
      return "doc.text";
    default: {
      const id = item.payload.plugin_id;
      return runner?.plugins?.find((plugin) => plugin.id === id)?.icon || "puzzlepiece.extension";
    }
  }
}

/// How it ended or where it stands, in a word or two; undefined while it waits for the user.
export function reviewStateWord(item: ReviewItem): string | undefined {
  switch (item.state) {
    case "approved":
    case "executing":
      return t("Running…");
    case "succeeded":
      return item.payload.kind === "draft" ? t("Accepted") : t("Done");
    case "failed":
      return t("Failed");
    case "rejected":
      return t("Rejected");
    case "cancelled":
      return t("Cancelled");
    case "uncertain":
      return t("Didn't finish");
    default:
      return undefined;
  }
}

const firstLine = (text: string) =>
  text
    .split("\n")
    .map((line) => line.trim())
    .find(Boolean) ?? text;
