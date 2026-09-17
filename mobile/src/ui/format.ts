// Stamps and previews, the rules the Mac app uses (Design/Formatters.swift, AppStore.preview).

import type { Bot, Chat, Routine } from "../core/model";
import { attachmentSummary, isSentMessage, recipientName } from "../core/model";

const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const WEEKDAYS_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export function time(date: Date): string {
  let h = date.getHours();
  const m = date.getMinutes().toString().padStart(2, "0");
  const suffix = h >= 12 ? "PM" : "AM";
  h = h % 12;
  if (h === 0) h = 12;
  return `${h}:${m} ${suffix}`;
}

export function isSameDay(a: Date, b: Date): boolean {
  return a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();
}

function isToday(date: Date): boolean {
  return isSameDay(date, new Date());
}

function isYesterday(date: Date): boolean {
  const y = new Date();
  y.setDate(y.getDate() - 1);
  return isSameDay(date, y);
}

/// Sidebar stamp: the time today, "Yesterday", the weekday within a week, "9/2" this year,
/// "9/2/25" before that.
export function stamp(date: Date): string {
  if (isToday(date)) return time(date);
  if (isYesterday(date)) return "Yesterday";
  const now = new Date();
  if (now.getTime() - date.getTime() < 7 * 86400_000) return WEEKDAYS[date.getDay()];
  if (now.getFullYear() === date.getFullYear()) return `${date.getMonth() + 1}/${date.getDate()}`;
  return `${date.getMonth() + 1}/${date.getDate()}/${date.getFullYear().toString().slice(-2)}`;
}

/// Separator before a cluster of messages: "Today 4:13 AM", "Yesterday 9:55 AM",
/// "Thu, Sep 10 9:48 AM".
export function daySeparator(date: Date): string {
  if (isToday(date)) return `Today ${time(date)}`;
  if (isYesterday(date)) return `Yesterday ${time(date)}`;
  return `${WEEKDAYS_SHORT[date.getDay()]}, ${MONTHS[date.getMonth()]} ${date.getDate()} ${time(date)}`;
}

/// "today 9:00 AM", "tomorrow 9:00 AM", "Monday 9:00 AM", "Oct 1 9:00 AM": when a run is due.
export function upcoming(unix: number): string {
  const date = new Date(unix * 1000);
  if (isToday(date)) return `today ${time(date)}`;
  const tomorrow = new Date();
  tomorrow.setDate(tomorrow.getDate() + 1);
  if (isSameDay(date, tomorrow)) return `tomorrow ${time(date)}`;
  if (date.getTime() - Date.now() < 6 * 86400_000) return `${WEEKDAYS[date.getDay()]} ${time(date)}`;
  return `${MONTHS[date.getMonth()]} ${date.getDate()} ${time(date)}`;
}

/// The line under a routine's name: the schedule, then what is going on.
export function routineDetail(routine: Routine): string {
  if (routine.is_running) return `${routine.schedule_text} · Running…`;
  if (!routine.is_enabled) return `${routine.schedule_text} · ${routine.paused_reason === "away" ? "Paused while you were away" : "Paused"}`;
  if (routine.next_run_at) return `${routine.schedule_text} · Next ${upcoming(routine.next_run_at)}`;
  return routine.schedule_text;
}

/// "Today 9:00 AM · replied", "Never", "Yesterday 6:00 PM · nothing to report".
export function lastRunSummary(routine: Routine): string {
  if (!routine.last_run_at) return "Never";
  const when = daySeparator(new Date(routine.last_run_at * 1000));
  switch (routine.last_outcome) {
    case "sent":
      return `${when} · replied`;
    case "pass":
      return `${when} · nothing to report`;
    case "error":
      return `${when} · failed`;
    default:
      return when;
  }
}

export function lastSeen(seenUnix: number | undefined): string {
  if (!seenUnix) return "Offline";
  const elapsed = Date.now() / 1000 - seenUnix;
  if (elapsed < 150) return "Active now";
  if (elapsed < 3600) return `Last seen ${Math.floor(elapsed / 60)}m ago`;
  if (elapsed < 86400) return `Last seen ${Math.floor(elapsed / 3600)}h ago`;
  return `Last seen ${Math.floor(elapsed / 86400)}d ago`;
}

/// The last thing worth previewing: tool calls never are, except a sent message.
export function preview(chat: Chat, bots: Map<string, Bot>): string {
  const shown = [...chat.messages].reverse().find((m) => (m.body.kind === "tool" ? isSentMessage(m.body) : true));
  if (!shown) return "No messages yet";
  let body: string;
  switch (shown.body.kind) {
    case "text":
      body = shown.body.text || attachmentSummary(shown.body.attachments ?? []);
      break;
    case "tool":
      body = `Messaged ${recipientName(shown.body)}: ${shown.body.detail}`;
      break;
    case "handoff":
      body =
        chat.kind !== "group" && chat.bot_ids.includes(shown.body.to)
          ? `Message from ${bots.get(shown.body.from)?.name ?? "a teammate"}: ${shown.body.reason}`
          : `Handed off to ${bots.get(shown.body.to)?.name ?? "a teammate"}`;
      break;
    case "notice":
      body = shown.body.text;
      break;
  }
  const flattened = body.replace(/\n/g, " ").replace(/\*\*/g, "").replace(/`/g, "").trim();
  if (chat.kind === "group" && shown.author.kind === "bot" && shown.body.kind === "text") {
    return `${bots.get(shown.author.bot_id)?.name ?? "Bot"}: ${flattened}`;
  }
  return flattened;
}

export function lastActivity(chat: Chat): number {
  return chat.messages.length ? chat.messages[chat.messages.length - 1].created_at : chat.created_at;
}

/// Dictated words go after whatever was typed, separated by one space.
export function joinDictation(base: string, transcript: string): string {
  if (!transcript) return base;
  if (!base) return transcript;
  return /\s$/.test(base) ? base + transcript : `${base} ${transcript}`;
}
