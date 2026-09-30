// The CLI's wire shapes as the desktop model reads them.

import { expect, test } from "bun:test";
import type { Message } from "./models";
import { toMessage, type WireRun } from "./wire";

const bashRow = (run: WireRun): Message =>
  toMessage({
    id: "call",
    chat_id: "chat",
    author: { kind: "bot", bot_id: "scout" },
    body: { kind: "tool", name: "bash", summary: "Running", detail: "", is_running: true, run },
    state: { kind: "complete" },
    created_at: 820,
  });
const runOf = (message: Message) => (message.body.kind === "tool" ? message.body.tool.run : undefined);

test("a command's run says when its terminal started it", () => {
  // The row goes up before Auto-review asks; its terminal starts it after the answer.
  const started = bashRow({ session_id: "bash-1", started_at: 1000.5, command: "bun install", state: "running" });
  expect(started.createdAt).toBe(820_000);
  expect(runOf(started)?.startedAt).toBe(1_000_500);
  expect(runOf(bashRow({ command: "bun install", state: "asking" }))?.startedAt).toBeUndefined();
});
