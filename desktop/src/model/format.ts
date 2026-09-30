// Dates, sizes, and schedules in the app's language, after the macOS app's `Format`.

import { isChinese, L, locale } from "../l10n";

type Options = Intl.DateTimeFormatOptions;

const formatters = new Map<string, Intl.DateTimeFormat>();

/** A formatter for a set of fields in the app's language; kept per language. */
function formatter(options: Options): Intl.DateTimeFormat {
  const key = `${locale()}|${JSON.stringify(options)}`;
  let made = formatters.get(key);
  if (!made) {
    made = new Intl.DateTimeFormat(locale(), options);
    formatters.set(key, made);
  }
  return made;
}

const DAY = 24 * 60 * 60 * 1000;

function startOfDay(ms: number): number {
  const date = new Date(ms);
  date.setHours(0, 0, 0, 0);
  return date.getTime();
}

/** Calendar days from today: 0 today, -1 yesterday, 1 tomorrow. Rounding absorbs the 23- and
 * 25-hour days of a clock change. */
function daysFromToday(ms: number): number {
  return Math.round((startOfDay(ms) - startOfDay(Date.now())) / DAY);
}

export function isToday(ms: number): boolean {
  return daysFromToday(ms) === 0;
}

export function isYesterday(ms: number): boolean {
  return daysFromToday(ms) === -1;
}

function isTomorrow(ms: number): boolean {
  return daysFromToday(ms) === 1;
}

export function isSameDay(a: number, b: number): boolean {
  return startOfDay(a) === startOfDay(b);
}

/** "4:13 AM", or "04:13" where the language keeps a 24-hour clock. */
export function time(ms: number): string {
  return formatter({ hour: "numeric", minute: "2-digit" }).format(ms);
}

/** Stamp for sidebar rows: the time today, "Yesterday", the weekday within a week, "9/2" this
 * year, "9/2/25" before that. */
export function stamp(ms: number): string {
  if (isToday(ms)) return time(ms);
  if (isYesterday(ms)) return L("Yesterday");
  if (Date.now() - ms < 7 * DAY) return formatter({ weekday: "long" }).format(ms);
  if (new Date(ms).getFullYear() === new Date().getFullYear()) return formatter({ month: "numeric", day: "numeric" }).format(ms);
  return formatter({ year: "2-digit", month: "numeric", day: "numeric" }).format(ms);
}

/** Separator before a cluster of messages: "Today 4:13 AM", "Yesterday 9:55 AM",
 * "Thu, Sep 10 9:48 AM". */
export function daySeparator(ms: number): string {
  if (isToday(ms)) return L("Today %@", time(ms));
  if (isYesterday(ms)) return L("Yesterday %@", time(ms));
  return formatter({ weekday: "short", month: "short", day: "numeric", hour: "numeric", minute: "2-digit" }).format(ms);
}

/** When an offline Device was last on the relay. The relay stamps `last_seen` as a socket
 * closes, so a Device that just dropped was seen seconds ago. */
export function lastSeen(ms: number): string {
  const elapsed = (Date.now() - ms) / 1000;
  if (elapsed < 60) return L("Last seen just now");
  if (elapsed < 3600) return L("Last seen %dm ago", Math.floor(elapsed / 60));
  if (elapsed < 60 * 60 * 24) return L("Last seen %dh ago", Math.floor(elapsed / 3600));
  return L("Last seen %dd ago", Math.floor(elapsed / 86400));
}

/** "today 9:00 AM", "tomorrow 9:00 AM", "Monday 9:00 AM", "Oct 1 9:00 AM": when a run is due. */
export function upcoming(ms: number): string {
  if (isToday(ms)) return L("today %@", time(ms));
  if (isTomorrow(ms)) return L("tomorrow %@", time(ms));
  if (ms - Date.now() < DAY * 6) return `${formatter({ weekday: "long" }).format(ms)} ${time(ms)}`;
  return `${formatter({ month: "short", day: "numeric" }).format(ms)} ${time(ms)}`;
}

/** "Today 9:00 AM" / "Sep 3, 2026, 9:00 AM": a date with its time, for "Last checked". */
export function dateTime(ms: number): string {
  if (isToday(ms)) return L("Today %@", time(ms));
  if (isYesterday(ms)) return L("Yesterday %@", time(ms));
  return formatter({ dateStyle: "medium", timeStyle: "short" }).format(ms);
}

/** "1.2 KB", "24 KB" */
export function kilobytes(bytes: number): string {
  const kb = bytes / 1000;
  return kb >= 10 || kb === Math.round(kb) ? `${Math.round(kb)} KB` : `${kb.toFixed(1)} KB`;
}

/** "950", "12k", "1.2M" */
export function tokens(count: number): string {
  if (count >= 1_000_000) return `${(count / 1_000_000).toFixed(1)}M`;
  if (count >= 1_000) return `${Math.floor(count / 1_000)}k`;
  return `${count}`;
}

/** "0:42", "12:03", "1:02:03". */
export function elapsed(fromMs: number, nowMs: number): string {
  const seconds = Math.max(0, Math.floor((nowMs - fromMs) / 1000));
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const rest = seconds % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return hours > 0 ? `${hours}:${two(minutes)}:${two(rest)}` : `${minutes}:${two(rest)}`;
}

/** The CLI words a schedule in English ("Weekdays at 9:00 AM and 5:30 PM", "Every 2 hours",
 * "On the 1st and 15th of every month at 9:00 AM"); in Chinese the same sentence is rebuilt from
 * its parts. A sentence that is not one of the CLI's (a raw cron line) reads as it came. */
export function schedule(text: string): string {
  if (!isChinese()) return text;
  const groups = (pattern: RegExp, input: string) => {
    const match = pattern.exec(input);
    return match ? match.slice(1).map((part) => part ?? "") : null;
  };
  const list = (words: string) => words.replaceAll(", and ", ", ").replaceAll(" and ", ", ").split(", ");
  const clock = (word: string) => {
    const parts = groups(/^(\d{1,2}):(\d{2}) (AM|PM)$/, word);
    return parts ? `${parts[2] === "PM" ? "下午" : "上午"} ${parts[0]}:${parts[1]}` : word;
  };
  const units: Record<string, string> = { day: "天", hour: "小时", minute: "分钟" };
  const single = groups(/^Every (day|hour|minute)$/, text);
  if (single) return `每${units[single[0]!] ?? single[0]}`;
  const interval = groups(/^Every (\d+) (day|hour|minute)s$/, text);
  if (interval) return `每 ${interval[0]} ${units[interval[1]!] ?? interval[1]}`;
  const past = groups(/^Every hour at :(\d{2})$/, text);
  if (past) return `每小时的第 ${Number(past[0])} 分`;
  const at = text.lastIndexOf(" at ");
  if (at < 0) return text;
  const times = list(text.slice(at + 4)).map(clock).join("、");
  const days = text.slice(0, at);
  if (days === "Every day") return `每天 ${times}`;
  if (days === "Weekdays") return `工作日 ${times}`;
  if (days === "Weekends") return `周末 ${times}`;
  const monthly = groups(/^On the (.+) of every month$/, days);
  if (monthly) {
    const dates = list(monthly[0]!).map((word) => `${Number(/^\d+/.exec(word)?.[0] ?? 0)} 日`).join("、");
    return `每月 ${dates} ${times}`;
  }
  const weekdays: Record<string, string> = {
    Sunday: "周日",
    Monday: "周一",
    Tuesday: "周二",
    Wednesday: "周三",
    Thursday: "周四",
    Friday: "周五",
    Saturday: "周六",
  };
  const names = list(days.startsWith("Every ") ? days.slice(6) : days).map((day) => weekdays[day]);
  if (names.every((name) => name !== undefined)) return `每${names.join("、")} ${times}`;
  return text;
}
