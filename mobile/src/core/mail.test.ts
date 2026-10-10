import { beforeEach, describe, expect, mock, test } from "bun:test";
import type { MailStatus } from "./mail";

// The account's email address through the real engine and store: the snapshot's `mail`, the
// `mail.changed` event, and the three requests. Only native bridges are replaced.
type Listener = (frame: { event: string; data: unknown }) => void;
const listeners = new Set<Listener>();
const event: Listener = (frame) => listeners.forEach((listener) => listener(frame));
const calls: { method: string; params: Record<string, any> }[] = [];

const off: MailStatus = { available: true, domain: "bots.lorca.app", address: null, lead_bot_id: "bot-chef", bots: [] };
const taken = (name: string): MailStatus => ({
  ...off,
  address: { name, email: `${name}@bots.lorca.app`, state: "active" },
  bots: [
    { bot_id: "bot-chef", email: `${name}+chef@bots.lorca.app` },
    { bot_id: "bot-scout", email: `${name}+scout@bots.lorca.app` },
  ],
});

mock.module("react-native", () => ({ Platform: { OS: "ios" }, AppState: { currentState: "active", addEventListener: () => {} } }));
mock.module("expo-web-browser", () => ({}));
mock.module("./host", () => ({ hostFacts: () => ({ name: "Phone", os: "ios", os_version: "", model: "" }) }));
mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("./push", () => ({ clearPushes: async () => {}, installPushHandlers: () => {}, registerForPushes: async () => {} }));
mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
mock.module("../../modules/lorca-core", () => ({
  start: () => {},
  wake: () => {},
  onEvent: (listener: Listener) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  request: async (method: string, params: Record<string, any> = {}) => {
    if (method.startsWith("mail.")) calls.push({ method, params });
    if (method === "mail.get") return taken("k7f3m9q2");
    if (method === "mail.apply") return params.name === "postmaster" ? { problem: "reserved" } : { mail: taken(params.name ?? "x4p9t2wq") };
    if (method === "mail.release") return { mail: off };
    if (method === "bootstrap") return snapshot();
    return null;
  },
}));

function snapshot(mail?: MailStatus | null) {
  return { has_identity: true, identity_id: "account", this_device_id: "phone", relay_url: null, relay_connected: true, devices: [], bots: [], chats: [], running_turns: [], ...(mail !== undefined ? { mail } : {}) };
}

const { engine } = await import("./engine");
const { replaceSnapshot, resetStore, useStore } = await import("./store");
const { mailNameIsValid, mailProblemText, mailRoutingNote, normalizeMailName } = await import("./mail");
await engine.start();

beforeEach(() => {
  resetStore();
  calls.length = 0;
});

describe("mail", () => {
  test("a name follows the relay's rule", () => {
    for (const name of ["egoist", "k7f3m9q2", "a.b-c", "abc", "a".repeat(32)]) expect(mailNameIsValid(name)).toBe(true);
    for (const name of ["ab", "a".repeat(33), "-abc", "abc.", ".abc", "a..b", "Abc", "a b", "a+b", "名字", "a_b"]) expect(mailNameIsValid(name)).toBe(false);
    expect(normalizeMailName("  Egoist ")).toBe("egoist");
    expect(mailProblemText("taken")).toBe("That name is taken.");
    expect(mailProblemText("reserved")).toBe("That name is reserved.");
    expect(mailProblemText("invalid")).toBe("Use letters, digits, dots, and hyphens.");
  });

  test("the footer names a bot's own address and the lead bot that takes the rest", () => {
    const bots = [{ id: "bot-chef", name: "Chef" }, { id: "bot-scout", name: "Scout" }];
    expect(mailRoutingNote(taken("k7f3m9q2"), bots)).toBe("Mail to k7f3m9q2+scout@bots.lorca.app goes to Scout, and other mail to Chef.");
    const alone = { ...taken("k7f3m9q2"), bots: [{ bot_id: "bot-chef", email: "k7f3m9q2+chef@bots.lorca.app" }] };
    expect(mailRoutingNote(alone, bots.slice(0, 1))).toBe("Mail to this address goes to Chef.");
    expect(mailRoutingNote({ ...off, lead_bot_id: null }, [])).toBeUndefined();
  });

  test("the snapshot carries the address, and says nothing until the core has asked", () => {
    replaceSnapshot(snapshot(taken("k7f3m9q2")) as any);
    expect(useStore.getState().mail?.address?.email).toBe("k7f3m9q2@bots.lorca.app");
    replaceSnapshot(snapshot() as any);
    expect(useStore.getState().mail).toBeNull();
  });

  test("mail.changed replaces the address", () => {
    event({ event: "mail.changed", data: taken("egoist") });
    expect(useStore.getState().mail?.address?.name).toBe("egoist");
    event({ event: "mail.changed", data: off });
    expect(useStore.getState().mail?.address).toBeNull();
  });

  test("applying asks for a random name or the user's own, and a refusal changes nothing", async () => {
    event({ event: "mail.changed", data: off });
    expect(await engine.applyMail()).toBeUndefined();
    expect(calls.at(-1)).toEqual({ method: "mail.apply", params: {} });
    expect(useStore.getState().mail?.address?.name).toBe("x4p9t2wq");

    expect(await engine.applyMail("postmaster")).toBe("reserved");
    expect(useStore.getState().mail?.address?.name).toBe("x4p9t2wq");

    expect(await engine.applyMail("egoist")).toBeUndefined();
    expect(calls.at(-1)).toEqual({ method: "mail.apply", params: { name: "egoist" } });
    expect(useStore.getState().mail?.address?.email).toBe("egoist@bots.lorca.app");
  });

  test("giving up and refreshing take the core's answer", async () => {
    await engine.refreshMail();
    expect(useStore.getState().mail?.address?.name).toBe("k7f3m9q2");
    await engine.releaseMail();
    expect(calls.map((call) => call.method)).toEqual(["mail.get", "mail.release"]);
    expect(useStore.getState().mail).toEqual(off);
  });
});
