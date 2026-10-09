import { describe, expect, test } from "bun:test";
import type { PluginStatus } from "../core/model";
import { accountOf, accountRow, configureParams, draftOf, isEdited, nextStep, sampleText, type WorkflowConnection, type WorkflowProgress } from "./workflows";

const gmail = (id: string, account_name: string, state: PluginStatus["state"] = "ready"): PluginStatus => ({ id, name: `Gmail · ${account_name}`, service_id: "gmail", account_name, state });
const connection = (over: Partial<WorkflowConnection>): WorkflowConnection => ({ service_id: "gmail", name: "Gmail", selected_id: null, choices: [], available: true, ...over });

function progress(over: { connections?: WorkflowConnection[]; answers?: Record<string, string>; sample?: WorkflowProgress["setup"]["sample"]; phase?: string; running?: boolean; messages?: string[]; selectedBot?: string | null } = {}): WorkflowProgress {
  return {
    setup: {
      id: "workflow-1",
      runner_id: "runner",
      pack: { id: "inbox-triage", name: "Inbox triage", outcome: "", description: "", symbol_name: "envelope", questions: [{ id: "scope", label: "Messages", placeholder: "" }], connections: [{ service_id: "gmail", name: "Gmail" }] },
      answers: over.answers ?? { scope: "Unread" },
      bot_ids: over.selectedBot ? { triager: over.selectedBot } : {},
      connection_ids: {},
      phase: over.phase ?? "connections",
      sample: over.sample ?? null,
    },
    connections: over.connections ?? [connection({ choices: [gmail("gmail-work", "Work")] })],
    specialists: [{ id: "triager", name: "Inbox Triager", selected_id: over.selectedBot ?? null, choices: [{ id: "bot-1", name: "Inbox Triager" }] }],
    routines: [{ id: "routine", name: "Triage", schedule_text: "Weekdays at 9:00 AM", is_enabled: over.phase === "enabled" }],
    sample_messages: (over.messages ?? []).map((text, i) => ({ id: `m${i}`, body: { text } })),
    is_running: over.running ?? false,
  };
}

describe("workflows", () => {
  test("a service uses the chosen account, the Runner's only one, or waits for a pick of several", () => {
    const work = gmail("gmail-work", "Work");
    const personal = gmail("gmail-personal", "Personal", "needs_auth");
    expect(accountOf(connection({ choices: [work] }))).toBe(work);
    expect(accountOf(connection({ choices: [work, personal], selected_id: "gmail-personal" }))).toBe(personal);
    expect(accountOf(connection({ choices: [work, personal] }))).toBeUndefined();
    expect(accountRow(connection({ choices: [work] }))).toEqual({ detail: "Connected", opens: true });
    expect(accountRow(connection({ choices: [work, personal], selected_id: "gmail-personal" }))).toEqual({ detail: "Needs sign-in", opens: true });
    expect(accountRow(connection({ choices: [work, personal] })).detail).toBe("Not chosen");
    // Chosen on a computer and not reported by the Runner yet; none yet, added on a computer; or
    // not in this marketplace.
    expect(accountRow(connection({ selected_id: "gmail-new" })).detail).toBe("Connecting…");
    expect(accountRow(connection({}))).toEqual({ detail: "Not added", opens: false, needsComputer: true });
    expect(accountRow(connection({ available: false })).detail).toBe("Not available");
  });

  test("the sample waits for the answers and the accounts, then the schedule waits for its result", () => {
    const fresh = progress({ answers: {} });
    expect(nextStep(fresh, draftOf(fresh))).toEqual({ kind: "run", disabled: "Fill in the setup first." });
    expect(nextStep(fresh, { ...draftOf(fresh), answers: { scope: "Unread" } })).toEqual({ kind: "run" });
    const signIn = progress({ connections: [connection({ choices: [gmail("gmail-work", "Work", "needs_auth")] })] });
    expect(nextStep(signIn, draftOf(signIn))).toEqual({ kind: "run", disabled: "Connect the accounts first." });
    const running = progress({ running: true, sample: { job_id: "j", chat_id: "c", bot_id: "bot-1", state: "running" } });
    expect(nextStep(running, draftOf(running))).toEqual({ kind: "run", disabled: "" });
    const failed = progress({ sample: { job_id: "j", chat_id: "c", bot_id: "bot-1", state: "failed" } });
    expect(nextStep(failed, draftOf(failed))).toEqual({ kind: "run" });
    const result = progress({ messages: ["One urgent message.", "And a draft."], sample: { job_id: "j", chat_id: "c", bot_id: "bot-1", state: "ready" }, selectedBot: "bot-1" });
    expect(nextStep(result, draftOf(result))).toEqual({ kind: "decide" });
    expect(sampleText(result)).toBe("One urgent message.\n\nAnd a draft.");
    expect(nextStep(progress({ phase: "enabled" }), draftOf(progress()))).toEqual({ kind: "on" });
  });

  test("an edit after the result asks for another sample, and the save trims and leaves the new bot to setup", () => {
    const result = progress({ messages: ["Done."], sample: { job_id: "j", chat_id: "c", bot_id: "bot-1", state: "reviewed" } });
    const edited = { ...draftOf(result), answers: { scope: "  Starred  " } };
    expect(isEdited(result, draftOf(result))).toBe(false);
    expect(isEdited(result, edited)).toBe(true);
    expect(nextStep(result, edited)).toEqual({ kind: "run" });
    expect(isEdited(result, { ...draftOf(result), bots: { triager: "bot-1" } })).toBe(true);
    expect(configureParams(result, edited)).toEqual({ id: "workflow-1", answers: { scope: "Starred" }, bot_ids: {} });
    expect(configureParams(result, { ...edited, bots: { triager: "bot-1" } }).bot_ids).toEqual({ triager: "bot-1" });
  });
});
