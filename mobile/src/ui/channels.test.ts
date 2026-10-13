import { describe, expect, mock, test } from "bun:test";

mock.module("../i18n", () => ({ t: (key: string) => key, tc: (key: string) => key }));
const { channelChats, channelProblem, listenSummary, requestKind } = await import("./channels");

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
});
