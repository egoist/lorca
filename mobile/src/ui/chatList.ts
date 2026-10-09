// What the chat list shows, after the Mac's sidebar: the pinned chats; each section under its
// header with its chats, then the chats in no section under Chats; and the hidden chats under
// Hidden. Without sections the chats are one list with no header, and without hidden chats there
// is no Hidden. A folded group shows its header alone.

import type { Chat, Section } from "../core/model";

export type ChatGroup = { kind: "section"; id: string } | { kind: "others" } | { kind: "hidden" };

export type ChatListRow =
  | { kind: "chat"; key: string; chat: Chat }
  | { kind: "header"; key: string; group: ChatGroup; collapsed: boolean };

/// `chats` in the list's order: pinned first, then by activity.
export function chatListRows(chats: Chat[], sections: Section[], showsHidden: boolean, collapsesOthers: boolean): ChatListRow[] {
  const rows: ChatListRow[] = [];
  const chatRow = (chat: Chat): ChatListRow => ({ kind: "chat", key: chat.id, chat });
  const group = (group: ChatGroup, collapsed: boolean, members: Chat[]) => {
    rows.push({ kind: "header", key: groupKey(group), group, collapsed });
    if (!collapsed) rows.push(...members.map(chatRow));
  };
  const listed = chats.filter((chat) => !chat.is_hidden);
  const hidden = chats.filter((chat) => chat.is_hidden);
  if (!sections.length) {
    rows.push(...listed.map(chatRow));
  } else {
    const known = new Set(sections.map((section) => section.id));
    const loose = listed.filter((chat) => !chat.is_pinned);
    rows.push(...listed.filter((chat) => chat.is_pinned).map(chatRow));
    for (const section of sections) {
      group({ kind: "section", id: section.id }, !!section.collapsed, loose.filter((chat) => chat.section_id === section.id));
    }
    const others = loose.filter((chat) => !chat.section_id || !known.has(chat.section_id));
    if (others.length) group({ kind: "others" }, collapsesOthers, others);
  }
  if (hidden.length) group({ kind: "hidden" }, !showsHidden, hidden);
  return rows;
}

export function groupKey(group: ChatGroup): string {
  return group.kind === "section" ? `section:${group.id}` : group.kind;
}

/// The group a chat shows under, when the list has groups for it.
export function groupOf(chat: Chat, sections: Section[]): ChatGroup | null {
  if (chat.is_hidden) return { kind: "hidden" };
  if (!sections.length || chat.is_pinned) return null;
  return chat.section_id && sections.some((section) => section.id === chat.section_id) ? { kind: "section", id: chat.section_id } : { kind: "others" };
}
