import { describe, expect, test } from "bun:test";
import type { Chat, Message } from "../core/model";
import { countsLine, diffLines, feedbackProblem, feedbackSources, isEmpty, noteTitle, targetName, toBotFeedback, type WireFeedback } from "./feedback";

// What `feedback.list` answers, as the CLI writes it.
const wire: WireFeedback = {
  feedback: [
    { id: "f2", kind: "edited", origin: { chat_id: "dm", message_id: "m2" }, note: "", example: "Lead with decisions.", target: { kind: "routine_prompt", id: "rt-1" }, created_at: 20 },
    { id: "f1", kind: "routine_failure", origin: { chat_id: "dm", message_id: "m1" }, note: "Checklist missing.", example: "", target: { kind: "routine_prompt", id: "rt-1" }, created_at: 10 },
  ],
  feedback_count: 2,
  proposals: [
    { id: "p1", target: { kind: "routine_prompt", id: "rt-1" }, explanation: "Older", diff: "", diff_hash: "h1", evidence: ["f2"], created_at: 30 },
    { id: "p2", target: { kind: "playbook", id: "s1", scope: { kind: "bot", id: "chef" } }, explanation: "Newer", diff: "", diff_hash: "h2", evidence: [], created_at: 40 },
  ],
  revisions: [{ id: "r1", target: { kind: "playbook", id: "s1", scope: { kind: "bot", id: "chef" } }, created_at: 50, rollback_of: null, diff: "+x", can_rollback: true, current_hash: "c" }],
  settings: { review_every_secs: null },
  targets: [
    { name: "Morning brief", target: { kind: "routine_prompt", id: "rt-1" } },
    { name: "launch-brief", target: { kind: "playbook", id: "s1", scope: { kind: "bot", id: "chef" } } },
  ],
};

describe("feedback", () => {
  test("the list reads newest first, and a note without words shows its message", () => {
    const f = toBotFeedback(wire);
    expect(f.suggestions.map((s) => s.id)).toEqual(["p2", "p1"]);
    expect(f.notes[0].text).toBe("Lead with decisions.");
    expect(f.reviewEvery).toBeUndefined();
    expect(f.changes[0]).toMatchObject({ canUndo: true, isUndo: false, currentHash: "c" });
    expect(targetName(f, f.changes[0].target, [])).toBe("launch-brief");
    expect(noteTitle(f, f.notes[1], [])).toBe("Morning brief");
    expect(countsLine(f)).toBe("2 notes · 1 change");
    expect(isEmpty(f)).toBe(false);
    expect(isEmpty(toBotFeedback({ ...wire, feedback: [], feedback_count: 0, proposals: [], revisions: [] }))).toBe(true);
  });

  test("a diff shows only its changed lines, and a refusal reads in the app's words", () => {
    expect(diffLines("--- current\n+++ proposed\n@@ -1,1 +1,2 @@\n-Old\n+New\n+\n")).toEqual([
      { removed: true, text: "Old" },
      { removed: false, text: "New" },
      { removed: false, text: "" },
    ]);
    expect(feedbackProblem("The routine changed; review a fresh proposal", "Morning brief")).toBe("“Morning brief” changed after this, so the change can't be made.");
    expect(feedbackProblem("Runner is offline", "x")).toBe("Runner is offline");
  });

  test("feedback goes on the bot's finished replies, newest first", () => {
    const m = (id: string, author: Message["author"], state: Message["state"] = { kind: "complete" }): Message => ({ id, chat_id: "dm", author, body: { kind: "text", text: id }, state, created_at: 1 });
    const chat: Chat = { id: "dm", kind: "dm", bot_ids: ["chef"], is_pinned: false, created_at: 0, unread_count: 0, messages: [m("a", { kind: "bot", bot_id: "chef" }), m("you", { kind: "you" }), m("b", { kind: "bot", bot_id: "chef" }), m("c", { kind: "bot", bot_id: "chef" }, { kind: "streaming" })] };
    expect(feedbackSources(chat).map((each) => each.id)).toEqual(["b", "a"]);
  });
});
