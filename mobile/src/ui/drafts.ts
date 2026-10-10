// A bot's email or Slack message as its draft card and editor word it, after the Mac app's
// DraftCellView.

import type { Body, MessageDraft } from "../core/model";
import { t } from "../i18n";

export type DraftBody = Extract<Body, { kind: "draft" }>;

/// "Chef drafted an email", "a reply", or "a Slack message".
export function draftTitle(draft: MessageDraft, who: string): string {
  if (draft.kind !== "email") return t("{who} drafted a Slack message", { who });
  return draft.reply ? t("{who} drafted a reply", { who }) : t("{who} drafted an email", { who });
}

/// How it ended or where it stands, in a word or two; undefined while it waits.
export function draftStateWord(state: DraftBody["state"]): string | undefined {
  switch (state) {
    case "approved":
    case "executing":
      return t("Sending…");
    case "succeeded":
      return t("Sent");
    case "failed":
      return t("Not sent");
    case "rejected":
    case "cancelled":
      return t("Discarded");
    case "uncertain":
      return t("Not confirmed");
  }
  return undefined;
}

/// "186 KB", "1.5 MB", as Files counts.
export function fileSize(bytes: number): string {
  if (bytes >= 1_000_000) return `${(bytes / 1_000_000).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1000))} KB`;
}

/// A recipients field's text as a list: commas between them.
export function recipients(text: string): string[] {
  return text
    .split(",")
    .map((part) => part.trim())
    .filter(Boolean);
}

/// Whether the message can go: someone to send it to, and something to say.
export function sendable(draft: MessageDraft): boolean {
  return draft.to.length > 0 && draft.body.trim().length > 0;
}
