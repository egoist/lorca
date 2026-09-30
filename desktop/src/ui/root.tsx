// The main window's state and commands, after the macOS app's RootSplitViewController: what is
// selected (a chat, or a settings pane, which puts the settings sidebar in the chats' place), the
// panes visited since Settings opened, the Device the Device panes show, the inspector and the
// sidebar, and the chat commands of the menus. The selection is the window's route (`/chat/:id`,
// `/settings/:pane`); `selection` mirrors it for code that reads it right after changing it.

import { preferences, setPreferences, host } from "../host";
import { L } from "../l10n";
import {
  canAddBot,
  decodeSelection,
  encodeSelection,
  isDM,
  isGroup,
  sameSelection,
  settingsPanes,
  type Selection,
  type SettingsPane,
} from "../model/models";
import { store } from "../model/store";
import { box } from "./box";
import { TextField } from "./controls";
import { alert } from "./overlay";
import type { SettingsEntry } from "./settings/search";

export const selection = box<Selection | null>(null, sameSelection);
/** Moves the window's route; the main window attaches the router's. */
let navigate: ((path: string) => void) | null = null;
/** The Device the Plugins, Bots, and Devices panes show: this computer until another is picked. */
export const settingsDeviceID = box<string | null>(null);
/** The panes visited since Settings opened, for the toolbar's back and forward. */
const paneHistory = box<SettingsPane[]>([]);
const paneIndex = box(0);
/** Whether the user wants the inspector beside a chat. */
export const userWantsInspector = box(true);
export const sidebarCollapsed = box(false);
export const sidebarWidth = box(260);
export const inspectorWidth = box(280);
/** A setting a search picked, for its pane to scroll to and flash. */
export const revealed = box<{ entry: SettingsEntry; at: number } | null>(null);

let lastChatID: string | null = null;
let walkingHistory = false;
let focusBeforeSettings: Element | null = null;

/** What the chat view registers, for the commands that reach into it. */
export const chatActions: {
  focusComposer?: () => void;
  prefill?: (text: string) => void;
  scrollToLatest?: () => void;
} = {};

/** What the sidebars register, for Find and the keyboard. */
export const sidebarActions: {
  focusSettingsSearch?: () => void;
  focusSettingsList?: () => void;
} = {};

export function loadWindowState(): void {
  const prefs = preferences();
  userWantsInspector.set(prefs.showsInspector);
  sidebarCollapsed.set(prefs.sidebarCollapsed);
  if (prefs.sidebarWidth) sidebarWidth.set(clamp(prefs.sidebarWidth, 232, 340));
  if (prefs.inspectorWidth) inspectorWidth.set(clamp(prefs.inspectorWidth, 268, 320));
}

export function clamp(value: number, low: number, high: number): number {
  return Math.min(high, Math.max(low, value));
}

export const canGoBack = () => paneIndex.read() > 0;
export const canGoForward = () => paneIndex.read() < paneHistory.read().length - 1;

export function goBack(): void {
  walkHistory(paneIndex.get() - 1);
}

export function goForward(): void {
  walkHistory(paneIndex.get() + 1);
}

function walkHistory(index: number): void {
  const pane = paneHistory.get()[index];
  if (!pane) return;
  paneIndex.set(index);
  walkingHistory = true;
  select({ kind: "settings", pane });
  walkingHistory = false;
}

function recordHistory(current: Selection | null): void {
  if (current?.kind !== "settings") {
    paneHistory.set([]);
    paneIndex.set(0);
    return;
  }
  if (walkingHistory) return;
  const kept = paneHistory.get().slice(0, paneIndex.get() + 1);
  if (kept[kept.length - 1] !== current.pane) kept.push(current.pane);
  paneHistory.set(kept);
  paneIndex.set(kept.length - 1);
}

/** The route that shows `value`. */
export function pathFor(value: Selection | null): string {
  if (value?.kind === "chat") return `/chat/${encodeURIComponent(value.id)}`;
  if (value?.kind === "settings") return `/settings/${value.pane}`;
  return "/";
}

/** What a route of the main window shows. */
export function selectionAt(pathname: string): Selection | null {
  const [, kind, value] = pathname.split("/");
  if (kind === "chat" && value) return { kind: "chat", id: decodeURIComponent(value) };
  const pane = settingsPanes.find((each) => each === value);
  if (kind === "settings" && pane) return { kind: "settings", pane };
  return null;
}

/** The main window hands over its router's navigation. Each selection replaces the route rather
 * than adding a history entry: back and forward walk the settings panes, as in the macOS app. */
export function attachNavigator(navigator: ((path: string) => void) | null): void {
  navigate = navigator;
}

function setSelection(next: Selection | null): void {
  const old = selection.get();
  if (sameSelection(old, next)) return;
  selection.set(next);
  recordHistory(next);
  if (old?.kind === "chat") lastChatID = old.id;
  if (!sameSelection(selectionAt(location.pathname), next)) navigate?.(pathFor(next));
  void setPreferences({ selection: encodeSelection(next) });
}

/** The route changed under the window, as on load: the selection follows it. */
export function followRoute(pathname: string): void {
  const next = selectionAt(pathname);
  if (sameSelection(selection.get(), next)) return;
  select(next);
}

export function select(next: Selection | null): void {
  const old = selection.get();
  if (next?.kind === "settings" && old?.kind !== "settings") focusBeforeSettings = document.activeElement;
  setSelection(next);
  if (next?.kind === "chat") store.markRead(next.id);
  // Leaving Settings, the keyboard goes back to whatever had it.
  if (old?.kind === "settings" && next?.kind !== "settings") {
    const held = focusBeforeSettings;
    focusBeforeSettings = null;
    queueMicrotask(() => {
      if (held instanceof HTMLElement && held.isConnected) held.focus();
      else chatActions.focusComposer?.();
    });
  } else if (next?.kind === "settings" && old?.kind !== "settings") {
    queueMicrotask(() => sidebarActions.focusSettingsList?.());
  }
}

export function selectedChatID(): string | null {
  const current = selection.get();
  return current?.kind === "chat" ? current.id : null;
}

/** Brings the selection back after a snapshot: the saved one when it still exists, else the
 * first chat. */
export function restoreSelection(): void {
  const saved = decodeSelection(preferences().selection);
  const current = selection.get();
  const exists = (candidate: Selection | null) => candidate !== null && (candidate.kind === "settings" || store.chat(candidate.id) !== undefined);
  if (exists(current)) return;
  const restored = exists(saved) ? saved : store.chats[0] ? ({ kind: "chat", id: store.chats[0].id } as Selection) : null;
  setSelection(restored);
}

/** Settings lives in this window: its panes take the content area, and the sidebar lists them in
 * place of the chats. */
export function showSettings(pane?: SettingsPane): void {
  if (sidebarCollapsed.get()) setSidebarCollapsed(false);
  if (pane) {
    select({ kind: "settings", pane });
    return;
  }
  if (selection.get()?.kind === "settings") return;
  select({ kind: "settings", pane: "general" });
}

/** Opens a setting's pane, scrolls to its row, and flashes it. */
export function reveal(entry: SettingsEntry): void {
  showSettings(entry.pane);
  revealed.set({ entry, at: Date.now() });
}

/** The Devices pane on that Device, with the other Device panes on it too. */
export function openDevice(id: string): void {
  showSettingsDevice(id);
  if (sidebarCollapsed.get()) setSidebarCollapsed(false);
  select({ kind: "settings", pane: "device" });
}

export function showSettingsDevice(id: string | null): void {
  settingsDeviceID.set((id && store.device(id)?.id) || store.thisDevice?.id || null);
}

export function closeSettings(): void {
  const chat = (lastChatID && store.chat(lastChatID)) || store.chats[0];
  select(chat ? { kind: "chat", id: chat.id } : null);
}

export function setSidebarCollapsed(collapsed: boolean): void {
  sidebarCollapsed.set(collapsed);
  void setPreferences({ sidebarCollapsed: collapsed });
}

export function toggleSidebar(): void {
  setSidebarCollapsed(!sidebarCollapsed.get());
}

export function toggleInspector(): void {
  if (selection.get()?.kind !== "chat" || !store.isConnected) {
    void host.beep();
    return;
  }
  const wants = !userWantsInspector.get();
  userWantsInspector.set(wants);
  void setPreferences({ showsInspector: wants });
}

export function open(chatID: string): void {
  select({ kind: "chat", id: chatID });
  queueMicrotask(() => chatActions.focusComposer?.());
}

/** Bots that could still join a chat. Empty for a DM, a full group, or when every bot is in it. */
export function botsAvailableToAdd(chatID: string) {
  const chat = store.chat(chatID);
  if (!chat || !canAddBot(chat)) return [];
  return store.bots.filter((bot) => !chat.botIDs.includes(bot.id));
}

export async function renameChat(): Promise<void> {
  const chatID = selectedChatID();
  const chat = chatID ? store.chat(chatID) : undefined;
  if (!chatID || !chat || !isGroup(chat)) {
    void host.beep();
    return;
  }
  let value = store.title(chat);
  const answer = await alert({
    message: L("Rename Chat"),
    informative: L("Chat names live inside the encrypted roster blob, never on the relay."),
    buttons: [{ title: L("Rename") }, { title: L("Cancel") }],
    accessory: () => <TextField value={value} placeholder={L("Chat name")} autofocus onInput={(next) => (value = next)} />,
  });
  if (answer === 0) store.rename(chatID, value);
}

export function togglePinChat(): void {
  const chatID = selectedChatID();
  if (chatID) store.togglePin(chatID);
}

export async function deleteChat(): Promise<void> {
  const chatID = selectedChatID();
  const chat = chatID ? store.chat(chatID) : undefined;
  if (!chatID || !chat) {
    void host.beep();
    return;
  }
  const deletesBot = isDM(chat) && chat.botIDs[0] !== undefined && store.bot(chat.botIDs[0]) !== undefined;
  const answer = await alert({
    message: L('Delete "%@"?', store.title(chat)),
    informative: deletesBot
      ? L("The bot, its routines, and this direct chat are removed from this Device and from paired Devices.")
      : L("The transcript is removed from this Device and from paired Devices."),
    style: "warning",
    buttons: [{ title: deletesBot ? L("Delete Bot") : L("Delete"), destructive: true }, { title: L("Cancel") }],
  });
  if (answer === 0) store.deleteChat(chatID);
}

/** The keeper of the window's reaction to the store: the Device picker falls back to this computer
 * when its Device leaves, and a chat that goes away gives the selection to the first chat. */
export function followStore(): () => void {
  return store.subscribe((event) => {
    switch (event.kind) {
      case "snapshotReplaced":
        showSettingsDevice(settingsDeviceID.get());
        restoreSelection();
        break;
      case "chatsChanged": {
        const current = selection.get();
        if (current?.kind === "chat" && !store.chat(current.id)) {
          select(store.chats[0] ? { kind: "chat", id: store.chats[0].id } : null);
        }
        break;
      }
      case "rosterChanged":
        showSettingsDevice(settingsDeviceID.get());
        break;
    }
  });
}
