import { expect, mock, test } from "bun:test";
import type { DurableTask } from "./model";

mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
const { acceptDurableTask, useStore } = await import("./store");
const { CANCELLED_BY_USER, NOTHING_TO_DO, taskCanStart, taskFoundNothingToDo, taskGlyph, taskOrder } = await import("./model");

const task = (revision: number, extra: Partial<DurableTask> = {}): DurableTask => ({
  id: "task-1", revision, authority_runner_id: "mac", owner_bot_id: "bot", runner_id: "mac", goal: "Fix the link",
  acceptance_criteria: ["Returns 200"], dependencies: [], next_action: "Check it", chat_ids: ["chat"], links: [],
  state: "queued", evidence: [], created_at: 1, updated_at: revision, ...extra,
});

test("a task reply or event older than what the phone has changes nothing", () => {
  useStore.setState({ tasks: [] });
  acceptDurableTask(task(3, { goal: "Newer" }));
  acceptDurableTask(task(2, { goal: "Older" }));
  expect(useStore.getState().tasks).toEqual([task(3, { goal: "Newer" })]);
  acceptDurableTask(task(4, { state: "blocked", reason: "No key" }));
  expect(useStore.getState().tasks[0].state).toBe("blocked");
});

test("only a queued or blocked task with no run starts, and what waits on the user comes first", () => {
  expect(taskCanStart(task(1))).toBe(true);
  expect(taskCanStart(task(1, { state: "blocked" }))).toBe(true);
  expect(taskCanStart(task(1, { active_run: { id: "run", bot_id: "bot", runner_id: "mac", chat_id: "chat", started_at: 1 } }))).toBe(false);
  expect(taskCanStart(task(1, { state: "awaiting_review" }))).toBe(false);
  expect(taskOrder("blocked")).toBeLessThan(taskOrder("awaiting_review"));
  expect(taskOrder("awaiting_review")).toBeLessThan(taskOrder("queued"));
  expect(taskOrder("queued")).toBeLessThan(taskOrder("completed"));
});

test("a cancelled task with nothing to do doesn't read as a cancellation", () => {
  const nothing = task(1, { state: "cancelled", reason: NOTHING_TO_DO });
  expect(NOTHING_TO_DO).toBe("Nothing to do for this request.");
  expect(taskFoundNothingToDo(nothing)).toBe(true);
  expect(taskGlyph(nothing)).toBe("checkmark.circle");
  expect(taskFoundNothingToDo(task(1, { state: "cancelled", reason: CANCELLED_BY_USER }))).toBe(false);
  expect(taskGlyph(task(1, { state: "cancelled" }))).toBe("xmark.circle");
  expect(taskFoundNothingToDo(task(1, { state: "queued", reason: NOTHING_TO_DO }))).toBe(false);
});
