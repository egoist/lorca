// The app's commands, after the macOS app's menu bar: one table that becomes the window's native
// menu bar (whose items keep their titles and enabled state as the selection changes), the words
// and enabled state of the command palette's actions, and, in a browser tab where there is no menu
// bar, the shortcuts.

import { createEffect, createMemo, createSignal } from "solid-js";
import { hostInfo, inApp, menus, onMenuCommand, watchWindowState, type MenuItemSpec, type MenuItemState } from "../host";
import { L } from "../l10n";
import { isDM, isGroup } from "../model/models";
import { track } from "../model/reactive";
import { store } from "../model/store";
import { hasSheet, readHasSheet } from "./overlay";
import { botsAvailableToAdd, selection } from "./root";

export interface Command {
  id: string;
  title: () => string;
  /** "CmdOrCtrl+Shift+N": Ctrl on Windows and Linux. */
  accelerator?: string;
  /** A standard item the system handles: Cut, Copy, Quit. */
  role?: string;
  /** Brings the main window up first when chosen elsewhere. */
  opensMain?: boolean;
  enabled?: () => boolean;
  checked?: () => boolean;
}

const handlers = new Map<string, () => void>();

/** Whether this page's window is in full screen, which words the View menu's item. */
const [isFullScreen, setFullScreen] = createSignal(false);

/** What a sheet leaves working: the app's own commands, not the window's, as a sheet takes the
 * window's keys on the Mac. */
const whileSheet = new Set(["quit", "about", "help", "architecture", "checkForUpdates", "fullScreen", "simulateOffline", "replayMock", "showOnboarding"]);

/** Closes what floats over the window before a command acts on it, as the palette. */
let beforeCommand: (() => void) | undefined;

export function setBeforeCommand(run: () => void): void {
  beforeCommand = run;
}

function chatSelected(): boolean {
  const current = selection.read();
  return current?.kind === "chat" && store.chat(current.id) !== undefined;
}

function selectedChat() {
  const current = selection.read();
  return current?.kind === "chat" ? store.chat(current.id) : undefined;
}

/** Every command, in menu order. `enabled` and `title` read the store and the selection, so a
 * memo over them follows both. */
export const commandTable: Command[] = [
  { id: "newBot", title: () => L("New Bot…"), accelerator: "CmdOrCtrl+N", opensMain: true },
  { id: "newGroupChat", title: () => L("New Group Chat…"), accelerator: "CmdOrCtrl+Shift+N", opensMain: true },
  { id: "marketplace", title: () => L("Marketplace…"), accelerator: "CmdOrCtrl+Shift+M", opensMain: true },
  { id: "pairDevice", title: () => L("Pair a Device…"), accelerator: "CmdOrCtrl+Shift+P", opensMain: true },
  { id: "settings", title: () => L("Settings…"), accelerator: "CmdOrCtrl+," },
  { id: "closeWindow", title: () => L("Close Window"), accelerator: "CmdOrCtrl+W", role: "close" },
  { id: "quit", title: () => L("Quit %@", hostInfo().name), accelerator: "CmdOrCtrl+Q", role: "quit" },
  { id: "find", title: () => L("Find…"), accelerator: "CmdOrCtrl+F", opensMain: true },
  { id: "palette", title: () => L("Command Palette…"), accelerator: "CmdOrCtrl+K", opensMain: true },
  { id: "toggleSidebar", title: () => L("Toggle Sidebar"), accelerator: "CmdOrCtrl+B" },
  {
    id: "toggleInspector",
    title: () => L("Toggle Inspector"),
    accelerator: "CmdOrCtrl+Shift+B",
  },
  { id: "scrollToLatest", title: () => L("Scroll to Latest"), accelerator: "CmdOrCtrl+J", enabled: chatSelected },
  { id: "fullScreen", title: () => (isFullScreen() ? L("Exit Full Screen") : L("Enter Full Screen")), accelerator: "F11", role: "toggleFullScreen" },
  {
    id: "addBot",
    title: () => L("Add Bot…"),
    accelerator: "CmdOrCtrl+Alt+B",
    enabled: () => {
      const chat = selectedChat();
      track.roster();
      return !!chat && botsAvailableToAdd(chat.id).length > 0;
    },
  },
  {
    id: "renameChat",
    title: () => L("Rename Chat…"),
    accelerator: "CmdOrCtrl+R",
    enabled: () => {
      const chat = selectedChat();
      return !!chat && isGroup(chat);
    },
  },
  { id: "pinChat", title: () => L("Pin Chat"), accelerator: "CmdOrCtrl+P", enabled: chatSelected },
  { id: "stopResponding", title: () => L("Stop Responding"), accelerator: "CmdOrCtrl+.", enabled: chatSelected },
  {
    // The Mac's ⌃B has no counterpart here: Ctrl+B toggles the sidebar.
    id: "runInBackground",
    title: () => L("Run Command in Background"),
    enabled: () => {
      const chat = selectedChat();
      if (!chat) return false;
      track.chat(chat.id);
      return store.foregroundCommands(chat.id).length > 0;
    },
  },
  {
    id: "deleteChat",
    title: () => {
      const chat = selectedChat();
      return chat && isDM(chat) ? L("Delete Bot") : L("Delete Chat");
    },
    enabled: chatSelected,
  },
  { id: "help", title: () => L("Lorca Help"), accelerator: "F1" },
  { id: "architecture", title: () => L("Architecture Notes") },
  {
    id: "checkForUpdates",
    title: () => L("Check for Updates…"),
    enabled: () => hostInfo().updatesEnabled,
  },
  { id: "about", title: () => L("About %@", hostInfo().name) },
  {
    id: "simulateOffline",
    title: () => (store.isMock ? "Simulate CLI Offline" : "Reconnect to CLI"),
    accelerator: "CmdOrCtrl+Alt+D",
    checked: () => store.isMock && !store.isConnected,
  },
  { id: "replayMock", title: () => "Replay Mock Data", enabled: () => store.isMock },
  { id: "showOnboarding", title: () => "Show Onboarding" },
];

const byID = new Map(commandTable.map((command) => [command.id, command]));

export const commands = {
  /** The window sets what each command does. */
  register(table: Record<string, () => void>): void {
    for (const [id, run] of Object.entries(table)) handlers.set(id, run);
  },
  run(id: string): void {
    const command = byID.get(id);
    if (command?.enabled && !command.enabled()) return;
    if (hasSheet() && !whileSheet.has(id)) return;
    // The palette's own shortcut opens it afresh; any other command closes it first.
    if (id !== "palette") beforeCommand?.();
    handlers.get(id)?.();
  },
  /** Whether the page answers `id` itself. */
  has(id: string): boolean {
    return handlers.has(id);
  },
  get(id: string): Command | undefined {
    return byID.get(id);
  },
};

/** How a shortcut reads here: "Ctrl+Shift+N" on Windows and Linux, "⇧⌘N" on a Mac. */
export function shortcutText(accelerator: string): string {
  const parts = accelerator.split("+");
  const key = parts.pop() ?? "";
  if (hostInfo().platform === "darwin") {
    const glyphs: Record<string, string> = { Ctrl: "⌃", Alt: "⌥", Shift: "⇧", CmdOrCtrl: "⌘", Cmd: "⌘" };
    const order = ["Ctrl", "Alt", "Shift", "CmdOrCtrl", "Cmd"];
    const keys: Record<string, string> = { Enter: "Return" };
    return order.filter((modifier) => parts.includes(modifier)).map((modifier) => glyphs[modifier]).join("") + (keys[key] ?? key.toUpperCase());
  }
  const names: Record<string, string> = { CmdOrCtrl: "Ctrl", Cmd: "Win" };
  return [...parts.map((modifier) => names[modifier] ?? modifier), key.length === 1 ? key.toUpperCase() : key].join("+");
}

// MARK: - The menu bar

function item(id: string, window: "main" | "other"): MenuItemSpec {
  const command = byID.get(id)!;
  const enabled =
    window === "main"
      ? (command.enabled?.() ?? true) && (!readHasSheet() || whileSheet.has(id))
      : command.role !== undefined || (["settings", "help", "architecture", "about", "checkForUpdates", "showOnboarding", "simulateOffline", "replayMock"].includes(id) && (command.enabled?.() ?? true));
  return {
    id,
    label: command.title(),
    accelerator: command.accelerator,
    role: command.role,
    opensMain: command.opensMain,
    disabled: !enabled,
    type: command.checked ? "checkbox" : undefined,
    checked: command.checked?.(),
  };
}

const separator: MenuItemSpec = { type: "separator" };

/** The menu bar, in the order the macOS app's has it; what belongs to the app menu there (Settings,
 * Quit, About, Check for Updates) sits in File and Help, as on Windows and Linux. */
function menuBar(window: "main" | "other"): MenuItemSpec[] {
  const at = (id: string) => item(id, window);
  const updates = hostInfo().updatesEnabled ? [separator, at("checkForUpdates")] : [];
  return [
    {
      label: L("File"),
      submenu: [at("newBot"), at("newGroupChat"), separator, at("marketplace"), separator, at("pairDevice"), separator, at("settings"), separator, at("closeWindow"), at("quit")],
    },
    {
      label: L("Edit"),
      submenu: [
        { role: "undo", label: L("Undo") },
        { role: "redo", label: L("Redo") },
        separator,
        { role: "cut", label: L("Cut") },
        { role: "copy", label: L("Copy") },
        { role: "paste", label: L("Paste") },
        { role: "pasteAndMatchStyle", label: L("Paste and Match Style"), accelerator: "CmdOrCtrl+Alt+Shift+V" },
        { role: "selectAll", label: L("Select All") },
        separator,
        at("find"),
      ],
    },
    {
      label: L("View"),
      submenu: [at("palette"), separator, at("toggleSidebar"), at("toggleInspector"), separator, at("scrollToLatest"), separator, at("fullScreen")],
    },
    { label: L("Chat"), submenu: [at("addBot"), at("renameChat"), at("pinChat"), separator, at("stopResponding"), at("runInBackground"), separator, at("deleteChat")] },
    { label: L("Window"), submenu: [{ role: "minimize", label: L("Minimize") }, { role: "zoom", label: L("Zoom") }] },
    { label: "Debug", submenu: [at("simulateOffline"), at("replayMock"), separator, at("showOnboarding")] },
    { label: L("Help"), submenu: [at("help"), at("architecture"), ...updates, separator, at("about")] },
  ];
}

function flatten(items: MenuItemSpec[]): MenuItemSpec[] {
  return items.flatMap((entry) => (entry.submenu ? flatten(entry.submenu) : entry.id ? [entry] : []));
}

/** Puts this window's menu bar up and keeps its items' titles, checks, and enabled states in step
 * with the store and the selection. Clicks come back as commands. A browser tab, with no menu bar,
 * answers the shortcuts itself. */
export function installMenuBar(window: "main" | "other"): () => void {
  // Only a window has full screen; a browser tab stays out of it.
  const offState = inApp ? watchWindowState((state) => setFullScreen(state.fullScreen)) : () => {};
  const offMenu = installMenus(window);
  return () => {
    offState();
    offMenu();
  };
}

function installMenus(window: "main" | "other"): () => void {
  if (!inApp) return installShortcuts();
  void menus.setBar(menuBar(window));
  const states = createMemo(() => {
    track.any();
    selection.read();
    return flatten(menuBar(window)).map((entry) => ({ id: entry.id!, label: entry.label, disabled: !!entry.disabled, checked: !!entry.checked }));
  });
  let sent = new Map<string, MenuItemState>();
  createEffect(states, (next) => {
    const changed: MenuItemState[] = [];
    const now = new Map<string, MenuItemState>();
    for (const state of next) {
      now.set(state.id, state);
      const before = sent.get(state.id);
      if (!before || before.label !== state.label || before.disabled !== state.disabled || before.checked !== state.checked) changed.push(state);
    }
    sent = now;
    if (changed.length > 0) void menus.update(changed);
  });
  const offCommand = onMenuCommand((id) => commands.run(id));
  const offKeys = installChatNumberKeys();
  return () => {
    offCommand();
    offKeys();
  };
}

/** Ctrl+1 to Ctrl+9 open the first nine chats. The menu bar keeps no hidden items on Windows, so
 * the page answers these keys itself. */
function installChatNumberKeys(): () => void {
  const onKey = (event: KeyboardEvent) => {
    const primary = hostInfo().platform === "darwin" ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
    if (!primary || event.shiftKey || event.altKey || hasSheet()) return;
    // The digit keys by where they sit, so a layout that types other characters on them (AZERTY)
    // still reaches the chats.
    const digit = /^Digit([1-9])$/.exec(event.code)?.[1];
    if (digit) {
      event.preventDefault();
      goToChat?.(Number(digit));
    }
  };
  window.addEventListener("keydown", onKey);
  return () => window.removeEventListener("keydown", onKey);
}

let goToChat: ((number: number) => void) | undefined;

export function setGoToChat(run: (number: number) => void): void {
  goToChat = run;
}

/** In a browser tab, with no menu bar, the page answers the menu's shortcuts itself; the system's
 * items (Quit, Close Window, Full Screen) are the browser's. */
function installShortcuts(): () => void {
  const onKey = (event: KeyboardEvent) => {
    // AltGr types characters ("{" is AltGr+B on some layouts); Windows reports it as Ctrl+Alt.
    if (event.defaultPrevented || event.getModifierState("AltGraph")) return;
    for (const command of commandTable) {
      if (!command.accelerator || (command.role && !commands.has(command.id)) || !matches(command.accelerator, event)) continue;
      event.preventDefault();
      commands.run(command.id);
      return;
    }
  };
  window.addEventListener("keydown", onKey);
  const offKeys = installChatNumberKeys();
  return () => {
    window.removeEventListener("keydown", onKey);
    offKeys();
  };
}

function matches(accelerator: string, event: KeyboardEvent): boolean {
  const parts = accelerator.split("+");
  const key = parts.pop()!;
  const mac = hostInfo().platform === "darwin";
  const wantsPrimary = parts.includes("CmdOrCtrl");
  const primary = mac ? event.metaKey : event.ctrlKey;
  if (wantsPrimary !== primary) return false;
  if (parts.includes("Shift") !== event.shiftKey) return false;
  if (parts.includes("Alt") !== event.altKey) return false;
  if (mac && parts.includes("Ctrl") !== event.ctrlKey) return false;
  const pressed = event.key.length === 1 ? event.key.toUpperCase() : event.key;
  const code = event.code.startsWith("Key") ? event.code.slice(3) : event.code.startsWith("Digit") ? event.code.slice(5) : null;
  return pressed === key.toUpperCase() || code === key.toUpperCase() || (key === "," && event.code === "Comma") || (key === "." && event.code === "Period");
}
