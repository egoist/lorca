import { beforeEach, expect, mock, test } from "bun:test";
import type { Notification, NotificationBehavior, NotificationResponse } from "expo-notifications";
import type { Chat, Message } from "./model";

// Exercise the real engine and store, including message/roster ordering, foreground
// transitions, and the working row. Only native bridges are replaced; no account or provider is
// contacted.
type Listener = (frame: { event: string; data: unknown }) => void;
const listeners = new Set<Listener>();
const event: Listener = (frame) => listeners.forEach((listener) => listener(frame));
/// Set to keep the next `bootstrap` answer until the test gives it.
let heldSnapshot: Promise<unknown> | null = null;
let appState: (status: string) => void;
const reads: string[] = [];
const cleared: string[] = [];
const opened: string[] = [];
let handleNotification: (notification: Notification) => Promise<NotificationBehavior>;
let openNotification: (response: NotificationResponse) => void;
mock.module("react-native", () => ({
  Platform: { OS: "ios" },
  AppState: { currentState: "active", addEventListener: (_: string, listener: typeof appState) => { appState = listener; } },
}));
mock.module("expo-web-browser", () => ({}));
mock.module("./host", () => ({ hostFacts: () => ({ name: "Phone", os: "ios", os_version: "", model: "" }) }));
mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("expo-application", () => ({}));
mock.module("expo-device", () => ({ isDevice: true }));
mock.module("expo-router", () => ({ router: { navigate: ({ params }: { params: { id: string } }) => { opened.push(params.id); } } }));
mock.module("expo-notifications", () => ({
  setNotificationHandler: (handler: { handleNotification: typeof handleNotification }) => { handleNotification = handler.handleNotification; },
  addNotificationResponseReceivedListener: (listener: typeof openNotification) => { openNotification = listener; },
  getLastNotificationResponse: () => null,
  getPermissionsAsync: async () => ({ status: "denied" }),
  getPresentedNotificationsAsync: async () => [notification("open", "Confirmation needed: Deploy the app"), notification("other", "Reply failed: Provider unavailable")],
  dismissNotificationAsync: async (id: string) => { cleared.push(id); },
}));
mock.module("../../modules/lorca-core", () => ({
  start: () => {}, wake: () => {},
  onEvent: (listener: Listener) => { listeners.add(listener); return () => listeners.delete(listener); },
  request: async (method: string, params: { chat_id?: string } = {}) => {
    if (method === "chats.mark_read") reads.push(params.chat_id!);
    return method === "bootstrap" ? (heldSnapshot ?? snapshot([])) : null;
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
const { workingActivity } = await import("../ui/format");
await engine.start();

beforeEach(async () => {
  resetStore();
  appState("active");
  event({ event: "snapshot", data: snapshot([chat("open"), chat("other")]) });
  useStore.setState({ openChatId: "open" });
  await flush();
  reads.length = 0;
  cleared.length = 0;
  opened.length = 0;
});

async function flush() {
  // markRead imports the native bridge asynchronously.
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function reply(id: string, kind: "reply" | "permission" | "failure" = "reply") {
  const message: Message = { id: "reply", chat_id: id, author: { kind: "bot", bot_id: "bot" },
    body: { kind: "text", text: "Done" }, state: { kind: "complete" }, created_at: 1 };
  if (kind === "permission") {
    message.body = { kind: "permission", plugin_id: "computer", plugin_name: "Mac", tool: "bash", summary: "Deploy the app", decision: "pending" };
  } else if (kind === "failure") {
    message.body = { kind: "text", text: "Partial response" };
    message.state = { kind: "failed", error: "Provider unavailable" };
  }
  event({ event: "message.updated", data: { chat_id: id, message } });
  event({ event: "roster.changed", data: { devices: [], bots: [], chats: [chat("open", id === "open" ? 1 : 0), chat("other", id === "other" ? 1 : 0)] } });
}

function notification(chatId: string, body: string): Notification {
  return { date: 1, request: { identifier: chatId, trigger: null, content: {
    title: "Chef", subtitle: null, body, data: { chat_id: chatId }, sound: "default",
  } } };
}

for (const kind of ["permission", "failure"] as const) {
  test(`a background ${kind} stays unread until the user returns`, async () => {
    appState("background");
    reply("open", kind);
    await flush();
    expect(reads).toEqual([]);
    expect(useStore.getState().chats[0].unread_count).toBe(1);
    appState("active");
    await flush();
    expect(reads).toEqual(["open"]);
    expect(cleared).toEqual(["open"]);
  });

  test(`a visible ${kind} is read after its unread count arrives`, async () => {
    reply("open", kind);
    await flush();
    expect(reads).toEqual(["open"]);
    expect(useStore.getState().chats[0].unread_count).toBe(0);
  });

  test(`${kind} pushes show away from the chat and open it when tapped`, async () => {
    const body = kind === "permission" ? "Confirmation needed: Deploy the app" : "Reply failed: Provider unavailable";
    const alert = notification("open", body);
    expect(await handleNotification(alert)).toMatchObject({ shouldShowBanner: false, shouldShowList: false, shouldPlaySound: false });
    appState("background");
    expect(await handleNotification(alert)).toMatchObject({ shouldShowBanner: true, shouldShowList: true, shouldPlaySound: true });
    appState("active");
    useStore.setState({ openChatId: "other" });
    expect(await handleNotification(alert)).toMatchObject({ shouldShowBanner: true, shouldShowList: true, shouldPlaySound: true });
    openNotification({ actionIdentifier: "expo.modules.notifications.actions.DEFAULT", notification: alert });
    expect(opened).toEqual(["open"]);
    expect(alert.request.content.body).toBe(body);
  });
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

test("a turn on the Runner reads as thinking, its command, and its retry", () => {
  const row = () => workingActivity(useStore.getState(), "open");
  const command = (is_running: boolean): Message => ({ id: "call", chat_id: "open", author: { kind: "bot", bot_id: "bot" },
    body: { kind: "tool", name: "bash", summary: is_running ? "Running bash…" : "installed", detail: "", is_running, description: "Install dependencies" },
    state: { kind: is_running ? "streaming" : "complete" }, created_at: 1 });
  event({ event: "job.started", data: { job_id: "job", chat_id: "open", bot_id: "bot" } });
  expect(row()).toBeNull();
  event({ event: "job.thinking", data: { chat_id: "open", bot_id: "bot" } });
  expect(row()).toBe("Thinking…");
  // The bot's next message is what its thinking came to.
  event({ event: "message.added", data: { chat_id: "open", message: command(true) } });
  expect(row()).toBe("Running command: Install dependencies…");
  event({ event: "job.thinking", data: { chat_id: "open", bot_id: "bot" } });
  event({ event: "message.updated", data: { chat_id: "open", message: command(false) } });
  expect(row()).toBe("Thinking…");
  event({ event: "job.retry", data: { chat_id: "open", bot_id: "bot", attempt: 1, max_attempts: 3, delay_ms: 2000, error: "overloaded" } });
  expect(row()).toBe("Retrying (1 of 3) in 2 s…");
  event({ event: "job.finished", data: { job_id: "job", chat_id: "open", bot_id: "bot" } });
  expect(useStore.getState()).toMatchObject({ running: {}, thinking: {}, retries: {} });
});

test("events that arrive while a snapshot is on its way are applied after it", async () => {
  // The relay connects and a reply lands while the core is still building the snapshot the app
  // asked for, so the snapshot is older than both.
  let answer!: (snapshot: unknown) => void;
  heldSnapshot = new Promise((resolve) => { answer = resolve; });
  const paired = engine.pair("lorca://pair");
  await flush();
  event({ event: "relay.status", data: { connected: true, update_required: false, url: "https://relay.example" } });
  reply("open");
  answer({ ...snapshot([chat("open"), chat("other")]), relay_connected: false });
  heldSnapshot = null;
  await paired;
  expect(useStore.getState().relayConnected).toBe(true);
  expect(useStore.getState().chats[0].messages.map((m) => m.id)).toEqual(["reply"]);
});
