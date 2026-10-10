// What a secret request's card and a Runner's saved secrets say, as the Mac app says it.

import type { Body, SavedSecret } from "../core/model";
import { t } from "../i18n";

type SecretCard = Extract<Body, { kind: "permission" }>;

/// "Developer needs a secret for github.com", or "… for its commands".
export function secretTitle(body: SecretCard, who: string): string {
  const use = body.secret?.use;
  if (use === "command") return t("{who} needs a secret for its commands", { who });
  return t("{who} needs a secret for {place}", { who, place: use === "browser" ? (body.secret?.site ?? body.plugin_name) : body.plugin_name });
}

/// Under the title: why, while it asks; the answer and what was asked for, once answered.
export function secretCaption(body: SecretCard): string | undefined {
  if (body.decision === "pending") return body.reason;
  const decided: Record<string, string> = { allowed: t("Saved"), denied: t("Not now"), expired: t("No answer in time"), dismissed: t("Dismissed") };
  return `${decided[body.decision] ?? body.decision} · ${body.summary}`;
}

/// Where a saved secret goes: the site it is typed into, or the bot's commands.
export function secretPlace(secret: SavedSecret): string {
  return secret.use === "browser" ? (secret.site ?? t("Browser")) : t("Commands");
}
