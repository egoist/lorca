// Which language the speech recognizer listens in: the setting, else the first of the phone's
// preferred languages the recognizer supports (so a Chinese speaker on an English-region phone
// gets zh-CN, not en-US).

import { getLocales } from "expo-localization";
import { ExpoSpeechRecognitionModule } from "expo-speech-recognition";
import { ActionSheetIOS, Alert, Platform } from "react-native";
import { mutate, useStore } from "../core/store";

let supported: string[] | null = null;

/// Language tags the recognizer knows, sorted by display name; cached after the first call.
export async function supportedLanguages(): Promise<string[]> {
  if (supported) return supported;
  try {
    const { locales } = await ExpoSpeechRecognitionModule.getSupportedLocales({});
    supported = [...new Set(locales.map(normalize))].sort((a, b) => languageName(a).localeCompare(languageName(b)));
  } catch {
    supported = [];
  }
  return supported;
}

function normalize(tag: string): string {
  return tag.replace(/_/g, "-");
}

/// "zh-CN" from the phone's "zh-Hans-CN": the recognizer names locales by language and region.
export function automaticLanguage(available: string[] = supported ?? []): string {
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

export function languageName(tag: string): string {
  try {
    const names = new Intl.DisplayNames([getLocales()[0]?.languageTag ?? "en"], { type: "language" });
    return names.of(tag) ?? tag;
  } catch {
    return tag;
  }
}

/// The language sheet, from Settings or a long press on the microphone.
export async function pickDictationLanguage() {
  const languages = await supportedLanguages();
  const automatic = `Automatic (${languageName(automaticLanguage(languages))})`;
  const options = [automatic, ...languages.map(languageName)];
  const choose = (index: number) => setDictationLanguage(index === 0 ? undefined : languages[index - 1]);
  if (Platform.OS === "ios") {
    ActionSheetIOS.showActionSheetWithOptions({ options: [...options, "Cancel"], cancelButtonIndex: options.length, title: "Dictation language" }, (index) => {
      if (index < options.length) choose(index);
    });
  } else {
    Alert.alert("Dictation language", undefined, [
      ...options.slice(0, 8).map((title, index) => ({ text: title, onPress: () => choose(index) })),
      { text: "Cancel", style: "cancel" as const },
    ]);
  }
}
