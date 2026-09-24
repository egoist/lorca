// Which language the speech recognizer listens in: the setting, else the first of the phone's
// preferred languages the recognizer supports (so a Chinese speaker on an English-region phone
// gets zh-CN, not en-US).

import { getLocales } from "expo-localization";
import { ExpoSpeechRecognitionModule } from "expo-speech-recognition";
import { ActionSheetIOS } from "react-native";
import { mutate, useStore } from "../core/store";
import { useEffect, useState } from "react";
import { language as appLanguage, t, useLanguage } from "../i18n";

/// Every locale the recognizer knows; Automatic matches the phone's languages against these.
let all: string[] | null = null;

/// The languages a menu offers: the common ones the recognizer knows, by name in the app's
/// language. Asked of the recognizer once.
export async function supportedLanguages(): Promise<string[]> {
  if (all) return offered();
  try {
    const { locales } = await ExpoSpeechRecognitionModule.getSupportedLocales({});
    all = [...new Set(locales.map(normalize))];
  } catch {
    all = [];
  }
  return offered();
}

function offered(): string[] {
  return (all ?? []).filter((tag) => tag in COMMON).sort((a, b) => languageName(a).localeCompare(languageName(b), appLanguage));
}

function normalize(tag: string): string {
  return tag.replace(/_/g, "-");
}

/// "zh-CN" from the phone's "zh-Hans-CN": the recognizer names locales by language and region.
export function automaticLanguage(available: string[] = all ?? []): string {
  for (const locale of getLocales()) {
    const language = locale.languageCode ?? locale.languageTag.split("-")[0];
    const sameLanguage = available.filter((tag) => tag.split("-")[0] === language);
    if (available.length === 0) return locale.regionCode ? `${language}-${locale.regionCode}` : locale.languageTag;
    if (sameLanguage.length === 0) continue;
    const region = locale.regionCode;
    return sameLanguage.find((tag) => region && tag.endsWith(`-${region}`)) ?? sameLanguage[0];
  }
  return "en-US";
}

export function useDictationLanguage(): { setting: string | undefined; language: string } {
  const setting = useStore((s) => s.dictation_lang);
  return { setting, language: setting ?? automaticLanguage() };
}

export function setDictationLanguage(tag: string | undefined) {
  mutate(() => ({ dictation_lang: tag }));
}

/// The languages the menus offer, named as the Mac app names them ("Chinese (China mainland)").
/// Hermes has no `Intl.DisplayNames`, so the names are kept here, in both of the app's languages;
/// the recognizer knows some sixty locales, and these are the ones most people speak.
const COMMON: Record<string, { en: string; zh: string }> = {
  "ar-SA": { en: "Arabic (Saudi Arabia)", zh: "阿拉伯语（沙特阿拉伯）" },
  "yue-CN": { en: "Cantonese (China mainland)", zh: "粤语（中国大陆）" },
  "zh-CN": { en: "Chinese (China mainland)", zh: "中文（中国大陆）" },
  "zh-HK": { en: "Chinese (Hong Kong)", zh: "中文（中国香港）" },
  "zh-TW": { en: "Chinese (Taiwan)", zh: "中文（台湾）" },
  "nl-NL": { en: "Dutch (Netherlands)", zh: "荷兰语（荷兰）" },
  "en-AU": { en: "English (Australia)", zh: "英语（澳大利亚）" },
  "en-IN": { en: "English (India)", zh: "英语（印度）" },
  "en-GB": { en: "English (United Kingdom)", zh: "英语（英国）" },
  "en-US": { en: "English (United States)", zh: "英语（美国）" },
  "fr-FR": { en: "French (France)", zh: "法语（法国）" },
  "de-DE": { en: "German (Germany)", zh: "德语（德国）" },
  "hi-IN": { en: "Hindi (India)", zh: "印地语（印度）" },
  "id-ID": { en: "Indonesian (Indonesia)", zh: "印度尼西亚语（印度尼西亚）" },
  "it-IT": { en: "Italian (Italy)", zh: "意大利语（意大利）" },
  "ja-JP": { en: "Japanese (Japan)", zh: "日语（日本）" },
  "ko-KR": { en: "Korean (South Korea)", zh: "韩语（韩国）" },
  "pt-BR": { en: "Portuguese (Brazil)", zh: "葡萄牙语（巴西）" },
  "ru-RU": { en: "Russian (Russia)", zh: "俄语（俄罗斯）" },
  "es-MX": { en: "Spanish (Mexico)", zh: "西班牙语（墨西哥）" },
  "es-ES": { en: "Spanish (Spain)", zh: "西班牙语（西班牙）" },
  "th-TH": { en: "Thai (Thailand)", zh: "泰语（泰国）" },
  "tr-TR": { en: "Turkish (Türkiye)", zh: "土耳其语（土耳其）" },
  "vi-VN": { en: "Vietnamese (Vietnam)", zh: "越南语（越南）" },
};

/// A language's name in the app's language. One outside the list (the phone's own, picked by
/// Automatic) is named by the system where it can be, else by its tag.
export function languageName(tag: string): string {
  const known = COMMON[tag];
  if (known) return known[appLanguage];
  try {
    return new Intl.DisplayNames([appLanguage === "zh" ? "zh-Hans" : "en"], { type: "language" }).of(tag) ?? tag;
  } catch {
    return tag;
  }
}

/// The recognizer's languages for a menu: empty until the first answer, then cached. A new app
/// language sorts them again by their new names.
export function useSupportedLanguages(): string[] {
  const { language } = useLanguage();
  const [languages, setLanguages] = useState<string[]>(offered);
  useEffect(() => {
    let live = true;
    void supportedLanguages().then((list) => live && setLanguages(list));
    return () => {
      live = false;
    };
  }, [language]);
  return languages;
}

/// The iOS language sheet opened by a long press on the microphone. Android hosts the same
/// complete list in a Material dropdown anchored to the microphone in `Composer`.
export async function pickDictationLanguage() {
  const languages = await supportedLanguages();
  const automatic = t("Automatic ({language})", { language: languageName(automaticLanguage(languages)) });
  const options = [automatic, ...languages.map(languageName)];
  const choose = (index: number) => setDictationLanguage(index === 0 ? undefined : languages[index - 1]);
  ActionSheetIOS.showActionSheetWithOptions({ options: [...options, t("Cancel")], cancelButtonIndex: options.length, title: t("Dictation language") }, (index) => {
    if (index < options.length) choose(index);
  });
}
