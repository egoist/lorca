// The few things the phone keeps for itself, outside the core: one small JSON file in the
// app's documents directory. Everything about the account lives in the core's own folder.

import { Directory, File, Paths } from "expo-file-system";

export interface Prefs {
  /// Speech recognizer language tag; unset follows the phone's preferred languages.
  dictation_lang?: string;
  /// The app's own language ("en", "zh"); unset follows the phone's.
  app_lang?: string;
}

function root(): Directory {
  const dir = new Directory(Paths.document, "lorca");
  if (!dir.exists) dir.create({ intermediates: true, idempotent: true });
  return dir;
}

function prefsFile(): File {
  return new File(root(), "prefs.json");
}

/// Where the Rust core keeps its files, as a plain path.
export function coreHome(): string {
  const dir = new Directory(root(), "core");
  if (!dir.exists) dir.create({ intermediates: true, idempotent: true });
  return pathOf(dir.uri);
}

/// A `file://` URI as the path the core reads and writes.
export function pathOf(uri: string): string {
  return decodeURIComponent(uri.replace(/^file:\/\//, ""));
}

export function loadPrefs(): Prefs {
  try {
    const file = prefsFile();
    if (!file.exists) return {};
    return JSON.parse(file.textSync()) as Prefs;
  } catch {
    return {};
  }
}

export function savePrefs(prefs: Prefs) {
  try {
    prefsFile().write(JSON.stringify(prefs));
  } catch (error) {
    console.warn("saving prefs", error);
  }
}

export function wipePrefs() {
  try {
    const file = prefsFile();
    if (file.exists) file.delete();
  } catch {
    // Nothing to remove.
  }
}
