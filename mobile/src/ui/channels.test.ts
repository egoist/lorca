import { describe, expect, mock, test } from "bun:test";
import type { ChannelIntake, IntakeDecision, IntakeDecisionKind, TaskMandate } from "../core/model";

mock.module("../i18n", () => ({ t: (key: string) => key, tc: (key: string) => key }));
const { channelChats, channelProblem, decisionKindTitle, decisionPerson, decisionTitle, intakeChoiceFooter, intakeChoiceOf, intakeChoiceTitle, intakeWithChoice, intakeWithoutPerson, intakeWithoutRule, intakeWithPerson, intakeWithRule, listenSummary, mandateFrom, mandateWhy, personName, requestKind, shownDecisions } = await import("./channels");

describe("channels", () => {
  test("say what a channel takes", () => {
    expect(listenSummary({ mentions: true, replies: true, tags: ["feedback"] })).toBe("Mentions, replies, #feedback");
    expect(listenSummary({ every: true, mentions: true })).toBe("Every message");
    expect(listenSummary({ tags: ["bug", "idea"] })).toBe("#bug, #idea");
  });

  test("name where it listens and what holds it", () => {
    const channel = { id: "c", bot_id: "b", name: "Feedback", service: "telegram", account_id: "a", listen: {}, task: "", state: "held" as const };
    expect(channelProblem(channel)).toBe("On hold");
    expect(channelProblem({ ...channel, state: "paused" })).toBeUndefined();
    expect(channelProblem({ ...channel, state: "listening", detail: "" })).toBeUndefined();
    expect(channelProblem({ ...channel, state: "listening", detail: "Telegram’s privacy mode is on for this bot…" })).toBe("Privacy mode on");
    expect(channelChats(channel)).toBe("None yet");
    expect(channelProblem({ ...channel, state: "listening", requests: [{ id: "7", title: "Mallory", private: true }] })).toBe("A chat is waiting");
    expect(requestKind({ id: "7", title: "Mallory", private: true })).toBe("Direct message");
    expect(requestKind({ id: "-1002", title: "Beta", from: "Priya" })).toBe("Group · {name}");
    expect(channelChats({ ...channel, chats: [{ id: "-1001", title: "Acme Community" }, { id: "-1002" }] })).toBe("Acme Community, -1002");
  });

  const rule = (id: string, name = id) => ({ id, name, enabled: true, instruction: "Bugs in checkout" });

  test("intake's choice is its mode, and Off keeps what the owner wrote", () => {
    expect(intakeChoiceOf(undefined)).toBe("off");
    expect(intakeChoiceOf(null)).toBe("off");
    expect(intakeChoiceOf({ mode: "observe" })).toBe("observe");
    expect(intakeChoiceTitle("auto")).toBe("On");
    expect(intakeChoiceTitle("observe")).toBe("Only Record");
    expect(intakeChoiceFooter("off")).toBe("Each message the channel takes starts a turn.");
    expect(intakeWithChoice({ mode: "auto", people: [{ id: "7" }], rules: [rule("a")] }, "off")).toEqual({ mode: "off", people: [{ id: "7" }], rules: [rule("a")] });
    // The CLI leaves empty lists off the wire; what goes back is whole.
    expect(intakeWithChoice(null, "observe")).toEqual({ mode: "observe", people: [], rules: [] });
    expect(intakeWithChoice({ mode: "observe", people: [{ id: "7" }], rules: [rule("a")] }, "auto")).toEqual({ mode: "auto", people: [{ id: "7" }], rules: [rule("a")] });
  });

  test("people are listed once, by id, and read by name", () => {
    const intake: ChannelIntake = { mode: "auto" };
    const listed = intakeWithPerson(intake, { id: "7", name: "Priya" });
    expect(listed.people).toEqual([{ id: "7", name: "Priya" }]);
    expect(intakeWithPerson(listed, { id: "7", name: "Someone else" })).toBe(listed);
    expect(intakeWithoutPerson(listed, "7").people).toEqual([]);
    expect(intakeWithoutPerson(intake, "7").people).toBeUndefined();
    expect(personName({ id: "7", name: "Priya" })).toBe("Priya");
    expect(personName({ id: "7" })).toBe("7");
    expect(personName({ id: "7", name: "" })).toBe("7");
  });

  test("a rule replaces the one with its id, and a new one goes last with an empty id", () => {
    const intake: ChannelIntake = { mode: "auto", rules: [rule("a"), rule("b")] };
    expect(intakeWithRule(intake, { ...rule("a"), enabled: false }).rules?.map((each) => each.enabled)).toEqual([false, true]);
    expect(intakeWithRule(intake, rule("", "New")).rules?.map((each) => each.name)).toEqual(["a", "b", "New"]);
    expect(intakeWithRule({ mode: "auto", rules: [rule("", "Unsaved")] }, rule("", "New")).rules?.map((each) => each.name)).toEqual(["Unsaved", "New"]);
    expect(intakeWithRule({ mode: "observe" }, rule("c")).rules).toEqual([rule("c")]);
    expect(intakeWithoutRule(intake, "a").rules?.map((each) => each.id)).toEqual(["b"]);
  });

  const decision = (kind: IntakeDecisionKind, extra: Partial<IntakeDecision> = {}): IntakeDecision => ({ id: kind, channel_id: "c", at: 1, kind, conversation_id: "chat", message_ids: ["m"], sender: "Priya", sender_id: "7", reason: "It reports a bug", ...extra });

  test("decisions leave out what was left alone, and list ten of the rest in order", () => {
    const record = [decision("ignored"), decision("accepted", { id: "first" }), ...Array.from({ length: 12 }, (_, index) => (index % 2 ? decision("replied", { id: `r${index}` }) : decision("ignored")))];
    const shown = shownDecisions(record);
    expect(shown[0].id).toBe("first");
    expect(shown.every((each) => each.kind !== "ignored")).toBe(true);
    expect(shown.length).toBe(7);
    expect(shownDecisions(Array.from({ length: 30 }, (_, index) => decision("accepted", { id: `a${index}` }))).map((each) => each.id).at(-1)).toBe("a9");
  });

  test("a decision says what it was about, and offers to list its sender while they aren't", () => {
    expect(["accepted", "would_accept", "attached", "stopped", "replied", "unavailable"].map((kind) => decisionKindTitle(kind as IntakeDecisionKind))).toEqual(["Took on", "Would take on", "Went to its task", "Stopped", "Answered", "Couldn't assess"]);
    expect(decisionTitle(decision("accepted", { title: "Fix the link" }))).toBe("Fix the link");
    expect(decisionTitle(decision("unavailable"))).toBe("It reports a bug");
    expect(decisionTitle(decision("unavailable", { reason: "" }))).toBe("Couldn't assess");
    expect(decisionPerson(decision("replied"), { mode: "auto" })).toEqual({ id: "7", name: "Priya" });
    expect(decisionPerson(decision("replied"), { mode: "auto", people: [{ id: "7" }] })).toBeUndefined();
    expect(decisionPerson(decision("replied", { sender_id: "" }), { mode: "auto" })).toBeUndefined();
  });

  test("a mandate says who it came from and why", () => {
    const mandate = { channel_name: "Feedback", sender: "Priya", strength: "inferred", rules: [rule("a", "Bugs"), rule("b", "Docs")], scope: "Fix it" } as TaskMandate;
    expect(mandateFrom(mandate)).toBe("Priya · Feedback");
    expect(mandateFrom({ ...mandate, sender: "" })).toBe("Feedback");
    expect(mandateWhy(mandate)).toBe("Bugs, Docs");
    expect(mandateWhy({ ...mandate, rules: undefined })).toBe("");
    expect(mandateWhy({ ...mandate, strength: "explicit" })).toBe("Asked directly");
  });
});
