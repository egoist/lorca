import { expect, mock, test } from "bun:test";
import type { BrowserProfile } from "../core/model";

mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
mock.module("../core/prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
const { profileActions, profileStateWord } = await import("./browserProfiles");

const profile = (state: BrowserProfile["state"]): BrowserProfile => ({ id: "browser-1", name: "Work", state, revision: 3 });
const menu = (state: BrowserProfile["state"], busy = false) =>
  profileActions(profile(state), "Workbench", true, busy).map((action) => action.title + (action.disabled ? " (disabled)" : ""));

test("a profile's state reads as the Mac's", () => {
  expect(["stopped", "bot", "taking_over", "human"].map((state) => profileStateWord(profile(state as BrowserProfile["state"])))).toEqual(["Closed", "Open", "Taking over…", "You have control"]);
});

test("the phone never opens a window; it takes over, hands back, and closes", () => {
  expect(menu("stopped")).toEqual(["Open on Workbench (disabled)", "Delete…"]);
  expect(menu("bot")).toEqual(["Take Over", "Take Screenshot", "Close Browser", "Delete…"]);
  expect(menu("taking_over")).toEqual(["Close Browser", "Delete…"]);
  expect(menu("human")).toEqual(["Return to Bot", "Take Screenshot", "Close Browser", "Delete…"]);
  // While an action runs, Close Browser still works.
  expect(menu("human", true)).toEqual(["Return to Bot (disabled)", "Take Screenshot (disabled)", "Close Browser", "Delete… (disabled)"]);
  expect(profileActions(profile("bot"), "Workbench", false, false).some((action) => action.id === "screenshot")).toBe(false);
});
