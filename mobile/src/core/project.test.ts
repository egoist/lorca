import { expect, mock, test } from "bun:test";
import type { ProjectEntry } from "./model";

mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
const { canCheckLink, isSuggestion, linkChecked, orderProjectEntries, otherVersions, projectHost } = await import("./model");
const { removeChat, useStore } = await import("./store");

function entry(id: string, kind: ProjectEntry["kind"], updated: number, extra: Partial<ProjectEntry> = {}): ProjectEntry {
  return { id, kind, title: id, text: "", source: { kind: "user", label: "User" }, verification: "agreed", freshness: "agreed", updated_at: updated, ...extra };
}

test("briefs come first and files last, newest first within a kind", () => {
  const ordered = orderProjectEntries([entry("fact", "fact", 30), entry("old", "decision", 10), entry("file", "asset", 50), entry("brief", "brief", 1), entry("new", "decision", 20)]);
  expect(ordered.map((e) => e.id)).toEqual(["brief", "new", "old", "fact", "file"]);
});

test("a bot's proposal is a suggestion, and an agreed decision's link is not checked", () => {
  const link = { kind: "url" as const, label: "docs.example.com", url: "https://www.example.com/plan" };
  expect(isSuggestion(entry("p", "fact", 1, { source: { kind: "bot", label: "Scout" }, verification: "unverified" }))).toBe(true);
  expect(isSuggestion(entry("u", "fact", 1, { verification: "unverified" }))).toBe(false);
  expect(canCheckLink(entry("d", "decision", 1, { source: link }))).toBe(false);
  const fetched = entry("l", "document", 1, { source: link, verification: "fetched", fetched_at: 5, verified_at: 3 });
  expect(canCheckLink(fetched)).toBe(true);
  expect(linkChecked(fetched)).toBe(5);
  expect(projectHost(fetched)).toBe("example.com");
});

test("two versions of one entry name each other, and a deleted group takes its context along", () => {
  const context = { entries: [entry("a", "brief", 1), entry("b", "brief", 2)], conflicts: [["a", "b"]] };
  expect(otherVersions(context, "a")).toEqual(["b"]);
  expect(otherVersions(context, "c")).toEqual([]);
  useStore.setState({ projects: { group: context } });
  removeChat("group");
  expect(useStore.getState().projects.group).toBeUndefined();
});
