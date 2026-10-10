// The account's email address, as the core says it stands (`mail.get`, the snapshot's `mail`,
// `mail.changed`). One address per account on the relay's mail domain, shared by every bot:
// each bot writes from its own `name+tag@domain`, and mail without a bot's tag goes to the lead
// bot. Nothing shows while the relay offers no email.

import { t } from "../i18n";

export interface MailAddress {
  name: string;
  email: string;
  /// `suspended`: mail to it bounces for now, because mail from it kept bouncing.
  state: "active" | "suspended";
}

export interface MailStatus {
  /// The relay offers email; without it the apps show nothing of it.
  available: boolean;
  domain: string | null;
  address: MailAddress | null;
  /// The bot that gets mail without a bot's tag.
  lead_bot_id: string | null;
  /// Each bot's own address.
  bots: { bot_id: string; email: string }[];
}

/// Why `mail.apply` took no address.
export type MailProblem = "taken" | "reserved" | "invalid";

/// What `mail.apply` answers.
export type MailApplied = { mail: MailStatus; problem?: undefined } | { problem: MailProblem; mail?: undefined };

/// A name as the user types it, the way the relay keeps it.
export function normalizeMailName(text: string): string {
  return text.trim().toLowerCase();
}

/// The relay's rule, checked here first so a bad name never leaves the phone: 3 to 32 lowercase
/// letters, digits, dots, and hyphens, starting and ending with a letter or digit, no `..`.
export function mailNameIsValid(name: string): boolean {
  return /^[a-z0-9](?:[a-z0-9.-]{1,30})[a-z0-9]$/.test(name) && !name.includes("..");
}

export function mailProblemText(problem: MailProblem): string {
  switch (problem) {
    case "taken":
      return t("That name is taken.");
    case "reserved":
      return t("That name is reserved.");
    case "invalid":
      return t("Use letters, digits, dots, and hyphens.");
  }
}

/// Where mail to the address goes, for the Email screen's footer: a bot's own address and the
/// lead bot that takes the rest, by name. With one bot, all mail is that bot's.
export function mailRoutingNote(mail: MailStatus, bots: { id: string; name: string }[]): string | undefined {
  const name = (id: string | null | undefined) => bots.find((bot) => bot.id === id)?.name;
  const lead = name(mail.lead_bot_id);
  const other = mail.bots.find((each) => each.bot_id !== mail.lead_bot_id && name(each.bot_id));
  if (other && lead) return t("Mail to {email} goes to {bot}, and other mail to {lead}.", { email: other.email, bot: name(other.bot_id)!, lead });
  if (lead) return t("Mail to this address goes to {lead}.", { lead });
  return undefined;
}
