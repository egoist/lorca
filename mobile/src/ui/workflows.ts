// A workflow's setup as the core's `workflows.*` answers it, and what the pack's screen shows of it,
// in the Mac's words and by its rules: the account a service uses, each account row's state or the
// one thing to do about it, and the next step.

import type { ChannelListen, ChannelStatus, PluginStatus } from "../core/model";
import { t } from "../i18n";
import { pluginStateWord } from "./plugins";

export interface WorkflowPack {
  id: string;
  name: string;
  outcome: string;
  description: string;
  symbol_name: string;
  questions: { id: string; label: string; placeholder: string }[];
  connections: { service_id: string; name: string }[];
}

export interface WorkflowConnection {
  service_id: string;
  name: string;
  selected_id: string | null;
  /// The Runner's installed plugins for the service: its named accounts, or the plugin.
  choices: PluginStatus[];
  /// Whether the marketplace has the service, so a computer can add it.
  available: boolean;
}

export interface WorkflowProgress {
  setup: {
    id: string;
    runner_id: string;
    pack: WorkflowPack;
    answers: Record<string, string>;
    bot_ids: Record<string, string>;
    connection_ids: Record<string, string>;
    /// questions, connections, sample, reviewed, enabled, or cancelled.
    phase: string;
    sample: { job_id: string; chat_id: string; bot_id: string; state: "running" | "ready" | "failed" | "reviewed" } | null;
  };
  connections: WorkflowConnection[];
  /// The bot setup uses for each specialist, or none while it would add one, and the Runner's bots.
  specialists: { id: string; name: string; selected_id: string | null; choices: { id: string; name: string }[] }[];
  routines: { id: string; name: string; schedule_text: string; is_enabled: boolean }[];
  /// What the workflow listens to once it is on, with the Runner's channel then.
  channels?: { id: string; name: string; service_id: string; listen: ChannelListen; channel: ChannelStatus | null }[];
  sample_messages: { id: string; body: { text?: string } }[];
  is_running: boolean;
}

/// The account a service uses: the one chosen, else the Runner's only one, which setup takes.
export function accountOf(connection: WorkflowConnection): PluginStatus | undefined {
  if (connection.selected_id) return connection.choices.find((choice) => choice.id === connection.selected_id);
  return connection.choices.length === 1 ? connection.choices[0] : undefined;
}

/// An account row's state in a word or two, and whether a tap opens the account's own screen,
/// where it signs in. Adding an account is left to a computer, as everywhere on the phone.
export function accountRow(connection: WorkflowConnection): { detail: string; opens: boolean; needsComputer?: boolean } {
  const account = accountOf(connection);
  if (account) return { detail: account.state === "ready" ? t("Connected") : pluginStateWord(account), opens: true };
  // Just added on a computer: the Runner has yet to report it.
  if (connection.selected_id) return { detail: t("Connecting…"), opens: false };
  if (connection.choices.length > 1) return { detail: t("Not chosen"), opens: false };
  if (connection.available) return { detail: t("Not added"), opens: false, needsComputer: true };
  return { detail: t("Not available"), opens: false };
}

/// What the screen shows as typed and picked, against what setup holds: a bot is its id, or empty
/// for the new one setup adds.
export interface WorkflowDraft {
  answers: Record<string, string>;
  bots: Record<string, string>;
}

export function draftOf(progress: WorkflowProgress): WorkflowDraft {
  return {
    answers: Object.fromEntries(progress.setup.pack.questions.map((q) => [q.id, progress.setup.answers[q.id] ?? ""])),
    bots: Object.fromEntries(progress.specialists.map((s) => [s.id, s.selected_id ?? ""])),
  };
}

/// Whether the draft holds answers or bots that running the sample has not saved yet.
export function isEdited(progress: WorkflowProgress, draft: WorkflowDraft): boolean {
  return (
    progress.setup.pack.questions.some((q) => (draft.answers[q.id] ?? "").trim() !== (progress.setup.answers[q.id] ?? "")) ||
    progress.specialists.some((s) => (draft.bots[s.id] ?? "") !== (s.selected_id ?? ""))
  );
}

export type WorkflowStep =
  | { kind: "run"; disabled?: string }
  | { kind: "decide" }
  | { kind: "on" };

/// The next step: run the sample (with why it can't yet), then turn the schedule on or leave it off
/// once a result is in, and nothing once it is on.
export function nextStep(progress: WorkflowProgress, draft: WorkflowDraft): WorkflowStep {
  const { setup } = progress;
  if (setup.phase === "enabled") return { kind: "on" };
  const result = (setup.sample?.state === "ready" || setup.sample?.state === "reviewed") && progress.sample_messages.length > 0 && !progress.is_running;
  if (result && !isEdited(progress, draft)) return { kind: "decide" };
  if (progress.is_running) return { kind: "run", disabled: "" };
  if (setup.pack.questions.some((q) => !(draft.answers[q.id] ?? "").trim())) return { kind: "run", disabled: t("Fill in the setup first.") };
  if (progress.connections.some((c) => accountOf(c)?.state !== "ready")) return { kind: "run", disabled: t("Connect the accounts first.") };
  return { kind: "run" };
}

/// The answers and picked bots to save before a sample, trimmed, with the new bot left to setup.
export function configureParams(progress: WorkflowProgress, draft: WorkflowDraft) {
  return {
    id: progress.setup.id,
    answers: Object.fromEntries(progress.setup.pack.questions.map((q) => [q.id, (draft.answers[q.id] ?? "").trim()])),
    bot_ids: Object.fromEntries(Object.entries(draft.bots).filter(([, id]) => id)),
  };
}

/// The sample's replies as one Markdown text, a blank line between them.
export function sampleText(progress: WorkflowProgress): string {
  return progress.sample_messages
    .map((message) => message.body.text ?? "")
    .filter(Boolean)
    .join("\n\n");
}
