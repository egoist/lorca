// Where the phone keeps things. The machine file (secret, account key) lives in the secure
// store; the plaintext state (roster, chats, sync cursor, outbox) is one JSON file in the app's
// documents directory, like the CLI's state.json.

import { Directory, File, Paths } from "expo-file-system";
import * as SecureStore from "expo-secure-store";
import type { MachineFile } from "./keys";
import type { Bot, Chat, Device } from "./model";

const MACHINE_KEY = "tinybot.machine";

export async function loadMachineFile(): Promise<MachineFile | null> {
  try {
    const text = await SecureStore.getItemAsync(MACHINE_KEY);
    return text ? (JSON.parse(text) as MachineFile) : null;
  } catch {
    return null;
  }
}

export async function saveMachineFile(file: MachineFile | null): Promise<void> {
  if (file) {
    await SecureStore.setItemAsync(MACHINE_KEY, JSON.stringify(file), {
      keychainAccessible: SecureStore.AFTER_FIRST_UNLOCK_THIS_DEVICE_ONLY,
    });
  } else {
    await SecureStore.deleteItemAsync(MACHINE_KEY);
  }
}

export interface OutboxItem {
  id: string;
  kind: string;
  recipient: string | null;
  /// base64url, or empty when the ciphertext sits in `ciphertext_file` (a `file` blob is too
  /// big to keep inside state.json).
  ciphertext: string;
  ciphertext_file?: string;
}

export interface State {
  devices: Device[];
  bots: Bot[];
  chats: Chat[];
  /// Last relay sequence applied.
  last_seq: number;
  applied_blob_ids: string[];
  outbox: OutboxItem[];
  /// Machine id → last seen (unix seconds), from relay presence.
  device_seen: Record<string, number>;
  machine_blob_hash?: string;
  /// Speech recognizer language tag; unset follows the phone's preferred languages.
  dictation_lang?: string;
}

export function emptyState(): State {
  return { devices: [], bots: [], chats: [], last_seq: 0, applied_blob_ids: [], outbox: [], device_seen: {} };
}

function root(): Directory {
  const dir = new Directory(Paths.document, "tinybot");
  if (!dir.exists) dir.create({ intermediates: true, idempotent: true });
  return dir;
}

function stateFile(): File {
  return new File(root(), "state.json");
}

function subdir(name: string): Directory {
  const dir = new Directory(root(), name);
  if (!dir.exists) dir.create({ intermediates: true, idempotent: true });
  return dir;
}

// MARK: - Attachments

/// The attachment's bytes, by id, once sent from here or fetched from the relay.
export function fileFor(id: string): File {
  return new File(subdir("files"), id);
}

export function hasFile(id: string): boolean {
  try {
    return fileFor(id).exists;
  } catch {
    return false;
  }
}

export function writeFile(id: string, bytes: Uint8Array) {
  fileFor(id).write(bytes);
}

/// Ciphertext waiting in the outbox, kept beside state.json rather than inside it.
export function writeOutboxCiphertext(id: string, b64: string): string {
  const file = new File(subdir("outbox"), id);
  file.write(b64);
  return file.uri;
}

export function readOutboxCiphertext(uri: string): string {
  return new File(uri).textSync();
}

export function deleteOutboxCiphertext(uri: string) {
  try {
    const file = new File(uri);
    if (file.exists) file.delete();
  } catch {
    // Already gone.
  }
}

export function loadState(): State {
  try {
    const file = stateFile();
    if (!file.exists) return emptyState();
    const parsed = JSON.parse(file.textSync()) as Partial<State>;
    return { ...emptyState(), ...parsed };
  } catch {
    return emptyState();
  }
}

let pending: ReturnType<typeof setTimeout> | null = null;
let latest: State | null = null;

/// Coalesces writes: the transcript changes many times a second while a reply grows.
export function saveState(state: State) {
  latest = state;
  if (pending) return;
  pending = setTimeout(() => {
    pending = null;
    flushState();
  }, 150);
}

export function flushState() {
  if (!latest) return;
  try {
    stateFile().write(JSON.stringify(latest));
  } catch (error) {
    console.warn("saving state", error);
  }
}

export function wipeState() {
  latest = null;
  if (pending) {
    clearTimeout(pending);
    pending = null;
  }
  try {
    const dir = new Directory(Paths.document, "tinybot");
    if (dir.exists) dir.delete();
  } catch {
    // Nothing to remove.
  }
}
