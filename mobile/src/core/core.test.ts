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
  recipientName,
  type Body,
  type Bot,
  type Chat,
} from "./model";
import { parsePairingString } from "./pairing";
import { daySeparator, joinDictation, preview, stamp, time } from "../ui/format";

let nextId = 0;
const uuid = () => `m-${++nextId}`;

describe("pairing", () => {
  test("parses the string the Mac shows", () => {
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
    expect(recipientName(sent)).toBe("Scout");
    expect(isSentMessage(running)).toBe(false);
    expect(isSentMessage(failed)).toBe(false);
    expect(isSentMessage({ kind: "text", text: "Messaged Scout" })).toBe(false);
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
    expect(PROVIDER_MODELS.chatgpt.at(-1)).toEqual({ id: "gpt-5.3-codex-spark", label: "Codex Spark (Pro)" });
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
    ["b1", { id: "b1", name: "Chef", label: "", description: "", symbol_name: "sparkles", accent: "indigo", runner_id: "r", provider: "deepseek", instructions: "", created_at: 0 }],
    ["b2", { id: "b2", name: "Scout", label: "", description: "", symbol_name: "magnifyingglass", accent: "teal", runner_id: "r", provider: "deepseek", instructions: "", created_at: 0 }],
  ]);
  const chat = (kind: "dm" | "group", messages: Chat["messages"]): Chat => ({ id: "c", kind, bot_ids: ["b1", "b2"], is_pinned: false, created_at: 0, messages, unread_count: 0 });
  const msg = (author: Chat["messages"][number]["author"], body: Body): Chat["messages"][number] => ({ id: uuid(), chat_id: "c", author, body, state: { kind: "complete" }, created_at: 1 });

  test("previews the way the Mac sidebar does", () => {
    expect(preview(chat("dm", []), bots)).toBe("No messages yet");
    expect(preview(chat("dm", [msg({ kind: "you" }, { kind: "text", text: "hi\nthere **now**" })]), bots)).toBe("hi there now");
    const photo = { id: "att-1", name: "IMG_1.jpg", mime: "image/jpeg", size: 1 };
    expect(preview(chat("dm", [msg({ kind: "you" }, { kind: "text", text: "", attachments: [photo] })]), bots)).toBe("Photo");
    expect(preview(chat("group", [msg({ kind: "bot", bot_id: "b2" }, { kind: "text", text: "yes" })]), bots)).toBe("Scout: yes");
    const tool = msg({ kind: "bot", bot_id: "b1" }, { kind: "tool", name: "read", summary: "Read a file", detail: "x", is_running: false });
    expect(preview(chat("dm", [msg({ kind: "you" }, { kind: "text", text: "go" }), tool]), bots)).toBe("go");
    const sent = msg({ kind: "bot", bot_id: "b1" }, { kind: "tool", name: "message_bot", summary: "Messaged Scout", detail: "please look", is_running: false });
    expect(preview(chat("dm", [sent]), bots)).toBe("Messaged Scout: please look");
    const handoff = msg({ kind: "bot", bot_id: "b1" }, { kind: "handoff", from: "b1", to: "b2", reason: "over to you" });
    expect(preview({ ...chat("dm", [handoff]), bot_ids: ["b2"] }, bots)).toBe("Message from Chef: over to you");
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
