import { describe, expect, mock, test } from "bun:test";

mock.module("../i18n", () => ({ t: (key: string) => key, tc: (key: string) => key }));
const { channelChats, channelProblem, listenSummary } = await import("./channels");

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
    expect(channelChats(channel)).toBe("Every chat the bot is in");
    expect(channelChats({ ...channel, chats: [{ id: "-1001", title: "Acme Community" }, { id: "-1002" }] })).toBe("Acme Community, -1002");
  });
});
