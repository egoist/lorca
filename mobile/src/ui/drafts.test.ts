import { expect, mock, test } from "bun:test";

mock.module("expo-localization", () => ({ getLocales: () => [{ languageCode: "en", languageTag: "en-US" }] }));
const { sameDraft } = await import("../core/model");
const { draftStateWord, draftTitle, fileSize, recipients, sendable } = await import("./drafts");

const email = { kind: "email", to: ["ana@example.com"], cc: ["bo@example.com"], subject: "Lunch", body: "Noon?", attachments: [{ name: "menu.pdf", size: 186_000 }], reply: "m-1" };

test("a draft card names what the bot wrote and how it ended", () => {
  expect(draftTitle(email, "Writer")).toBe("Writer drafted a reply");
  expect(draftTitle({ ...email, reply: undefined }, "Writer")).toBe("Writer drafted an email");
  expect(draftTitle({ kind: "slack", to: ["C024BE91L"], body: "Shipped" }, "Writer")).toBe("Writer drafted a Slack message");
  expect(draftStateWord("pending")).toBeUndefined();
  expect(draftStateWord("succeeded")).toBe("Sent");
  expect(draftStateWord("rejected")).toBe("Discarded");
  expect(fileSize(186_000)).toBe("186 KB");
  expect(fileSize(1_500_000)).toBe("1.5 MB");
});

test("an unchanged card sends without an edit, and one with nobody to send to can't send", () => {
  expect(sameDraft(email, { ...email, cc: [...email.cc] })).toBe(true);
  expect(sameDraft(email, { ...email, body: "Noon works." })).toBe(false);
  expect(sameDraft(email, { ...email, attachments: [] })).toBe(false);
  expect(recipients(" ana@example.com, ,bo@example.com ")).toEqual(["ana@example.com", "bo@example.com"]);
  expect(sendable({ ...email, to: [] })).toBe(false);
  expect(sendable({ ...email, body: "  " })).toBe(false);
  expect(sendable(email)).toBe(true);
});
