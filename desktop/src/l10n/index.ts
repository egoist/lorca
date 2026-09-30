// The app's words in the user's language, after the macOS app's `L()`. The key is the English
// text, so a string with no translation reads as written; the table is `zh.ts`, which
// `bun run l10n` checks against every `L("…")` and `Lc("…", "…")` in the sources.
//
// The language can change while the app runs: `L()` reads the table in force, and the window
// builds its views again (App.tsx keys them on `languageKey`). So `L()` is called when a view is
// built and its words never kept in a module-level constant.

import { createSignal } from "solid-js";
import { zh } from "./zh";

export type AppLanguageCode = "en" | "zh-Hans";

export const supportedLanguages: { code: AppLanguageCode; name: string }[] = [
  { code: "en", name: "English" },
  { code: "zh-Hans", name: "简体中文" },
];

let chosen: AppLanguageCode | null = null;
let systemLocale = typeof navigator === "undefined" ? "en-US" : navigator.language || "en-US";
let table: Record<string, string> | null = null;

const [languageKey, setLanguageKey] = createSignal(0);

/** Changes when the language does; the window's views are keyed on it. */
export { languageKey };

/** The language in force: the one picked in Settings, else the system's. */
export function languageCode(): AppLanguageCode {
  if (chosen) return chosen;
  return systemLocale.toLowerCase().startsWith("zh") ? "zh-Hans" : "en";
}

export function isChinese(): boolean {
  return languageCode() === "zh-Hans";
}

/** The pick ("en", "zh-Hans"); null follows the system. */
export function chosenLanguage(): AppLanguageCode | null {
  return chosen;
}

/** For dates and numbers: the user's region in the app's language ("zh-Hans-US"). */
export function locale(): string {
  const region = (() => {
    try {
      return new Intl.Locale(systemLocale).maximize().region;
    } catch {
      return undefined;
    }
  })();
  try {
    return new Intl.Locale(languageCode(), region ? { region } : {}).toString();
  } catch {
    return languageCode();
  }
}

/** Applies the system locale and the pick; returns whether the words changed. */
export function setLanguage(pick: string | null | undefined, system?: string): boolean {
  const before = languageCode();
  if (system) systemLocale = system;
  chosen = pick === "en" || pick === "zh-Hans" ? pick : null;
  table = languageCode() === "zh-Hans" ? zh : null;
  document.documentElement.lang = languageCode();
  const changed = before !== languageCode();
  if (changed) setLanguageKey((key) => key + 1);
  return changed;
}

function lookup(key: string): string {
  return table?.[key] ?? key;
}

/** Fills `%@` (text) and `%d` (whole numbers) in order, or `%1$@`, `%2$d` by position. */
export function format(template: string, args: (string | number)[]): string {
  let next = 0;
  return template.replace(/%(?:(\d+)\$)?([@ds%])/g, (match, position: string | undefined, kind: string) => {
    if (kind === "%") return "%";
    const index = position ? Number(position) - 1 : next++;
    const value = args[index];
    if (value === undefined) return match;
    return kind === "d" ? String(Math.trunc(Number(value))) : String(value);
  });
}

/** `L("New Bot…")`, or with values: `L("%@ is working…", name)`. */
export function L(key: string, ...args: (string | number)[]): string {
  const text = lookup(key);
  return args.length === 0 ? text : format(text, args);
}

/** One English word that means two things: `Lc("Pairing", "device state")` looks up
 * `Pairing|device state`, and reads as "Pairing" where that has no translation. */
export function Lc(text: string, context: string): string {
  return table?.[`${text}|${context}`] ?? text;
}
