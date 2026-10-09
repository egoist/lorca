import { describe, expect, test } from "bun:test";
import { isMuted, sectionName, type Chat, type Section } from "../core/model";
import { chatListRows, groupOf, type ChatListRow } from "./chatList";

const chat = (id: string, extra: Partial<Chat> = {}): Chat => ({ id, kind: "dm", bot_ids: ["bot"], is_pinned: false, created_at: 0, unread_count: 0, messages: [], ...extra });
const names = (rows: ChatListRow[]) => rows.map((row) => (row.kind === "chat" ? row.chat.id : `[${row.key}]`));

describe("chat list", () => {
  test("without sections the chats are one list, and hidden ones fold away under Hidden", () => {
    const chats = [chat("pinned", { is_pinned: true }), chat("a"), chat("gone", { is_hidden: true })];
    expect(names(chatListRows(chats, [], false, false))).toEqual(["pinned", "a", "[hidden]"]);
    expect(names(chatListRows(chats, [], true, false))).toEqual(["pinned", "a", "[hidden]", "gone"]);
    expect(names(chatListRows(chats.slice(0, 2), [], false, false))).toEqual(["pinned", "a"]);
  });

  test("sections list their chats, then the rest under Chats; folded groups show their header", () => {
    const sections: Section[] = [{ id: "s1", name: "Pipeline", collapsed: true }, { id: "s2", name: "Empty" }];
    const chats = [chat("pinned", { is_pinned: true, section_id: "s1" }), chat("deal", { section_id: "s1" }), chat("loose"), chat("orphan", { section_id: "deleted" })];
    expect(names(chatListRows(chats, sections, false, false))).toEqual(["pinned", "[section:s1]", "[section:s2]", "[others]", "loose", "orphan"]);
    expect(names(chatListRows(chats, [{ ...sections[0], collapsed: false }], false, true))).toEqual(["pinned", "[section:s1]", "deal", "[others]"]);
    expect(names(chatListRows(chats.slice(0, 2), sections, false, false))).not.toContain("[others]");
    expect(groupOf(chats[1], sections)).toEqual({ kind: "section", id: "s1" });
    expect(groupOf(chats[3], sections)).toEqual({ kind: "others" });
    expect(groupOf(chats[0], sections)).toBeNull();
  });

  test("a mute until a time runs out on its own, and section names are one short line", () => {
    const now = 1_000_000;
    expect(isMuted(chat("a", { mute: { until: now / 1000 + 1 } }), now)).toBe(true);
    expect(isMuted(chat("a", { mute: { until: now / 1000 } }), now)).toBe(false);
    expect(isMuted(chat("a", { mute: {} }), now)).toBe(true);
    expect(isMuted(chat("a"), now)).toBe(false);
    expect(sectionName("  Big \n deals ")).toBe("Big deals");
    expect(sectionName("x".repeat(80))).toHaveLength(60);
  });
});
