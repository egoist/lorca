import { expect, mock, test } from "bun:test";
import type { ReviewItem } from "./model";

mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
const { acceptReview, useStore } = await import("./store");
const { reviewIsOpen } = await import("./model");

const item = (revision: number, extra: Partial<ReviewItem> = {}): ReviewItem => ({
  id: "review-1", runner_id: "mac", bot_id: "bot", origin: { chat_id: "chat" }, target: { account: "Workbench", resource: "~/relay" },
  rationale: "Pushes a tag", payload: { kind: "shell", arguments: { command: "git push --tags" } }, version: 1, revision,
  preconditions: { workdir: "~/relay", files: [] }, state: "pending", created_at: 1, ...extra,
});

test("a review reply or event older than what the phone has changes nothing", () => {
  useStore.setState({ reviews: [] });
  acceptReview(item(3, { state: "approved" }));
  acceptReview(item(2));
  expect(useStore.getState().reviews).toEqual([item(3, { state: "approved" })]);
  acceptReview(item(4, { state: "succeeded" }));
  expect(useStore.getState().reviews[0].state).toBe("succeeded");
});

test("only what waits on the user or runs is open", () => {
  expect(reviewIsOpen(item(1))).toBe(true);
  expect(reviewIsOpen(item(1, { state: "executing" }))).toBe(true);
  for (const state of ["succeeded", "failed", "rejected", "cancelled", "uncertain"] as const) expect(reviewIsOpen(item(1, { state }))).toBe(false);
});
