// A bot's browser profiles as the Mac's Profiles section words them: a state in a word or two, and
// the menu a tap on a profile opens. The phone is never a Runner, so a profile never opens here:
// its menu names the Runner where it does.

import type { BrowserProfile } from "../core/model";
import { t, tc } from "../i18n";

export function profileStateWord(profile: BrowserProfile): string {
  switch (profile.state) {
    case "bot":
      return tc("Open", "browser");
    case "taking_over":
      return t("Taking over…");
    case "human":
      return t("You have control");
    case "stopped":
      return tc("Closed", "browser");
  }
}

/// What the state means for the bot, as the Mac's tooltip says it.
export function profileExplanation(profile: BrowserProfile, bot: string, runner: string): string {
  switch (profile.state) {
    case "bot":
      return t("{bot}'s browser calls use this profile.", { bot });
    case "taking_over":
      return t("Waiting for {bot}'s current step in the browser to finish.", { bot });
    case "human":
      return t("{bot}'s browser calls wait until you return the browser.", { bot });
    case "stopped":
      return t("Open it on {runner} to sign in. {bot} can open it too.", { runner, bot });
  }
}

export type ProfileAction = {
  id: "open" | "takeover" | "resume" | "screenshot" | "stop" | "delete";
  title: string;
  /// The SF Symbol iOS shows beside it.
  symbol: "macwindow" | "hand.raised" | "arrow.uturn.left" | "camera.viewfinder" | "xmark.circle" | "trash";
  disabled?: boolean;
  destructive?: boolean;
};

/// The profile's menu, while nothing runs on it: `busy` leaves only Close Browser, which works while
/// a takeover waits.
export function profileActions(profile: BrowserProfile, runner: string, canScreenshot: boolean, busy: boolean): ProfileAction[] {
  const actions: ProfileAction[] = [];
  if (profile.state === "stopped") actions.push({ id: "open", title: t("Open on {runner}", { runner }), symbol: "macwindow", disabled: true });
  if (profile.state === "bot") actions.push({ id: "takeover", title: t("Take Over"), symbol: "hand.raised", disabled: busy });
  if (profile.state === "human") actions.push({ id: "resume", title: t("Return to Bot"), symbol: "arrow.uturn.left", disabled: busy });
  if ((profile.state === "bot" || profile.state === "human") && canScreenshot) actions.push({ id: "screenshot", title: t("Take Screenshot"), symbol: "camera.viewfinder", disabled: busy });
  if (profile.state !== "stopped" || busy) actions.push({ id: "stop", title: t("Close Browser"), symbol: "xmark.circle" });
  actions.push({ id: "delete", title: t("Delete…"), symbol: "trash", destructive: true, disabled: busy });
  return actions;
}
