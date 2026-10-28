import { describe, expect, mock, test } from "bun:test";

mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
const { applyRoster, replaceSnapshot, resetStore, useStore } = await import("./store");
const { redirectSystemPath } = await import("../../app/+native-intent");
const templates = await import("./templates");
const {
  allPicked,
  connectionTitle,
  flagWords,
  importNote,
  initialSelection,
  linkTitle,
  memoryLines,
  newestLinks,
  routineLine,
  sharedLinkFor,
  templateAddress,
  templateOrder,
  toggleAll,
  toggleItem,
} = templates;
type SharedLink = import("./templates").SharedLink;
type TemplateContents = import("./templates").TemplateContents;
type TemplateImportPreview = import("./templates").TemplateImportPreview;

const KEY = "-xMo1zlzI63iQ_tY2ATkxlkle7thq-OfylCF-jFDqYw";
const item = <T,>(id: string, content: T, flags: string[] = []) => ({ id, content, flags });
const contents: TemplateContents = {
  profile: item("profile", { name: "Scout", description: "Finds sources", symbol_name: "sparkles", accent: "indigo" }),
  skills: [item("s1", { name: "review", description: "Review code" })],
  memories: [item("m1", "- Likes tea"), item("m2", "## Work\nShips on Fridays", ["email"])],
  routines: [item("r1", { name: "Morning", schedule: "0 9 * * *", prompt: "Read the inbox" })],
  requirements: [{ service_id: "github", name: "GitHub" }],
};
const link = (id: string, bot: string, updated: number, extra: Partial<SharedLink> = {}): SharedLink => ({
  id,
  url: `https://lorca.app/t/${id}#${KEY}`,
  bot_id: bot,
  name: "Scout",
  selection: { profile: true, skill_ids: [], memory_ids: [], routine_ids: [], requirement_ids: [] },
  created_at: 1,
  updated_at: updated,
  ...extra,
});

describe("templates", () => {
  test("a link reads as the page's address whichever way it comes, and only once it is whole", () => {
    const page = `https://lorca.app/t/yWUtxVvEA7X0edjxRUKd2A#${KEY}`;
    expect(templateAddress(page)).toBe(page);
    expect(templateAddress(`  ${page}\n`)).toBe(page);
    expect(templateAddress(`lorca://t/yWUtxVvEA7X0edjxRUKd2A#${KEY}`)).toBe(page);
    expect(templateAddress(`lorca-dev://t/abc?relay=http%3A%2F%2F192.168.1.4%3A8787#${KEY}`)).toBe(`https://lorca.app/t/abc?relay=http%3A%2F%2F192.168.1.4%3A8787#${KEY}`);
    for (const partial of ["https://lorca.app/t/abc", "https://lorca.app/t/abc#", "lorca://t/abc", "https://lorca.app/docs#x", "lorca://pair?relay=x&id=y#z", "hello"]) {
      expect(templateAddress(partial)).toBeUndefined();
    }
    expect(linkTitle(`https://lorca.app/t/abc?relay=x#${KEY}`)).toBe("lorca.app/t/abc");
  });

  test("Open in Lorca waits whole in the store for an account; other links go to their routes", () => {
    resetStore();
    expect(redirectSystemPath({ path: `lorca-dev://t/abc#${KEY}`, initial: true })).toBeNull();
    expect(useStore.getState().pendingTemplateLink).toBe(`https://lorca.app/t/abc#${KEY}`);
    // Unpairing forgets the account, not a link the app was opened with.
    resetStore();
    expect(useStore.getState().pendingTemplateLink).toBe(`https://lorca.app/t/abc#${KEY}`);
    expect(redirectSystemPath({ path: "lorca://pair?relay=x&id=y&ek=z&n=1", initial: false })).toBe("lorca://pair?relay=x&id=y&ek=z&n=1");
    // A link cut before its key opens nothing rather than a route that doesn't exist.
    expect(redirectSystemPath({ path: "lorca://t/abc", initial: false })).toBe("lorca://t/abc");
    useStore.setState({ pendingTemplateLink: null });
  });

  test("the account's links come with the snapshot and the roster", () => {
    const first = link("a", "bot", 5);
    replaceSnapshot({ has_identity: true, identity_id: "me", this_device_id: "phone", relay_url: null, relay_connected: false, devices: [], bots: [], chats: [], running_turns: [], shared_links: [first] });
    expect(useStore.getState().shared_links).toEqual([first]);
    applyRoster({ devices: [], bots: [], chats: [] });
    expect(useStore.getState().shared_links).toEqual([first]);
    applyRoster({ devices: [], bots: [], chats: [], shared_links: [] });
    expect(useStore.getState().shared_links).toEqual([]);
  });

  test("a bot's link is its newest, and Settings lists the newest change first", () => {
    const links = [link("a", "scout", 5), link("b", "chef", 9), link("c", "scout", 7)];
    expect(sharedLinkFor(links, "scout")?.id).toBe("c");
    expect(sharedLinkFor(links, "nobody")).toBeUndefined();
    expect(newestLinks(links).map((each) => each.id)).toEqual(["b", "c", "a"]);
  });

  test("a sheet starts with the profile, or with what the link holds that the bot still has", () => {
    expect(initialSelection(contents)).toEqual({ profile: true, skill_ids: [], memory_ids: [], routine_ids: [], requirement_ids: [] });
    const shared = link("a", "scout", 1, { selection: { profile: true, skill_ids: ["gone", "s1"], memory_ids: ["m2"], routine_ids: [], requirement_ids: ["github", "slack"] } });
    expect(initialSelection(contents, shared)).toEqual({ profile: true, skill_ids: ["s1"], memory_ids: ["m2"], routine_ids: [], requirement_ids: ["github"] });
  });

  test("picks keep the bot's order, and Select All picks or drops a whole list", () => {
    const order = templateOrder(contents).memory_ids;
    let selection = initialSelection(contents);
    selection = toggleItem(selection, "memory_ids", "m2", order);
    selection = toggleItem(selection, "memory_ids", "m1", order);
    expect(selection.memory_ids).toEqual(["m1", "m2"]);
    expect(allPicked(selection, "memory_ids", order)).toBe(true);
    expect(toggleItem(selection, "memory_ids", "m1", order).memory_ids).toEqual(["m2"]);
    expect(toggleAll(selection, "memory_ids", order).memory_ids).toEqual([]);
    expect(toggleAll(toggleItem(selection, "memory_ids", "m1", order), "memory_ids", order).memory_ids).toEqual(["m1", "m2"]);
    expect(allPicked(selection, "skill_ids", [])).toBe(false);
  });

  test("rows read as the desktop apps word them", () => {
    expect(memoryLines("## Work\n- Ships on Fridays\n\n1. Reviews on Mondays")).toEqual({ title: "Work", detail: "Ships on Fridays Reviews on Mondays" });
    expect(routineLine({ name: "Morning", schedule: "0 9 * * *", prompt: "Read the inbox", schedule_text: "Every day at 9:00" })).toBe("Every day at 9:00 · Read the inbox");
    expect(routineLine({ name: "Morning", schedule: "0 9 * * *", prompt: "" })).toBe("0 9 * * *");
    expect(flagWords([])).toBeUndefined();
    expect(flagWords(["credential"])).toEqual({ text: "Key removed", personal: false });
    expect(flagWords(["email", "phone", "path"])).toEqual({ text: "Email address, Phone number", personal: true });
    expect(connectionTitle({ id: "a", name: "Work", state: "needs_auth" })).toBe("Work (needs setup)");
    expect(connectionTitle({ id: "a", name: "Work", state: "ready" })).toBe("Work");
  });

  test("the import says what is missing before it says that routines start paused", () => {
    const preview = (extra: Partial<TemplateImportPreview>): TemplateImportPreview => ({ digest: "d", can_import: true, issues: [], requirements: [], template: { routines: [{ name: "Morning", schedule: "0 9 * * *", prompt: "" }] }, ...extra });
    const github = (candidates: { id: string; name: string; state: string }[], selected?: string) => ({ service_id: "github", name: "GitHub", candidates, selected });
    expect(importNote(preview({}), undefined)).toEqual({ text: "Pair a Runner first: a bot runs on a computer.", warning: true });
    expect(importNote(preview({ issues: ["Bad schedule."] }), "Studio")).toEqual({ text: "Bad schedule.", warning: true });
    expect(importNote(preview({ requirements: [github([])] }), "Studio")?.text).toBe("Add GitHub to Studio from the Marketplace first.");
    expect(importNote(preview({ requirements: [github([{ id: "a", name: "Work", state: "ready" }, { id: "b", name: "Home", state: "ready" }])] }), "Studio")?.text).toBe("Choose the GitHub connection this bot uses.");
    expect(importNote(preview({ requirements: [github([{ id: "a", name: "Work", state: "needs_auth" }], "a")] }), "Studio")?.text).toBe("Finish setting up GitHub on Studio first.");
    expect(importNote(preview({ requirements: [github([{ id: "a", name: "Work", state: "ready" }], "a")] }), "Studio")).toEqual({ text: "Routines start paused.", warning: false });
    expect(importNote(preview({ template: { routines: [] } }), "Studio")).toBeUndefined();
  });
});
