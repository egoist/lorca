// How a routine stands, in the words the Details row and the routine screen use, after the Mac
// app's Routine and RoutineProblem (Model/Models.swift): what went wrong in a word or two, what
// happened and how to fix it, the schedule with its timezone, and its last check.

import type { Routine } from "../core/model";
import { t } from "../i18n";
import { daySeparator, scheduleText, upcoming } from "./format";

/// What went wrong with a routine: three failed sign-ins in a row paused it, of its runs
/// (`model`) or its check; its Runner is offline; its checks or runs can't connect or sign in and
/// try again; its check failed, or called something that could change things.
export type RoutineProblem =
  | { kind: "signedOut"; model: boolean }
  | { kind: "offline" }
  | { kind: "cantConnect"; model: boolean }
  | { kind: "signInFailed"; model: boolean }
  | { kind: "checkFailed" }
  | { kind: "checkBlocked" };

/// What went wrong, while something did: the core's `state` with the kind of failure.
export function routineProblem(routine: Routine): RoutineProblem | undefined {
  if (routine.is_running) return undefined;
  const health = routine.health;
  const model = !!health?.model?.status;
  switch (routine.state) {
    case "blocked":
      if (routine.paused_reason !== "authentication") return { kind: "checkBlocked" };
      return { kind: "signedOut", model: (health?.model?.authentication_failures ?? 0) >= 3 };
    case "waiting_for_runner":
      return { kind: "offline" };
    case "failed":
      if (model) return (health?.model?.authentication_failures ?? 0) > 0 ? { kind: "signInFailed", model: true } : { kind: "cantConnect", model: true };
      if ((health?.authentication_failures ?? 0) > 0) return { kind: "signInFailed", model: false };
      if ((health?.connection_failures ?? 0) > 0) return { kind: "cantConnect", model: false };
      return { kind: "checkFailed" };
  }
  return undefined;
}

/// One or two words for the row and the screen's state.
export function problemWord(problem: RoutineProblem): string {
  switch (problem.kind) {
    case "signedOut":
      return t("Needs sign-in");
    case "offline":
      return t("Waiting for Runner");
    case "cantConnect":
      return t("Can’t connect");
    case "signInFailed":
      return t("Sign-in failed");
    case "checkFailed":
    case "checkBlocked":
      return t("Check failed");
  }
}

/// Whether the user has to do something; a connection that fails is tried again on its own.
export const problemNeedsUser = (problem: RoutineProblem) => problem.kind !== "cantConnect";

/// What happened and how to fix it, for the routine screen.
export function problemExplanation(problem: RoutineProblem, bot: string, runner: string): string {
  switch (problem.kind) {
    case "signedOut":
      return problem.model
        ? t("The model provider turned down three sign-ins in a row, so the routine is paused. Reconnect the provider in Settings, then resume it.")
        : t("The check couldn’t sign in to a plugin three times in a row, so the routine is paused. Sign in to the plugin again on {runner}, then resume it.", { runner });
    case "offline":
      return t("{runner} is offline, so the routine waits for it. To keep it available while the app is closed, run lorca service install on it.", { runner });
    case "cantConnect":
      return problem.model
        ? t("The last run couldn’t reach the model provider. It tries again at the next run, waiting longer after each failure.")
        : t("The last check couldn’t connect. It tries again at the next check, waiting longer after each failure.");
    case "signInFailed":
      return problem.model
        ? t("The last run couldn’t sign in to the model provider. Reconnect it in Settings; after three failures in a row the routine pauses.")
        : t("The last check couldn’t sign in to a plugin. Sign in to it again on {runner}; after three failures in a row the routine pauses.", { runner });
    case "checkFailed":
      return t("The check stopped with an error. {bot} got the error and can fix the check.", { bot });
    case "checkBlocked":
      return t("The check tried to change something, or to use something this bot's Access leaves out. Ask {bot} to fix it.", { bot });
  }
}

/// The offset from UTC, in minutes, that `zone` keeps at `at`; undefined for a zone the phone
/// doesn't know.
function offsetIn(zone: string, at: Date): number | undefined {
  try {
    // An hour of 24 for midnight, as some engines write it, rolls over in Date.UTC.
    const parts = new Intl.DateTimeFormat("en-US", { timeZone: zone, hour12: false, year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric" }).formatToParts(at);
    const part = (type: string) => Number(parts.find((p) => p.type === type)?.value);
    return Math.round((Date.UTC(part("year"), part("month") - 1, part("day"), part("hour"), part("minute")) - at.getTime()) / 60_000);
  } catch {
    return undefined;
  }
}

/// The routine's timezone, as "New York time", when the phone keeps other hours now or in half a
/// year; none for an interval, which counts time whatever the zone. The Mac says it after the
/// schedule; the phone's Schedule section gives it a row's value.
export function timezoneLabel(routine: Routine, now = new Date()): string | undefined {
  const zone = routine.timezone;
  if (!zone || routine.schedule.startsWith("every ")) return undefined;
  const differs = [now, new Date(now.getTime() + 182 * 86_400_000)].some((at) => {
    const there = offsetIn(zone, at);
    return there !== undefined && there !== -at.getTimezoneOffset();
  });
  return differs ? t("{city} time", { city: zone.slice(zone.lastIndexOf("/") + 1).replace(/_/g, " ") }) : undefined;
}

/// When the routine's check last ran and how it went ("Nothing new"), for a routine with a
/// check that has run.
export function lastCheck(routine: Routine): { when: string; outcome?: string } | undefined {
  const at = routine.health?.last_check_at;
  if (!routine.check || !at) return undefined;
  const outcome = { quiet: t("nothing new"), ready: t("found something"), failed: t("failed"), blocked: t("failed") }[routine.health?.status ?? ""];
  return { when: daySeparator(new Date(at * 1000)), outcome: outcome && capitalized(outcome) };
}

/// When the routine last ran and how it ended ("Replied"), or Never.
export function lastRun(routine: Routine): { when: string; outcome?: string } {
  if (!routine.last_run_at) return { when: t("Never") };
  const outcome = { sent: t("replied"), pass: t("nothing to report"), error: t("failed") }[routine.last_outcome ?? ""];
  return { when: daySeparator(new Date(routine.last_run_at * 1000)), outcome: outcome && capitalized(outcome) };
}

/// A word that starts a line: "Nothing new". Chinese has no case and reads as it is.
const capitalized = (text: string) => text.charAt(0).toUpperCase() + text.slice(1);

/// Whether the last check failed, so the last successful one says something.
export const checkIsFailing = (routine: Routine) => routine.health?.status === "failed" || routine.health?.status === "blocked";

/// "Tomorrow 9:00 AM": the next run on a line of its own, as the last check and run read.
export function nextRunText(routine: Routine): string | undefined {
  if (!routine.next_run_at) return undefined;
  return capitalized(upcoming(routine.next_run_at));
}

/// What happens to runs the Runner missed: the value, and what it means.
export function missedRuns(routine: Routine, runner: string): { value: string; note: string } {
  return routine.missed_run_policy === "skip"
    ? { value: t("Skip"), note: t("When {runner} was off at a scheduled time, the routine waits for the next one.", { runner }) }
    : { value: t("Run once"), note: t("When {runner} was off at a scheduled time, the routine runs once when it’s back.", { runner }) };
}

/// The line under the name in Details: what went wrong first, else the schedule and what is
/// going on.
export function routineDetail(routine: Routine): string {
  const schedule = scheduleText(routine.schedule_text);
  if (routine.is_running) return `${schedule} · ${t("Running…")}`;
  const problem = routineProblem(routine);
  if (problem) return `${problemWord(problem)} · ${schedule}`;
  if (!routine.is_enabled) return `${schedule} · ${routine.paused_reason === "away" ? t("Paused while you were away") : t("Paused")}`;
  if (routine.next_run_at) {
    const when = upcoming(routine.next_run_at);
    return `${schedule} · ${routine.check ? t("Next check {when}", { when }) : t("Next {when}", { when })}`;
  }
  return schedule;
}
