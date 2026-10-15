// Coding agents as Details lists them and their page draws their steps.

import { describe, expect, test } from "bun:test";
import type { AgentItem, AgentRun, Message } from "../core/model";
import { agentStatusWord, chatAgents, exploredLine, toolLine, transcriptBlocks } from "./codingAgents";

const NOW = 1_800_000_000;
const DAY = 24 * 3600;

function start(id: string, agent: Partial<AgentRun>): Message {
  return {
    id,
    chat_id: "c",
    author: { kind: "bot", bot_id: "b" },
    body: { kind: "tool", name: "coding_agent", summary: "", detail: "", is_running: false, agent: { id: `agent-${id}`, kind: "claude", title: id, task: id, state: "working", ...agent } },
    state: { kind: "complete" },
    created_at: NOW - 10 * DAY,
  };
}

describe("coding agents", () => {
  test("Coding lists open agents first, then the latest change, and drops ones ended days ago", () => {
    const messages = [
      start("old-working", { state: "working", updated_at: NOW - 9 * DAY }),
      start("recent-idle", { state: "idle", updated_at: NOW - 3600 }),
      start("older-idle", { state: "idle", updated_at: NOW - 2 * DAY }),
      start("long-done", { state: "idle", updated_at: NOW - 4 * DAY }),
      start("asking", { state: "asking", updated_at: NOW - 60 }),
      start("never-started", { id: "", state: "failed", updated_at: NOW }),
    ];
    expect(chatAgents(messages, NOW).map((each) => each.message.id)).toEqual(["asking", "old-working", "recent-idle", "older-idle"]);
  });

  test("a finished agent with a pull request is ready for review", () => {
    const agent: AgentRun = { id: "a", kind: "codex", title: "", task: "", state: "idle" };
    expect(agentStatusWord(agent)).toBe("Finished");
    expect(agentStatusWord({ ...agent, pull_request: "https://github.com/a/b/pull/1" })).toBe("Ready for review");
    expect(agentStatusWord({ ...agent, state: "asking" })).toBe("Needs attention");
    expect(agentStatusWord({ ...agent, state: "failed" })).toBe("Run failed");
  });

  test("reads and searches in a row fold into one Explored line", () => {
    const tool = (id: string, kind: NonNullable<AgentItem["tool"]>["kind"], title: string, status: NonNullable<AgentItem["tool"]>["status"] = "done"): AgentItem => ({ id, kind: "tool", at: 0, tool: { kind, title, status } });
    const items: AgentItem[] = [
      { id: "p", kind: "prompt", at: 0, text: "Fix it" },
      tool("1", "read", "src/a.rs"),
      tool("2", "search", "fn main"),
      tool("3", "read", "src/b.rs"),
      tool("4", "edit", "src/a.rs"),
      tool("5", "read", "src/c.rs", "running"),
    ];
    const blocks = transcriptBlocks(items);
    expect(blocks.map((block) => (block.kind === "explored" ? `explored:${block.steps.length}` : block.item.kind))).toEqual(["prompt", "explored:3", "tool", "explored:1"]);
    const first = blocks[1];
    expect(first.kind === "explored" && exploredLine(first.steps, false)).toBe("Explored 2 files 1 search");
    expect(exploredLine([{ kind: "search", title: "x", status: "done" }, { kind: "search", title: "y", status: "done" }], false)).toBe("Explored 2 searches");
    expect(exploredLine([{ kind: "read", title: "x", status: "running" }], true)).toBe("Exploring 1 file");
    expect(toolLine({ kind: "execute", title: "git status", status: "done" })).toBe("Ran git status");
    expect(toolLine({ kind: "execute", title: "cargo test", status: "running" })).toBe("Running cargo test");
    expect(toolLine({ kind: "edit", title: "src/a.rs", status: "denied" })).toBe("Not allowed: src/a.rs");
  });
});
