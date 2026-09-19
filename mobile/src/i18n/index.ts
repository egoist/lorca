// The app's words in the user's language. The key is the English text, so a string with no
// translation reads as written; `zh.ts` is the Chinese table. The language is the one picked
// in Settings, else the phone's; picking one re-renders the app from the root (`useLanguage`
// keys the navigator in `app/_layout.tsx`), so call `t()` while rendering, not at module load.

import { useSyncExternalStore } from "react";

import { zh } from "./zh";

export type Language = "en" | "zh";

export const languages: Language[] = ["en", "zh"];

/// Each language named in itself, so the list reads whatever is showing.
export const languageNames: Record<Language, string> = { en: "English", zh: "简体中文" };

const tables: Record<Language, Record<string, string>> = { en: {}, zh };

/// The phone's first language. Outside the app (the core's tests under bun) there is no native
/// module to ask, and the words stay English.
export function deviceLanguage(): Language {
  try {
    // Required here, not imported: the module pulls in React Native, which bun cannot load.
    const { getLocales } = require("expo-localization") as typeof import("expo-localization");
    return getLocales()[0]?.languageCode === "zh" ? "zh" : "en";
  } catch {
    return "en";
  }
}

/// The language picked in Settings (`app_lang` in the phone's prefs), if one was.
function savedLanguage(): Language | undefined {
  try {
    const { loadPrefs } = require("../core/prefs") as typeof import("../core/prefs");
    const saved = loadPrefs().app_lang;
    return languages.find((code) => code === saved);
  } catch {
    return undefined;
  }
}

let chosen: Language | undefined = savedLanguage();

/// The language on screen. A live binding: it changes when Settings picks another.
export let language: Language = chosen ?? deviceLanguage();

const listeners = new Set<() => void>();

/// Settings' pick; `undefined` follows the phone again.
export function setAppLanguage(next: Language | undefined) {
  const { loadPrefs, savePrefs } = require("../core/prefs") as typeof import("../core/prefs");
  savePrefs({ ...loadPrefs(), app_lang: next });
  chosen = next;
  language = next ?? deviceLanguage();
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/// The language on screen and Settings' pick, for a component that shows or follows them.
export function useLanguage(): { language: Language; chosen: Language | undefined } {
  const current = useSyncExternalStore(subscribe, () => language);
  return { language: current, chosen };
}

/// `t("{name} is working…", { name })`: the sentence in the app's language with the values in.
export function t(key: string, values?: Record<string, string | number>): string {
  const text = tables[language][key] ?? key;
  if (!values) return text;
  return text.replace(/\{(\w+)\}/g, (match, name: string) => (name in values ? String(values[name]) : match));
}
