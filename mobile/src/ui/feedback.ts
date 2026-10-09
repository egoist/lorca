// Workflow feedback in the words the desktop apps use: what the user said about a bot's messages,
// the changes to its routines and skills the bot suggests from it, and the changes accepted. The
// calls are in `src/core/feedback.ts`.

import type { Chat, Message, Routine } from "../core/model";
import { t } from "../i18n";

/// A routine's task, or a saved skill of the bot's or of a group it is in. Sent back as it came.
export interface FeedbackTarget {
  kind: string;
  id?: string;
  scope?: { kind: string; id: string };
}

export type FeedbackKind = "accepted" | "rejected" | "edited" | "explicit" | "routine_failure" | "ignored_alert";

export interface FeedbackNote {
  id: string;
  kind: FeedbackKind;
  chatId: string;
  messageId: string;
  /// The user's words, else the start of the message it is about.
  text: string;
  target?: FeedbackTarget;
  /// Unix seconds.
  createdAt: number;
}

export interface FeedbackSuggestion {
  id: string;
  target: FeedbackTarget;
  explanation: string;
  diff: string;
  diffHash: string;
  evidence: string[];
  createdAt: number;
}

export interface FeedbackChange {
  id: string;
  target: FeedbackTarget;
  diff: string;
  isUndo: boolean;
  canUndo: boolean;
  currentHash?: string;
  createdAt: number;
}

export interface BotFeedback {
  notes: FeedbackNote[];
  noteCount: number;
  suggestions: FeedbackSuggestion[];
  changes: FeedbackChange[];
  /// Seconds between the bot's looks for changes; undefined when it looks only when asked.
  reviewEvery?: number;
  targets: { name: string; target: FeedbackTarget }[];
}

/// What `feedback.list` answers.
export interface WireFeedback {
  feedback: { id: string; kind: FeedbackKind; origin: { chat_id: string; message_id: string }; note: string; example: string; target?: FeedbackTarget | null; created_at: number }[];
  feedback_count: number;
  proposals: { id: string; target: FeedbackTarget; explanation: string; diff: string; diff_hash: string; evidence: string[]; created_at: number }[];
  revisions: { id: string; target: FeedbackTarget; created_at: number; rollback_of?: string | null; diff: string; can_rollback: boolean; current_hash?: string | null }[];
  settings: { review_every_secs?: number | null };
  targets: { name: string; target: FeedbackTarget }[];
}

/// The list, newest first.
export function toBotFeedback(wire: WireFeedback): BotFeedback {
  return {
    notes: wire.feedback.map((n) => ({
      id: n.id,
      kind: n.kind,
      chatId: n.origin.chat_id,
      messageId: n.origin.message_id,
      text: (n.note || n.example).trim(),
      target: n.target ?? undefined,
      createdAt: n.created_at,
    })),
    noteCount: wire.feedback_count,
    suggestions: [...wire.proposals].reverse().map((p) => ({ id: p.id, target: p.target, explanation: p.explanation, diff: p.diff, diffHash: p.diff_hash, evidence: p.evidence, createdAt: p.created_at })),
    changes: wire.revisions.map((r) => ({ id: r.id, target: r.target, diff: r.diff, isUndo: !!r.rollback_of, canUndo: r.can_rollback, currentHash: r.current_hash ?? undefined, createdAt: r.created_at })),
    reviewEvery: wire.settings.review_every_secs ?? undefined,
    targets: wire.targets,
  };
}

export const isEmpty = (f: BotFeedback) => f.noteCount === 0 && f.suggestions.length === 0 && f.changes.length === 0 && f.reviewEvery === undefined;

export const sameTarget = (a: FeedbackTarget, b: FeedbackTarget) => a.kind === b.kind && a.id === b.id && a.scope?.kind === b.scope?.kind && a.scope?.id === b.scope?.id;

/// What the user calls a target: its routine's or skill's name.
export function targetName(f: BotFeedback, target: FeedbackTarget, routines: Routine[]): string {
  return f.targets.find((known) => sameTarget(known.target, target))?.name ?? routines.find((r) => target.kind === "routine_prompt" && r.id === target.id)?.name ?? target.id ?? t("Workflow");
}

/// "4 notes · 1 change".
export function countsLine(f: BotFeedback): string {
  const notes = f.noteCount === 1 ? t("1 note") : t("{count} notes", { count: f.noteCount });
  if (f.changes.length === 0) return notes;
  return `${notes} · ${f.changes.length === 1 ? t("1 change") : t("{count} changes", { count: f.changes.length })}`;
}

export function kindSymbol(kind: FeedbackKind): string {
  switch (kind) {
    case "accepted":
      return "checkmark.circle";
    case "rejected":
      return "xmark.circle";
    case "edited":
      return "pencil";
    case "routine_failure":
      return "exclamationmark.triangle";
    case "ignored_alert":
      return "eye.slash";
    default:
      return "bubble.left";
  }
}

export function kindTitle(kind: FeedbackKind): string {
  switch (kind) {
    case "accepted":
      return t("Good");
    case "rejected":
      return t("Not right");
    case "edited":
      return t("Corrected");
    case "routine_failure":
      return t("Run failed");
    case "ignored_alert":
      return t("Ignored");
    default:
      return t("Note");
  }
}

/// The responses the form offers, in its order.
export const responses: { kind: FeedbackKind; title: () => string; placeholder: () => string }[] = [
  { kind: "accepted", title: () => t("Looks good"), placeholder: () => t("What worked? (optional)") },
  { kind: "rejected", title: () => t("Not what I wanted"), placeholder: () => t("What was wrong? (optional)") },
  { kind: "edited", title: () => t("I corrected it"), placeholder: () => t("What did you change? (optional)") },
  { kind: "explicit", title: () => t("A note for next time"), placeholder: () => t("What should it do next time?") },
];

/// A note's title: its words, or the routine a failed run belongs to.
export function noteTitle(f: BotFeedback, note: FeedbackNote, routines: Routine[]): string {
  const title = note.kind === "routine_failure" && note.target ? targetName(f, note.target, routines) : note.text;
  return title || kindTitle(note.kind);
}

/// The changed lines of the CLI's diff, without its file and hunk headers.
export function diffLines(diff: string): { removed: boolean; text: string }[] {
  return diff.split("\n").flatMap((line): { removed: boolean; text: string }[] => {
    if (line.startsWith("---") || line.startsWith("+++") || line.startsWith("@@")) return [];
    if (line.startsWith("-")) return [{ removed: true, text: line.slice(1) }];
    if (line.startsWith("+")) return [{ removed: false, text: line.slice(1) }];
    return [];
  });
}

/// What the Runner refused, in the app's terms.
export function feedbackProblem(message: string, name: string): string {
  if (message.includes("review a fresh proposal") || message.includes("cannot overwrite later edits")) return t("“{name}” changed after this, so the change can't be made.", { name });
  if (message.includes("displayed diff changed") || message.includes("no longer pending")) return t("This suggestion changed on another Device.");
  if (message.includes("no longer included")) return t("This suggestion is based on feedback you excluded.");
  return message;
}

/// The bot's replies a note can be about, newest first.
export function feedbackSources(chat: Chat): Message[] {
  return chat.messages
    .filter((m) => m.author.kind === "bot" && m.body.kind === "text" && m.body.text.trim().length > 0 && m.state.kind === "complete")
    .slice(-8)
    .reverse();
}
