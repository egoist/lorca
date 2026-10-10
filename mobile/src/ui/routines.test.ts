import { describe, expect, test } from "bun:test";
import type { Routine } from "../core/model";
import { aroundEvents, lastCheck, lastRun, missedRuns, nextRunText, onceText, problemExplanation, problemNeedsUser, problemWord, routineDetail, routineProblem, routineSchedule, routineSymbol, timezoneLabel } from "./routines";

// A routine as the core sends it: its timezone, missed-run policy, state, and check health.
const base: Routine = {
  id: "rt-1",
  bot_id: "b1",
  name: "Brief",
  prompt: "Read the inbox",
  schedule: "0 9 * * 1-5",
  schedule_text: "Weekdays at 9:00 AM",
  is_enabled: true,
  is_running: false,
  created_at: 0,
  state: "on",
};
const routine = (patch: Partial<Routine>): Routine => ({ ...base, ...patch });

describe("routines", () => {
  test("each state says what went wrong, and whether the user has to act", () => {
    const signedOut = routine({ is_enabled: false, paused_reason: "authentication", state: "blocked", check: "x", health: { status: "blocked", authentication_failures: 3 } });
    expect(routineProblem(signedOut)).toEqual({ kind: "signedOut", model: false });
    expect(routineDetail(signedOut)).toBe("Needs sign-in · Weekdays at 9:00 AM");
    expect(problemExplanation(routineProblem(signedOut)!, "Scout", "Workbench")).toContain("on Workbench, then resume it");
    const model = routine({ is_enabled: false, paused_reason: "authentication", state: "blocked", health: { model: { status: "blocked", authentication_failures: 3 } } });
    expect(routineProblem(model)).toEqual({ kind: "signedOut", model: true });
    expect(routineProblem(routine({ state: "waiting_for_runner" }))).toEqual({ kind: "offline" });
    const connection = routineProblem(routine({ state: "failed", check: "x", health: { status: "failed", connection_failures: 2 } }))!;
    expect(connection).toEqual({ kind: "cantConnect", model: false });
    expect(problemNeedsUser(connection)).toBe(false);
    expect(problemWord(connection)).toBe("Can’t connect");
    expect(routineProblem(routine({ state: "failed", check: "x", health: { status: "failed" } }))).toEqual({ kind: "checkFailed" });
    expect(routineProblem(routine({ state: "blocked", check: "x", health: { status: "blocked" } }))).toEqual({ kind: "checkBlocked" });
    expect(routineProblem(routine({ state: "failed", is_running: true }))).toBeUndefined();
    expect(routineProblem(routine({}))).toBeUndefined();
  });

  test("a routine names another timezone only, and an interval none", () => {
    const now = new Date(Date.UTC(2026, 9, 10, 12));
    const here = -now.getTimezoneOffset();
    // A zone 14 hours ahead of UTC is another zone wherever the test runs, except there.
    const away = here === 14 * 60 ? "Europe/London" : "Pacific/Kiritimati";
    expect(timezoneLabel(routine({ timezone: away }), now)).toBe(`${away === "Europe/London" ? "London" : "Kiritimati"} time`);
    expect(timezoneLabel(routine({ timezone: "America/New_York", schedule: "every 2h", schedule_text: "Every 2 hours" }), now)).toBeUndefined();
    expect(timezoneLabel(routine({ timezone: "Mars/Base" }), now)).toBeUndefined();
    expect(timezoneLabel(routine({ timezone: Intl.DateTimeFormat().resolvedOptions().timeZone }), now)).toBeUndefined();
  });

  test("the last check and run say how they went, and missed runs what they do", () => {
    expect(lastCheck(routine({}))).toBeUndefined();
    expect(lastCheck(routine({ check: "x", health: { last_check_at: Date.now() / 1000, status: "quiet" } }))?.outcome).toBe("Nothing new");
    expect(lastRun(routine({}))).toEqual({ when: "Never" });
    expect(lastRun(routine({ last_run_at: Date.now() / 1000, last_outcome: "pass" })).outcome).toBe("Nothing to report");
    expect(missedRuns(routine({}), "Workbench").value).toBe("Run once");
    expect(missedRuns(routine({ missed_run_policy: "skip" }), "Workbench").note).toBe("When Workbench was off at a scheduled time, the routine waits for the next one.");
  });

  test("one-time routines, watches, and routines around events say what places their runs", () => {
    // 2030-10-12 09:00 in Singapore, which keeps no daylight saving.
    const once = routine({ schedule: "once 2030-10-12 09:00", schedule_text: "Once on 2030-10-12 at 9:00 AM", timezone: "Asia/Singapore", once_at: Date.UTC(2030, 9, 12, 1) / 1000 });
    expect(onceText(once.once_at!, "Asia/Singapore")).toBe("Once on Oct 12, 2030 at 9:00 AM");
    expect(routineSchedule(once)).toBe("Once on Oct 12, 2030 at 9:00 AM");
    expect(routineSymbol(once)).toBe("alarm");
    const watch = routine({ schedule: "every 10m", schedule_text: "Watches acme/project#42", timezone: "Pacific/Kiritimati", pull_request: { repo: "acme/project", number: 42, title: "Add login", url: "https://github.com/acme/project/pull/42" }, next_run_at: Date.now() / 1000 + 600 });
    expect(routineSchedule(watch)).toBe("Watches acme/project#42");
    expect(routineDetail(watch)).toMatch(/^Watches acme\/project#42 · Next check today /);
    expect(timezoneLabel(watch)).toBeUndefined();
    expect(routineSymbol(watch)).toBe("arrow.triangle.pull");
    expect(routineProblem(routine({ ...watch, state: "failed", health: { status: "failed" } }))).toEqual({ kind: "readFailed", calendar: false });
    expect(aroundEvents(15, false, null)).toBe("15 minutes before each event");
    expect(aroundEvents(60, false, "Customer")).toBe("1 hour before events matching “Customer”");
    expect(aroundEvents(0, true, null)).toBe("When each event ends");
    expect(aroundEvents(10, true, "Customer")).toBe("10 minutes after events matching “Customer” end");
    const events = routine({ schedule: "15m before events", schedule_text: "15 minutes before events matching “Customer”", calendar: { account: "Google Calendar · Work", matching: "Customer", minutes: 15, after: false } });
    expect(routineSchedule(events)).toBe("15 minutes before events matching “Customer”");
    expect(nextRunText(events)).toBe("None in the next day");
    expect(routineSymbol(events)).toBe("calendar");
    const failing = routineProblem(routine({ ...events, state: "blocked", health: { status: "blocked" } }))!;
    expect(problemExplanation(failing, "Scout", "Workbench")).toContain("signed in on Workbench");
  });
});
