// The parts of the phone's core that run outside React Native, run with `bun test`: the
// pairing-string check, the model helpers, and the formatting the transcript and list share.

import { describe, expect, test } from "bun:test";
import {
  attachmentSummary,
  fileSize,
  isProviderKind,
  isSentMessage,
  providerConnectMethod,
  providerDefaultBaseURL,
  providerUsesAPIKey,
  PROVIDER_MODELS,
  showsCard,
  type Body,
  type Bot,
  type Chat,
  type CommandRun,
} from "./model";
import { parsePairingString } from "./pairing";
import { daySeparator, joinDictation, preview, stamp, time, workingActivity, type WorkState } from "../ui/format";

let nextId = 0;
const uuid = () => `m-${++nextId}`;

describe("pairing", () => {
  test("parses a pairing string from another Device", () => {
    const target = parsePairingString(
      " lorca://pair?relay=http%3A%2F%2F127.0.0.1%3A18790%2F&id=Clup-vXLfBF6T2JkKpLqNOpyE9hdQbqXjrIWfdDvLbs&ek=nAanQrXTSxfK1tf7m3V2Fg-65OL84r6MeqmQmWw5uhw&n=1qUeEODuTz_A2y5jSZdBqQ \n",
    );
    expect(target).toEqual({
      relay: "http://127.0.0.1:18790",
      id: "Clup-vXLfBF6T2JkKpLqNOpyE9hdQbqXjrIWfdDvLbs",
      ek: "nAanQrXTSxfK1tf7m3V2Fg-65OL84r6MeqmQmWw5uhw",
      nonce: "1qUeEODuTz_A2y5jSZdBqQ",
    });
  });

  test("rejects other text", () => {
    expect(() => parsePairingString("hello")).toThrow("not a Lorca pairing string");
    expect(() => parsePairingString("lorca://pair?relay=x&id=y")).toThrow("missing a field");
  });
});

describe("model", () => {
  test("only a finished message_bot is a sent message", () => {
    const sent: Body = { kind: "tool", name: "message_bot", summary: "Messaged Scout", detail: "look into tea", is_running: false };
    const running: Body = { ...sent, is_running: true };
    const failed: Body = { ...sent, summary: "message_bot failed" };
    expect(isSentMessage(sent)).toBe(true);
    expect(isSentMessage(running)).toBe(false);
    expect(isSentMessage(failed)).toBe(false);
    expect(isSentMessage({ kind: "text", text: "Messaged Scout" })).toBe(false);
  });

  test("a command shows its card only while it needs the user", () => {
    const call = (is_running: boolean, state: CommandRun["state"]) =>
      showsCard({ kind: "tool", name: "bash", summary: "Running", detail: "", is_running, run: { command: "sudo pacman -Syu", state } });
    // Its question, whether or not the call still waits on it.
    expect(call(true, "asking")).toBe(true);
    // Inside its call: the working row says what runs.
    expect(call(true, "checking")).toBe(false);
    expect(call(true, "running")).toBe(false);
    expect(call(true, "waiting")).toBe(false);
    // Its call returned while it still runs: waiting for an answer, or going on by itself.
    expect(call(false, "waiting")).toBe(true);
    expect(call(false, "running")).toBe(true);
    // Ended.
    for (const state of ["exited", "failed", "stopped", "denied", "expired", "dismissed"] as const) expect(call(false, state)).toBe(false);
    expect(showsCard({ kind: "tool", name: "read", summary: "", detail: "", is_running: false })).toBe(false);
  });

  test("describes provider credential setup", () => {
    expect(isProviderKind("opencode-go")).toBe(true);
    expect(isProviderKind("unknown")).toBe(false);
    expect(providerUsesAPIKey("anthropic")).toBe(true);
    expect(providerUsesAPIKey("chatgpt")).toBe(false);
    expect(providerConnectMethod("opencode-go")).toBe("providers.connect_opencode_go");
    expect(providerConnectMethod("grok")).toBe("providers.connect_grok");
    expect(providerDefaultBaseURL("deepseek")).toBe("https://api.deepseek.com");
    expect(providerDefaultBaseURL("chatgpt")).toBe("");
    expect(PROVIDER_MODELS.chatgpt.map((m) => m.id)).toEqual(["gpt-6-sol", "gpt-6-astra", "gpt-6-luna"]);
  });
});

describe("attachments", () => {
  const photo = { id: "att-1", name: "IMG_1.jpg", mime: "image/jpeg", size: 2_500_000 };
  const doc = { id: "att-2", name: "report.pdf", mime: "application/pdf", size: 900 };

  test("summaries and sizes", () => {
    expect(attachmentSummary([])).toBe("");
    expect(attachmentSummary([photo])).toBe("Photo");
    expect(attachmentSummary([photo, photo])).toBe("2 photos");
    expect(attachmentSummary([doc])).toBe("report.pdf");
    expect(attachmentSummary([photo, doc])).toBe("2 files");
    expect(fileSize(900)).toBe("900 B");
    expect(fileSize(2_500_000)).toBe("2.4 MB");
  });

  test("dictation joins after typed text", () => {
    expect(joinDictation("", "hello there")).toBe("hello there");
    expect(joinDictation("Hi", "there")).toBe("Hi there");
    expect(joinDictation("Hi ", "there")).toBe("Hi there");
    expect(joinDictation("Hi", "")).toBe("Hi");
  });
});

describe("format", () => {
  const bots = new Map<string, Bot>([
    ["b1", { id: "b1", name: "Chef", description: "", symbol_name: "sparkles", accent: "indigo", runner_id: "r", provider: "deepseek", created_at: 0 }],
    ["b2", { id: "b2", name: "Scout", description: "", symbol_name: "magnifyingglass", accent: "teal", runner_id: "r", provider: "deepseek", created_at: 0 }],
  ]);
  const chat = (kind: "dm" | "group", messages: Chat["messages"]): Chat => ({ id: "c", kind, bot_ids: ["b1", "b2"], is_pinned: false, created_at: 0, messages, unread_count: 0 });
  const msg = (author: Chat["messages"][number]["author"], body: Body): Chat["messages"][number] => ({ id: uuid(), chat_id: "c", author, body, state: { kind: "complete" }, created_at: 1 });

  test("previews the way the desktop sidebar does", () => {
    expect(preview(chat("dm", []), bots)).toBe("No messages yet");
    expect(preview(chat("dm", [msg({ kind: "you" }, { kind: "text", text: "hi\nthere **now**" })]), bots)).toBe("hi there now");
    const photo = { id: "att-1", name: "IMG_1.jpg", mime: "image/jpeg", size: 1 };
    expect(preview(chat("dm", [msg({ kind: "you" }, { kind: "text", text: "", attachments: [photo] })]), bots)).toBe("Photo");
    expect(preview(chat("group", [msg({ kind: "bot", bot_id: "b2" }, { kind: "text", text: "yes" })]), bots)).toBe("Scout: yes");
    const tool = msg({ kind: "bot", bot_id: "b1" }, { kind: "tool", name: "read", summary: "Read a file", detail: "x", is_running: false });
    expect(preview(chat("dm", [msg({ kind: "you" }, { kind: "text", text: "go" }), tool]), bots)).toBe("go");
    const sent = msg({ kind: "bot", bot_id: "b1" }, { kind: "tool", name: "message_bot", summary: "Messaged Scout", detail: "please look", is_running: false, target_bot_id: "b2" });
    expect(preview(chat("dm", [sent]), bots)).toBe("Messaged Scout: please look");
    const handoff = msg({ kind: "bot", bot_id: "b1" }, { kind: "handoff", from: "b1", to: "b2", reason: "over to you" });
    expect(preview({ ...chat("dm", [handoff]), bot_ids: ["b2"] }, bots)).toBe("Message from Chef: over to you");
  });

  test("the working row reads what the one bot at work is doing", () => {
    const you = msg({ kind: "you" }, { kind: "text", text: "set it up" });
    const tool = (name: string, extra: Partial<Extract<Body, { kind: "tool" }>> = {}) =>
      msg({ kind: "bot", bot_id: "b1" }, { kind: "tool", name, summary: `Running ${name}…`, detail: "", is_running: true, ...extra });
    const state = (messages: Chat["messages"], more: Partial<WorkState> = {}): WorkState => ({
      running: { job: { chatId: "c", botId: "b1" } },
      thinking: {},
      retries: {},
      chats: [chat("dm", messages)],
      bots: [...bots.values()],
      devices: [],
      ...more,
    });
    const activity = (messages: Chat["messages"], more?: Partial<WorkState>) => workingActivity(state(messages, more), "c");

    expect(activity([you])).toBeNull();
    expect(activity([you, tool("bash", { description: "Install dependencies" })])).toBe("Running command: Install dependencies…");
    // Between calls the row keeps the last one.
    expect(activity([you, tool("bash", { is_running: false })])).toBe("Running commands…");
    expect(activity([you, tool("memory_update")])).toBe("Taking a note…");
    expect(activity([you, tool("message_bot", { target_bot_id: "b2" })])).toBe("Messaging Scout…");
    expect(activity([you, tool("message_bot", { summary: "Messaged Scout", is_running: false })])).toBeNull();
    expect(activity([you, tool("github__create_issue")])).toBe("Using Github…");
    const runner = { id: "r", name: "Mac", model: "", os: "macos", os_version: "", machine_key: "", is_this_device: false, status: "online" as const, last_seen: 0, plugins: [{ id: "github", name: "GitHub", state: "ready" as const }] };
    expect(activity([you, tool("github__create_issue")], { devices: [runner] })).toBe("Using GitHub…");
    expect(activity([you, tool("routines")])).toBe("Working…");
    // What the bot said since is the news; its thinking and a retry outrank the last call.
    expect(activity([you, tool("read"), msg({ kind: "bot", bot_id: "b1" }, { kind: "text", text: "Done" })])).toBeNull();
    expect(activity([you, tool("read")], { thinking: { c: "b1" } })).toBe("Thinking…");
    expect(activity([you], { thinking: { c: "b1" }, retries: { c: { attempt: 2, max_attempts: 3, delay_ms: 3600 } } })).toBe("Retrying (2 of 3) in 4 s…");
    // Two bots at work read as their names.
    expect(activity([you, tool("read")], { running: { a: { chatId: "c", botId: "b1" }, b: { chatId: "c", botId: "b2" } } })).toBeNull();
  });

  test("stamps and separators", () => {
    const now = new Date();
    const today = new Date(now.getFullYear(), now.getMonth(), now.getDate(), 16, 13);
    expect(stamp(today)).toBe(time(today));
    expect(time(today)).toBe("4:13 PM");
    expect(daySeparator(today)).toBe("Today 4:13 PM");
    const yesterday = new Date(today.getTime() - 86400_000);
    expect(stamp(yesterday)).toBe("Yesterday");
    expect(daySeparator(yesterday)).toBe("Yesterday 4:13 PM");
    const lastYear = new Date(now.getFullYear() - 1, 8, 2, 9, 0);
    expect(stamp(lastYear)).toBe(`9/2/${String(now.getFullYear() - 1).slice(-2)}`);
    expect(daySeparator(lastYear)).toMatch(/^\w{3}, Sep 2 9:00 AM$/);
  });
});
