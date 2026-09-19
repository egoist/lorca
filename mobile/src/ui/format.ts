// Stamps and previews, the rules the Mac app uses (Design/Formatters.swift, AppStore.preview).

import type { Bot, Chat, Routine } from "../core/model";
import { attachmentSummary, isSentMessage, recipientName } from "../core/model";
import { language, t } from "../i18n";

const WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const WEEKDAYS_SHORT = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const WEEKDAYS_ZH = ["星期日", "星期一", "星期二", "星期三", "星期四", "星期五", "星期六"];
const WEEKDAYS_SHORT_ZH = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

function weekday(date: Date): string {
  return (language === "zh" ? WEEKDAYS_ZH : WEEKDAYS)[date.getDay()];
}

/// "Sep 10", "9月10日".
function monthDay(date: Date): string {
  return language === "zh" ? `${date.getMonth() + 1}月${date.getDate()}日` : `${MONTHS[date.getMonth()]} ${date.getDate()}`;
}

export function time(date: Date): string {
  let h = date.getHours();
  const m = date.getMinutes().toString().padStart(2, "0");
  const afternoon = h >= 12;
  h = h % 12;
  if (h === 0) h = 12;
  if (language === "zh") return `${afternoon ? "下午" : "上午"} ${h}:${m}`;
  return `${h}:${m} ${afternoon ? "PM" : "AM"}`;
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
  if (isYesterday(date)) return t("Yesterday");
  const now = new Date();
  if (now.getTime() - date.getTime() < 7 * 86400_000) return weekday(date);
  if (now.getFullYear() === date.getFullYear()) return `${date.getMonth() + 1}/${date.getDate()}`;
  if (language === "zh") return `${date.getFullYear()}/${date.getMonth() + 1}/${date.getDate()}`;
  return `${date.getMonth() + 1}/${date.getDate()}/${date.getFullYear().toString().slice(-2)}`;
}

/// Separator before a cluster of messages: "Today 4:13 AM", "Yesterday 9:55 AM",
/// "Thu, Sep 10 9:48 AM".
export function daySeparator(date: Date): string {
  if (isToday(date)) return t("Today {time}", { time: time(date) });
  if (isYesterday(date)) return t("Yesterday {time}", { time: time(date) });
  if (language === "zh") return `${monthDay(date)} ${WEEKDAYS_SHORT_ZH[date.getDay()]} ${time(date)}`;
  return `${WEEKDAYS_SHORT[date.getDay()]}, ${MONTHS[date.getMonth()]} ${date.getDate()} ${time(date)}`;
}

/// "today 9:00 AM", "tomorrow 9:00 AM", "Monday 9:00 AM", "Oct 1 9:00 AM": when a run is due.
export function upcoming(unix: number): string {
  const date = new Date(unix * 1000);
  if (isToday(date)) return t("today {time}", { time: time(date) });
  const tomorrow = new Date();
  tomorrow.setDate(tomorrow.getDate() + 1);
  if (isSameDay(date, tomorrow)) return t("tomorrow {time}", { time: time(date) });
  if (date.getTime() - Date.now() < 6 * 86400_000) return `${weekday(date)} ${time(date)}`;
  return `${monthDay(date)} ${time(date)}`;
}

/// The core words a schedule in English ("Weekdays at 9:00 AM and 5:30 PM", "Every 2 hours",
/// "On the 1st and 15th of every month at 9:00 AM"); in Chinese the same sentence is rebuilt
/// from its parts. A sentence that is not one of the core's (a raw cron line) reads as it came.
export function scheduleText(text: string): string {
  if (language !== "zh") return text;
  const list = (words: string) => words.split(/, and | and |, /);
  const clock = (word: string) => {
    const match = /^(\d{1,2}):(\d{2}) (AM|PM)$/.exec(word);
    return match ? `${match[3] === "PM" ? "下午" : "上午"} ${match[1]}:${match[2]}` : word;
  };
  const units: Record<string, string> = { day: "天", hour: "小时", minute: "分钟" };
  const single = /^Every (day|hour|minute)$/.exec(text);
  if (single) return `每${units[single[1]]}`;
  const interval = /^Every (\d+) (day|hour|minute)s$/.exec(text);
  if (interval) return `每 ${interval[1]} ${units[interval[2]]}`;
  const past = /^Every hour at :(\d{2})$/.exec(text);
  if (past) return `每小时的第 ${Number(past[1])} 分`;
  const at = text.lastIndexOf(" at ");
  if (at < 0) return text;
  const times = list(text.slice(at + 4)).map(clock).join("、");
  const days = text.slice(0, at);
  const short = (name: string) => {
    const index = WEEKDAYS.indexOf(name);
    return index < 0 ? null : WEEKDAYS_SHORT_ZH[index];
  };
  if (days === "Every day") return `每天 ${times}`;
  if (days === "Weekdays") return `工作日 ${times}`;
  if (days === "Weekends") return `周末 ${times}`;
  const monthly = /^On the (.+) of every month$/.exec(days);
  if (monthly) return `每月 ${list(monthly[1]).map((d) => `${parseInt(d, 10)} 日`).join("、")} ${times}`;
  const names = list(days.replace(/^Every /, "")).map(short);
  if (names.every((name): name is string => name !== null)) return `每${names.join("、")} ${times}`;
  return text;
}

/// The line under a routine's name: the schedule, then what is going on.
export function routineDetail(routine: Routine): string {
  const schedule = scheduleText(routine.schedule_text);
  if (routine.is_running) return `${schedule} · ${t("Running…")}`;
  if (!routine.is_enabled) return `${schedule} · ${routine.paused_reason === "away" ? t("Paused while you were away") : t("Paused")}`;
  if (routine.next_run_at) return `${schedule} · ${t("Next {when}", { when: upcoming(routine.next_run_at) })}`;
  return schedule;
}

/// "Today 9:00 AM · replied", "Never", "Yesterday 6:00 PM · nothing to report".
export function lastRunSummary(routine: Routine): string {
  if (!routine.last_run_at) return t("Never");
  const when = daySeparator(new Date(routine.last_run_at * 1000));
  switch (routine.last_outcome) {
    case "sent":
      return `${when} · ${t("replied")}`;
    case "pass":
      return `${when} · ${t("nothing to report")}`;
    case "error":
      return `${when} · ${t("failed")}`;
    default:
      return when;
  }
}

export function lastSeen(seenUnix: number | undefined): string {
  if (!seenUnix) return t("Offline");
  const elapsed = Date.now() / 1000 - seenUnix;
  if (elapsed < 150) return t("Active now");
  if (elapsed < 3600) return t("Last seen {count}m ago", { count: Math.floor(elapsed / 60) });
  if (elapsed < 86400) return t("Last seen {count}h ago", { count: Math.floor(elapsed / 3600) });
  return t("Last seen {count}d ago", { count: Math.floor(elapsed / 86400) });
}

/// The last thing worth previewing: tool calls never are, except a sent message.
export function preview(chat: Chat, bots: Map<string, Bot>): string {
  const shown = [...chat.messages].reverse().find((m) => (m.body.kind === "tool" ? isSentMessage(m.body) : true));
  if (!shown) return t("No messages yet");
  let body: string;
  switch (shown.body.kind) {
    case "text":
      body = shown.body.text || attachmentSummary(shown.body.attachments ?? []);
      break;
    case "tool":
      body = t("Messaged {name}: {detail}", { name: recipientName(shown.body), detail: shown.body.detail });
      break;
    case "handoff":
      body =
        chat.kind !== "group" && chat.bot_ids.includes(shown.body.to)
          ? t("Message from {name}: {reason}", { name: bots.get(shown.body.from)?.name ?? t("a teammate"), reason: shown.body.reason })
          : t("Handed off to {name}", { name: bots.get(shown.body.to)?.name ?? t("a teammate") });
      break;
    case "notice":
      body = shown.body.text;
      break;
    case "permission": {
      const who = shown.author.kind === "bot" ? (bots.get(shown.author.bot_id)?.name ?? t("A bot")) : t("A bot");
      const plugin = shown.body.plugin_name;
      body =
        shown.body.tool === "connect"
          ? t("{who} needs a sign-in to {plugin}", { who, plugin })
          : shown.body.tool === "install"
            ? t("{who} wants to install {plugin}", { who, plugin })
            : t("{who} wants to use {plugin}", { who, plugin });
      break;
    }
  }
  const flattened = body.replace(/\n/g, " ").replace(/\*\*/g, "").replace(/`/g, "").trim();
  if (chat.kind === "group" && shown.author.kind === "bot" && shown.body.kind === "text") {
    return `${bots.get(shown.author.bot_id)?.name ?? t("Bot")}: ${flattened}`;
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

/// The first non-empty line of a message, fence markers skipped: what a one-line preview shows.
export function firstLine(text: string): string {
  for (const raw of text.split(/\r?\n/)) {
    const line = raw.trim();
    if (line && !line.startsWith("```")) return line;
  }
  return "";
}
