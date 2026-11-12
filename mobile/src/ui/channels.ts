// Channels in the phone's words: what a channel takes, where it listens, its state, and its intake,
// as the desktop app's channel rows and sheet say them.

import type { ChannelIntake, ChannelListen, ChannelStatus, ChatRequest, IntakeDecision, IntakeDecisionKind, IntakePerson, IntakeRule, TaskMandate } from "../core/model";
import { t, tc } from "../i18n";

/// "Mentions, replies, #feedback", or "Every message".
export function listenSummary(listen: ChannelListen): string {
  if (listen.every) return t("Every message");
  const parts = [...(listen.mentions ? [t("mentions")] : []), ...(listen.replies ? [t("replies")] : []), ...(listen.tags ?? []).map((tag) => `#${tag}`)];
  const joined = parts.join(tc(", ", "list"));
  return joined.charAt(0).toUpperCase() + joined.slice(1);
}

/// The service as people call it.
export function serviceName(service: string): string {
  return service === "slack" ? "Slack" : "Telegram";
}

/// The symbol that stands for the service on a row.
export function serviceSymbol(service: string): string {
  return service === "slack" ? "number" : "paperplane";
}

/// A state the user has something to do about, in a word or two; none while it listens fully or rests.
export function channelProblem(channel: ChannelStatus): string | undefined {
  if (channel.state === "held") return t("On hold");
  if (channel.state === "offline") return t("Can’t connect");
  const waiting = channel.requests?.length ?? 0;
  if (waiting === 1) return t("A chat is waiting");
  if (waiting > 1) return t("{count} chats are waiting", { count: waiting });
  // A listening channel's only detail is the Telegram privacy-mode note.
  if (channel.state === "listening" && channel.detail) return t("Privacy mode on");
  return undefined;
}

/// Where it listens: the chats the user allowed.
export function channelChats(channel: ChannelStatus): string {
  if (!channel.chats?.length) return t("None yet");
  return channel.chats.map((chat) => chat.title || chat.id).join(tc(", ", "list"));
}

/// What kind of chat asks: a direct message, or a group and who wrote there or added the bot.
export function requestKind(request: ChatRequest): string {
  if (request.private) return t("Direct message");
  return request.from ? t("Group · {name}", { name: request.from }) : t("Group");
}

// MARK: - Intake

/// What "Takes on work" offers: a channel's mode, which is Off with no intake too.
export type IntakeChoice = ChannelIntake["mode"];

export const INTAKE_CHOICES: IntakeChoice[] = ["off", "observe", "auto"];

export const intakeChoiceOf = (intake: ChannelIntake | null | undefined): IntakeChoice => intake?.mode ?? "off";

export function intakeChoiceTitle(choice: IntakeChoice): string {
  switch (choice) {
    case "off":
      return t("Off");
    case "observe":
      return t("Only Record");
    case "auto":
      return t("On");
  }
}

export function intakeChoiceFooter(choice: IntakeChoice): string {
  switch (choice) {
    case "off":
      return t("Each message the channel takes starts a turn.");
    case "observe":
      return t("The bot reads each message against your rules and records what it would take on. Someone you list still gets what they ask for.");
    case "auto":
      return t("The bot takes on what your rules cover, or what someone you list asks for, as a task in its own chat.");
  }
}

/// The intake a choice sends: every choice keeps the people and rules there are, Off included, so
/// one tap never loses what the owner wrote.
export function intakeWithChoice(intake: ChannelIntake | null | undefined, choice: IntakeChoice): ChannelIntake {
  return { mode: choice, people: intake?.people ?? [], rules: intake?.rules ?? [] };
}

export const personName = (person: IntakePerson): string => person.name || person.id;

export const intakeLists = (intake: ChannelIntake, id: string): boolean => !!intake.people?.some((person) => person.id === id);

/// `intake` with this person too; someone already listed stays as they are.
export function intakeWithPerson(intake: ChannelIntake, person: IntakePerson): ChannelIntake {
  return intakeLists(intake, person.id) ? intake : { ...intake, people: [...(intake.people ?? []), person] };
}

export function intakeWithoutPerson(intake: ChannelIntake, id: string): ChannelIntake {
  return { ...intake, people: intake.people?.filter((person) => person.id !== id) };
}

/// `intake` with this rule in the place of the one with its id, or after the others.
export function intakeWithRule(intake: ChannelIntake, rule: IntakeRule): ChannelIntake {
  const rules = intake.rules ?? [];
  return { ...intake, rules: rule.id && rules.some((each) => each.id === rule.id) ? rules.map((each) => (each.id === rule.id ? rule : each)) : [...rules, rule] };
}

export function intakeWithoutRule(intake: ChannelIntake, id: string): ChannelIntake {
  return { ...intake, rules: intake.rules?.filter((rule) => rule.id !== id) };
}

export function decisionKindTitle(kind: IntakeDecisionKind): string {
  switch (kind) {
    case "accepted":
      return t("Took on");
    case "would_accept":
      return t("Would take on");
    case "attached":
      return t("Went to its task");
    case "stopped":
      return t("Stopped");
    case "replied":
      return t("Answered");
    case "ignored":
      return t("Left alone");
    case "unavailable":
      return t("Couldn't assess");
  }
}

/// How many decisions the channel's screen lists.
export const DECISION_ROWS = 10;

/// The decisions the channel's screen lists, from the newest-first record: what was left alone says
/// nothing, and in a busy group it is most of the record.
export function shownDecisions(decisions: IntakeDecision[]): IntakeDecision[] {
  return decisions.filter((decision) => decision.kind !== "ignored").slice(0, DECISION_ROWS);
}

/// What the decision was about: the work's title, else why it went as it did.
export const decisionTitle = (decision: IntakeDecision): string => decision.title || decision.reason || decisionKindTitle(decision.kind);

/// The person a decision's long press can list: known to the service, and not listed yet.
export function decisionPerson(decision: IntakeDecision, intake: ChannelIntake): IntakePerson | undefined {
  if (!decision.sender_id || intakeLists(intake, decision.sender_id)) return undefined;
  return { id: decision.sender_id, name: decision.sender };
}

/// Who a mandated task came from, and through which channel.
export const mandateFrom = (mandate: TaskMandate): string => (mandate.sender ? `${mandate.sender} · ${mandate.channel_name}` : mandate.channel_name);

/// Why the work may be done: someone listed asked, or the rules it matched, which may be none on
/// record.
export function mandateWhy(mandate: TaskMandate): string {
  if (mandate.strength === "explicit") return t("Asked directly");
  return (mandate.rules ?? []).map((rule) => rule.name).join(tc(", ", "list"));
}
