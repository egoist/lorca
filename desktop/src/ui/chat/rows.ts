// The transcript's rows, after the macOS app's ChatViewController.rebuildRows: a day separator
// after fifteen minutes of silence or a new day, the messages (tool calls only as a sent
// message's marker or a command's card while it needs the user), and at the end the working row
// or a status line.

import { isSameDay } from "../../model/format";
import { isShown, sameAuthor, type Chat, type Message } from "../../model/models";

/** A new "Today 4:13 AM" separator after this much silence. */
export const separatorGap = 15 * 60 * 1000;

export type ChatRow =
  | { kind: "day"; key: string; at: number }
  | { kind: "message"; key: string; message: Message; groupStart: boolean }
  /** Bots with a turn running, shown after the last message. */
  | { kind: "working"; key: string; botIDs: string[] }
  /** A one-line note after the last message, such as "Chef stopped without replying". */
  | { kind: "status"; key: string; text: string };

export function buildRows(chat: Chat, working: string[], stoppedNotice: string | null): ChatRow[] {
  const rows: ChatRow[] = [];
  let previousAuthor: Message["author"] | undefined;
  let previousDate: number | undefined;
  for (const message of chat.messages) {
    // Tool calls are the bot's business; a sent message leaves a marker, and a command shows as
    // its card while it needs the user.
    if (message.body.kind === "tool" && !isShown(message.body.tool)) continue;
    const silence = previousDate === undefined ? Infinity : message.createdAt - previousDate;
    if (silence > separatorGap || previousDate === undefined || !isSameDay(previousDate, message.createdAt)) {
      rows.push({ kind: "day", key: `day:${message.id}`, at: message.createdAt });
      previousAuthor = undefined;
    }
    const isChrome = message.body.kind !== "text";
    rows.push({ kind: "message", key: message.id, message, groupStart: isChrome || !sameAuthor(previousAuthor, message.author) });
    previousAuthor = isChrome ? undefined : message.author;
    previousDate = message.createdAt;
  }
  if (working.length > 0) rows.push({ kind: "working", key: "working", botIDs: working });
  else if (stoppedNotice) rows.push({ kind: "status", key: "status", text: stoppedNotice });
  return rows;
}
