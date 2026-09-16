// The protocol core, run with `bun test`. These modules have no React Native imports, so the
// same code the app ships is exercised here, including the vectors the Rust CLI checks in
// crates/cli/src/crypto.rs.

import { describe, expect, test } from "bun:test";
import { b64, fromUtf8, hex, unb64, utf8, uuid } from "./bytes";
import { decrypt, encrypt, encryptJson, decryptJson, seal, sealJson, unseal, unsealJson } from "./crypto";
import { identityId, Machine } from "./keys";
import { attachmentSummary, fileSize, isSentMessage, recipientName, type Body, type Chat, type Bot } from "./model";
import { parsePairingString } from "./pairing";
import { daySeparator, joinDictation, preview, stamp, time } from "../ui/format";

const SECRET = new Uint8Array(32).map((_, i) => i + 1);
const DEK = new Uint8Array(32).map((_, i) => 200 - i);

describe("bytes", () => {
  test("base64url round trip without padding", () => {
    for (const n of [0, 1, 2, 3, 31, 32, 33]) {
      const bytes = new Uint8Array(n).map((_, i) => (i * 37) & 0xff);
      const text = b64(bytes);
      expect(text).not.toContain("=");
      expect(Array.from(unb64(text))).toEqual(Array.from(bytes));
    }
    expect(b64(unb64("AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA"))).toBe("AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyA");
  });

  test("uuid shape", () => {
    expect(uuid()).toMatch(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
  });
});

describe("keys", () => {
  test("derives the same keys as the CLI", () => {
    const machine = new Machine(SECRET);
    expect(machine.pubkey).toBe("KXfnobZE_H5jKjoU2vzdWTx9SYlxt-JNBapkeKQzUVo");
    expect(machine.boxPubkey).toBe("l4G7NMHlyHxj-9EQh-izsFceTPBys8oFzbLOJwhfJ08");
    expect(identityId(machine.pubkey)).toBe("958263b260940c07");
    expect(machine.sign(utf8("abc"))).toBe("ivX9SkcoNZytd0MycLoFlmVgia78qGJjxEHbX78veJKN0Q7hcfegZoECSb37u-K4RrMsWQRd9Yspe4mqdRbXCw");
    expect(hex(machine.secret)).toBe(hex(SECRET));
  });
});

describe("crypto", () => {
  test("opens the CLI's envelope and binds the kind", () => {
    const envelope = unb64("YCLl5HooW12UVqhC4FvGBPGHxdZ1HlLcuJpi4z4e5vVejV1CXWlnSW6aFNC_5dDj6dSkBRgTnws7XwnW");
    expect(fromUtf8(decrypt(DEK, "chat", envelope))).toBe("hello from the phone");
    expect(() => decrypt(DEK, "roster", envelope)).toThrow();
  });

  test("aead round trip with fresh nonces", () => {
    const a = encrypt(DEK, "chat", utf8("x"));
    const b = encrypt(DEK, "chat", utf8("x"));
    expect(b64(a)).not.toBe(b64(b));
    expect(decryptJson(DEK, "roster", encryptJson(DEK, "roster", { bots: [] }))).toEqual({ bots: [] });
  });

  test("sealed boxes open only for the recipient", () => {
    const machine = new Machine(SECRET);
    const sealed = unb64("5GykOdJhJA8j4qUzTlAwPOLgkw4lM0TQOKeNt9iNUngUg5gbHFd_uzIfl3KpE3kIWPbYqBqxqto-Y4rhVykVkoqu");
    expect(fromUtf8(unseal(machine.boxSecret, sealed))).toBe("job for the runner");
    const other = Machine.generate();
    expect(() => unseal(other.boxSecret, sealed)).toThrow();
    const job = { id: "job-1", kind: "turn" };
    expect(unsealJson(other.boxSecret, sealJson(other.boxPubkey, job))).toEqual(job);
    expect(unseal(machine.boxSecret, seal(machine.boxPubkey, new Uint8Array(0))).length).toBe(0);
  });
});

describe("pairing", () => {
  test("parses the string the Mac shows", () => {
    const target = parsePairingString(
      " tinybot://pair?relay=http%3A%2F%2F127.0.0.1%3A18790%2F&id=Clup-vXLfBF6T2JkKpLqNOpyE9hdQbqXjrIWfdDvLbs&ek=nAanQrXTSxfK1tf7m3V2Fg-65OL84r6MeqmQmWw5uhw&n=1qUeEODuTz_A2y5jSZdBqQ \n",
    );
    expect(target).toEqual({
      relay: "http://127.0.0.1:18790",
      id: "Clup-vXLfBF6T2JkKpLqNOpyE9hdQbqXjrIWfdDvLbs",
      ek: "nAanQrXTSxfK1tf7m3V2Fg-65OL84r6MeqmQmWw5uhw",
      nonce: "1qUeEODuTz_A2y5jSZdBqQ",
    });
  });

  test("rejects other text", () => {
    expect(() => parsePairingString("hello")).toThrow("not a Tinybot pairing string");
    expect(() => parsePairingString("tinybot://pair?relay=x&id=y")).toThrow("missing a field");
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
