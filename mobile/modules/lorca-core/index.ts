// The phone's door to the Rust core: start it once, then the JSON API the desktop app speaks
// to the CLI (`request`) and the events the CLI would push over the websocket (`onEvent`).
import { requireNativeModule, type EventSubscription } from "expo-modules-core";

interface Native {
  start(home: string, name: string, os: string, osVersion: string, model: string): Promise<void>;
  request(method: string, params: string): Promise<string>;
  wake(): void;
  /// iOS only.
  previewFile?(path: string): Promise<void>;
  /// Android only.
  setOpenChat?(chatId: string | null): void;
  /// Android only.
  installUpdate?(url: string, sha256: string, size: number): Promise<void>;
  addListener(event: "event", listener: (payload: { json: string }) => void): EventSubscription;
  addListener(event: "update", listener: (status: InstallStatus) => void): EventSubscription;
}

const native = requireNativeModule<Native>("LorcaCore");

export interface HostFacts {
  name: string;
  os: string;
  os_version: string;
  model: string;
}

/// One event frame, as the websocket carries it.
export interface Frame {
  event: string;
  data: unknown;
}

/// Opens and loads the account off the JS thread; requests wait for it.
export function start(home: string, facts: HostFacts): Promise<void> {
  return native.start(home, facts.name, facts.os, facts.os_version, facts.model);
}

/// One API call. Throws with the core's message when it answers with an error.
export async function request<T = unknown>(method: string, params: unknown = {}): Promise<T> {
  const raw = await native.request(method, JSON.stringify(params ?? {}));
  const parsed = JSON.parse(raw) as { result?: T; error?: { message: string } };
  if (parsed.error) throw new Error(parsed.error.message);
  return parsed.result as T;
}

export function onEvent(listener: (frame: Frame) => void): () => void {
  const subscription = native.addListener("event", ({ json }) => listener(JSON.parse(json) as Frame));
  return () => subscription.remove();
}

/// iOS: the file in Quick Look, over the app. Android has none; the file goes to the share sheet.
export const canPreviewFiles = !!native.previewFile;

export function previewFile(path: string): Promise<void> {
  return native.previewFile ? native.previewFile(path) : Promise.resolve();
}

/// The app came to the foreground: sync now rather than after the backoff.
export function wake() {
  native.wake();
}

/// Android: the chat on screen, or null. While the app is in front, the native push service
/// posts nothing for it and clears what it posted for it. iOS asks the notification handler.
export function setOpenChat(chatId: string | null) {
  native.setOpenChat?.(chatId);
}

/// Where installing a release stands, as the Android module reports it.
export type InstallStatus =
  | { state: "downloading"; received: number; total: number }
  | { state: "installing" }
  | { state: "confirming" }
  | { state: "cancelled" }
  | { state: "failed"; message: string };

/// Android: downloads a release's APK, checks its size and SHA-256, and hands it to the package
/// installer, which replaces the running app with it. Resolves once the installer has it; the
/// download's progress and how the install ends come to `onInstallStatus`.
export function installUpdate(url: string, sha256: string, size: number): Promise<void> {
  if (!native.installUpdate) return Promise.reject(new Error("This build cannot install updates"));
  return native.installUpdate(url, sha256, size);
}

export function onInstallStatus(listener: (status: InstallStatus) => void): () => void {
  const subscription = native.addListener("update", listener);
  return () => subscription.remove();
}
