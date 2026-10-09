import { describe, expect, test } from "bun:test";
import type { Routine } from "../core/model";
import { lastCheckSummary, missedRuns, problemExplanation, problemNeedsUser, problemWord, routineDetail, routineProblem, scheduleSummary } from "./routines";

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

  test("the schedule names another timezone only, and an interval none", () => {
    const now = new Date(Date.UTC(2026, 9, 10, 12));
    const here = -now.getTimezoneOffset();
    // A zone 14 hours ahead of UTC is another zone wherever the test runs, except there.
    const away = here === 14 * 60 ? "Europe/London" : "Pacific/Kiritimati";
    expect(scheduleSummary(routine({ timezone: away }), now)).toBe(`Weekdays at 9:00 AM (${away === "Europe/London" ? "London" : "Kiritimati"} time)`);
    expect(scheduleSummary(routine({ timezone: "America/New_York", schedule: "every 2h", schedule_text: "Every 2 hours" }), now)).toBe("Every 2 hours");
    expect(scheduleSummary(routine({ timezone: "Mars/Base" }), now)).toBe("Weekdays at 9:00 AM");
  });

  test("the last check says how it went, and missed runs what they do", () => {
    expect(lastCheckSummary(routine({}))).toBeUndefined();
    expect(lastCheckSummary(routine({ check: "x", health: { last_check_at: Date.now() / 1000, status: "quiet" } }))).toEndWith(" · nothing new");
    expect(missedRuns(routine({}), "Workbench").value).toBe("Run once");
    expect(missedRuns(routine({ missed_run_policy: "skip" }), "Workbench").note).toBe("When Workbench was off at a scheduled time, the routine waits for the next one.");
  });
});
