// A bot's browser profiles as the Mac's Profiles section words them: a state in a word or two, and
// the menu a tap on a profile opens. The phone is never a Runner, so a profile never opens here:
// its menu names the Runner where it does, and records only in a browser open on that screen.

import type { BrowserProfile } from "../core/model";
import { t, tc } from "../i18n";

export function profileStateWord(profile: BrowserProfile): string {
  if (profile.recording) return t("Recording");
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
  if (profile.recording) return t("Do the task in this browser on {runner}, then choose Stop Recording. Passwords aren't recorded.", { runner });
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
  id: "open" | "takeover" | "resume" | "record" | "stoprecording" | "screenshot" | "stop" | "delete";
  title: string;
  /// The SF Symbol iOS shows beside it.
  symbol: "macwindow" | "hand.raised" | "arrow.uturn.left" | "record.circle" | "stop.circle" | "camera.viewfinder" | "xmark.circle" | "trash";
  disabled?: boolean;
  destructive?: boolean;
};

/// The profile's menu, while nothing runs on it: `busy` leaves only Close Browser, which works while
/// a takeover waits. A chat (`inChat`) takes screenshots and recordings.
export function profileActions(profile: BrowserProfile, runner: string, inChat: boolean, busy: boolean): ProfileAction[] {
  const actions: ProfileAction[] = [];
  const record = () => inChat && actions.push({ id: "record", title: tc("Record", "browser"), symbol: "record.circle", disabled: busy });
  if (profile.recording) {
    actions.push({ id: "stoprecording", title: t("Stop Recording"), symbol: "stop.circle", disabled: busy || !inChat });
  } else if (profile.state === "stopped") {
    actions.push({ id: "open", title: t("Open on {runner}", { runner }), symbol: "macwindow", disabled: true });
    // The user does the task in the window on the Runner, so the recording starts there.
    if (inChat) actions.push({ id: "record", title: t("Record on {runner}", { runner }), symbol: "record.circle", disabled: true });
  } else if (profile.state === "bot") {
    actions.push({ id: "takeover", title: t("Take Over"), symbol: "hand.raised", disabled: busy });
    record();
  } else if (profile.state === "human") {
    actions.push({ id: "resume", title: t("Return to Bot"), symbol: "arrow.uturn.left", disabled: busy });
    record();
  }
  if ((profile.state === "bot" || profile.state === "human") && inChat) actions.push({ id: "screenshot", title: t("Take Screenshot"), symbol: "camera.viewfinder", disabled: busy });
  if (profile.state !== "stopped" || busy) actions.push({ id: "stop", title: t("Close Browser"), symbol: "xmark.circle" });
  actions.push({ id: "delete", title: t("Delete…"), symbol: "trash", destructive: true, disabled: busy });
  return actions;
}
