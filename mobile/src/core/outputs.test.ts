import { expect, mock, test } from "bun:test";
import type { Message, Output } from "./model";

mock.module("./prefs", () => ({ loadPrefs: () => ({}), savePrefs: () => {}, coreHome: () => "/unused", pathOf: (p: string) => p, wipePrefs: () => {} }));
mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
const { documentUrl, evidenceStatus, groupOutputs, outputSymbol } = await import("./model");
const { markFile, markFileError, removeChat, removeMessage, useStore } = await import("./store");

function version(id: string, n: number, minutesAgo: number, output: Partial<Output> = {}): Message {
  return {
    id: `${id}-${n}`,
    chat_id: "chat",
    author: { kind: "bot", bot_id: "bot" },
    body: { kind: "text", text: "" },
    state: { kind: "complete" },
    created_at: 1_760_000_000 - minutesAgo * 60,
    output: { id, name: `${id}.txt`, mime: "text/plain", bot_id: "bot", version: n, ...output },
  };
}

test("versions group under their output, newest first, the latest published output first", () => {
  const plain: Message = { ...version("x", 1, 0), output: undefined };
  const series = groupOutputs([version("report", 1, 30), version("chart", 1, 20), version("report", 2, 5), plain, version("report", 2, 5)]);
  expect(series.map((s) => s.id)).toEqual(["report", "chart"]);
  expect(series[0].versions.map((m) => m.output!.version)).toEqual([2, 1]);
});

test("a check reads as one word, and only an https link opens", () => {
  expect(evidenceStatus({ kind: "test_result", summary: "", status: "unverified" })).toBe("Not verified");
  expect(outputSymbol({ id: "o", name: "n", mime: "image/png", bot_id: "b", version: 1 })).toBe("photo");
  expect(documentUrl({ id: "o", name: "n", mime: "text/html", bot_id: "b", version: 1, url: "https://docs.example.com/a" })).toBe("https://docs.example.com/a");
  for (const url of ["http://example.com", "javascript:alert(1)", "https://user:secret@example.com", "file:///etc/passwd"]) {
    expect(documentUrl({ id: "o", name: "n", mime: "text/html", bot_id: "b", version: 1, url })).toBeUndefined();
  }
});

test("a failed fetch is kept until the bytes land, and listed outputs follow removals", () => {
  markFileError("att", "the relay no longer has it");
  expect(useStore.getState().fileErrors.att).toBe("the relay no longer has it");
  markFile("att", "file:///a");
  expect(useStore.getState().fileErrors.att).toBeUndefined();

  useStore.setState({ outputs: { chat: [version("report", 1, 3), version("report", 2, 1)] } });
  removeMessage("chat", "report-1");
  expect(useStore.getState().outputs.chat.map((m) => m.id)).toEqual(["report-2"]);
  removeChat("chat");
  expect(useStore.getState().outputs.chat).toBeUndefined();
});
