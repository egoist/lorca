import { beforeEach, expect, mock, test } from "bun:test";
import type { Chat, Message } from "./model";

// Exercise the real engine and store, including message/roster ordering and foreground
// transitions. Only native bridges are replaced; no account or provider is contacted.
let event: (frame: { event: string; data: unknown }) => void;
let appState: (status: string) => void;
const reads: string[] = [];
const cleared: string[] = [];
mock.module("react-native", () => ({
  Platform: { OS: "ios" },
  AppState: { currentState: "active", addEventListener: (_: string, listener: typeof appState) => { appState = listener; } },
}));
mock.module("expo-web-browser", () => ({}));
mock.module("./host", () => ({ hostFacts: () => ({ name: "Phone", os: "ios", os_version: "", model: "" }) }));
mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("./push", () => ({
  installPushHandlers: () => {}, registerForPushes: async () => {}, clearPushes: async (id: string) => { cleared.push(id); },
}));
mock.module("../../modules/lorca-core", () => ({
  start: () => {}, wake: () => {},
  onEvent: (listener: typeof event) => { event = listener; return () => {}; },
  request: async (method: string, params: { chat_id?: string } = {}) => {
    if (method === "chats.mark_read") reads.push(params.chat_id!);
    return method === "bootstrap" ? snapshot([]) : null;
  },
}));

function chat(id: string, unread_count = 0): Chat {
  return { id, kind: "dm", bot_ids: ["bot"], is_pinned: false, created_at: 0, messages: [], unread_count };
}

function snapshot(chats: Chat[]) {
  return { has_identity: true, identity_id: "account", this_device_id: "phone", relay_url: null,
    relay_connected: true, devices: [], bots: [], chats, running_turns: [] };
}

const { engine } = await import("./engine");
const { resetStore, useStore } = await import("./store");
await engine.start();

beforeEach(() => {
  resetStore();
  appState("active");
  event({ event: "snapshot", data: snapshot([chat("open"), chat("other")]) });
  useStore.setState({ openChatId: "open" });
  reads.length = 0;
  cleared.length = 0;
});

async function flush() {
  // markRead imports the native bridge asynchronously.
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function reply(id: string) {
  const message: Message = { id: "reply", chat_id: id, author: { kind: "bot", bot_id: "bot" },
    body: { kind: "text", text: "Done" }, state: { kind: "complete" }, created_at: 1 };
  event({ event: "message.updated", data: { chat_id: id, message } });
  event({ event: "roster.changed", data: { devices: [], bots: [], chats: [chat("open", id === "open" ? 1 : 0), chat("other", id === "other" ? 1 : 0)] } });
}

test("a visible reply is acknowledged after its unread count arrives", async () => {
  reply("open");
  await flush();
  expect(reads).toEqual(["open"]);
  expect(useStore.getState().chats[0].unread_count).toBe(0);
});

test("a mounted chat cannot mark a background reply read; returning reads it", async () => {
  appState("background");
  reply("open");
  await flush();
  expect(reads).toEqual([]);
  expect(useStore.getState().chats[0].unread_count).toBe(1);
  appState("active");
  await flush();
  expect(reads).toEqual(["open"]);
  expect(cleared).toEqual(["open"]);
});

test("inactive or unfocused UI leaves replies unread", async () => {
  appState("inactive");
  reply("open");
  await flush();
  expect(reads).toEqual([]);
  useStore.setState({ openChatId: null });
  appState("active");
  await flush();
  expect(reads).toEqual([]);
});

test("another chat still notifies while this chat is visible", async () => {
  reply("other");
  await flush();
  expect(reads).toEqual([]);
  expect(useStore.getState().chats[1].unread_count).toBe(1);
});

test("a backlog snapshot is read only while the chat is foregrounded", async () => {
  appState("background");
  event({ event: "snapshot", data: snapshot([chat("open", 2)]) });
  await flush();
  expect(reads).toEqual([]);
  appState("active");
  await flush();
  expect(reads).toEqual(["open"]);
});
