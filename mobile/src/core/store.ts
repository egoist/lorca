// The phone's plaintext store: roster, chats, presence, sync cursor, outbox, and the working
// state the transcript shows. Pure data and reducers; behavior (relay sync, sending, rooms)
// lives in engine.ts.

import { create } from "zustand";
import { useShallow } from "zustand/react/shallow";
import { nowUnix } from "./bytes";
import type { MachineFile } from "./keys";
import { ONLINE_WINDOW_SECS, type Bot, type Chat, type ChatMeta, type Device, type Message, type RosterBlob } from "./model";
import { emptyState, loadMachineFile, loadState, saveState, type OutboxItem, type State } from "./storage";

export interface Running {
  chatId: string;
  /// Empty while a group exchange is between member turns.
  botId: string;
}

export interface StoreState extends State {
  ready: boolean;
  machineFile: MachineFile | null;
  relayConnected: boolean;
  /// Turns in flight that this Device asked for, by job id.
  running: Record<string, Running>;
  /// "Chef stopped without replying", by chat id, after a turn ends with nothing said.
  statuses: Record<string, string>;
  /// The chat on screen: new replies there do not count as unread.
  openChatId: string | null;
  /// Attachment id → file URI, for the bytes this phone has.
  files: Record<string, string>;
}

export const useStore = create<StoreState>()(() => ({
  ...emptyState(),
  ready: false,
  machineFile: null,
  relayConnected: false,
  running: {},
  statuses: {},
  openChatId: null,
  files: {},
}));

const DATA_KEYS: (keyof State)[] = ["devices", "bots", "chats", "last_seq", "applied_blob_ids", "outbox", "device_seen", "machine_blob_hash", "dictation_lang"];

function persist() {
  const s = useStore.getState();
  const data = {} as State;
  for (const key of DATA_KEYS) (data as any)[key] = s[key];
  saveState(data);
}

/// Applies a data change and schedules a save.
export function mutate(update: (s: StoreState) => Partial<StoreState>) {
  useStore.setState((s) => update(s));
  persist();
}

export async function loadStore() {
  const machineFile = await loadMachineFile();
  const data = machineFile ? loadState() : emptyState();
  useStore.setState({ ...data, machineFile, ready: true });
}

export function resetStore(machineFile: MachineFile | null) {
  useStore.setState({ ...emptyState(), machineFile, running: {}, statuses: {}, relayConnected: false, files: {} });
  persist();
}

export function markFile(id: string, uri: string) {
  useStore.setState((s) => (s.files[id] === uri ? s : { files: { ...s.files, [id]: uri } }));
}

// MARK: - Lookup

export function thisDeviceId(): string | null {
  return useStore.getState().devices.find((d) => d.id === machinePubkey())?.id ?? machinePubkey();
}

let cachedPubkey: string | null = null;
export function setMachinePubkey(pubkey: string | null) {
  cachedPubkey = pubkey;
}
function machinePubkey(): string | null {
  return cachedPubkey;
}

export function botById(id: string): Bot | undefined {
  return useStore.getState().bots.find((b) => b.id === id);
}

export function chatById(id: string): Chat | undefined {
  return useStore.getState().chats.find((c) => c.id === id);
}

export function deviceById(id: string): Device | undefined {
  return useStore.getState().devices.find((d) => d.id === id);
}

export function deviceIsOnline(id: string, s: StoreState = useStore.getState()): boolean {
  if (id === cachedPubkey) return true;
  const seen = s.device_seen[id];
  return seen !== undefined && nowUnix() - seen < ONLINE_WINDOW_SECS;
}

// MARK: - Reducers

export function upsertMessage(message: Message): { added: boolean; changed: boolean } {
  let added = false;
  let changed = false;
  mutate((s) => {
    let chats = s.chats;
    if (!chats.some((c) => c.id === message.chat_id)) {
      // Roster not here yet: keep the message under a placeholder until it is.
      chats = [
        ...chats,
        { id: message.chat_id, kind: "group", title: "Chat", bot_ids: [], is_pinned: false, created_at: message.created_at, messages: [], unread_count: 0 },
      ];
    }
    chats = chats.map((chat) => {
      if (chat.id !== message.chat_id) return chat;
      const index = chat.messages.findIndex((m) => m.id === message.id);
      if (index < 0) {
        added = true;
        changed = true;
        return { ...chat, messages: [...chat.messages, message] };
      }
      const existing = chat.messages[index];
      if (JSON.stringify(existing) === JSON.stringify(message)) return chat;
      changed = true;
      const messages = chat.messages.slice();
      messages[index] = message;
      return { ...chat, messages };
    });
    return { chats };
  });
  return { added, changed };
}

export function removeMessage(chatId: string, messageId: string) {
  mutate((s) => ({
    chats: s.chats.map((chat) => (chat.id === chatId ? { ...chat, messages: chat.messages.filter((m) => m.id !== messageId) } : chat)),
  }));
}

export function bumpUnread(chatId: string) {
  mutate((s) => ({ chats: s.chats.map((chat) => (chat.id === chatId ? { ...chat, unread_count: chat.unread_count + 1 } : chat)) }));
}

export function markRead(chatId: string) {
  const chat = chatById(chatId);
  if (!chat || chat.unread_count === 0) return;
  mutate((s) => ({ chats: s.chats.map((c) => (c.id === chatId ? { ...c, unread_count: 0 } : c)) }));
}

/// Whole-roster snapshot from the relay: bots replace, chat metadata merges over kept messages.
export function applyRoster(roster: RosterBlob): { removed: string[] } {
  const removed: string[] = [];
  mutate((s) => {
    const incoming = new Set(roster.chats.map((c) => c.id));
    for (const chat of s.chats) if (!incoming.has(chat.id)) removed.push(chat.id);
    const existing = new Map(s.chats.map((c) => [c.id, c]));
    const chats: Chat[] = roster.chats.map((meta) => {
      const old = existing.get(meta.id);
      return { ...meta, is_pinned: meta.is_pinned ?? false, messages: old?.messages ?? [], unread_count: old?.unread_count ?? 0 };
    });
    return { bots: roster.bots, chats };
  });
  return { removed };
}

export function upsertDevice(device: Device): boolean {
  const s = useStore.getState();
  const index = s.devices.findIndex((d) => d.id === device.id);
  if (index >= 0 && JSON.stringify(s.devices[index]) === JSON.stringify(device)) return false;
  mutate((s) => {
    const devices = s.devices.slice();
    const at = devices.findIndex((d) => d.id === device.id);
    if (at < 0) devices.push(device);
    else devices[at] = device;
    return { devices };
  });
  return true;
}

/// Presence from the relay's machine list. A machine the roster names as a Runner but whose
/// `machine` blob has not arrived gets a placeholder with its box key, so a job can still be
/// sealed to it; the blob fills in its name and OS when it comes.
export function setDeviceSeen(seen: Record<string, number>, boxKeys: Record<string, string>) {
  const s = useStore.getState();
  const seenChanged = Object.entries(seen).some(([id, at]) => s.device_seen[id] !== at);
  const keysChanged = s.devices.some((d) => d.box_pubkey === "" && boxKeys[d.id]);
  const known = new Set(s.devices.map((d) => d.id));
  const missing = s.bots.map((b) => b.runner_id).filter((id, i, all) => !known.has(id) && boxKeys[id] && all.indexOf(id) === i);
  if (!seenChanged && !keysChanged && missing.length === 0) return;
  mutate((s) => ({
    device_seen: { ...s.device_seen, ...seen },
    devices: [
      ...(keysChanged ? s.devices.map((d) => (d.box_pubkey === "" && boxKeys[d.id] ? { ...d, box_pubkey: boxKeys[d.id] } : d)) : s.devices),
      ...missing.map((id) => ({ id, name: "Runner", model: "", os: "macos", os_version: "", box_pubkey: boxKeys[id], providers_connected: [], updated_at: 0 })),
    ],
  }));
}

export function enqueue(item: OutboxItem) {
  mutate((s) => ({ outbox: [...s.outbox, item], applied_blob_ids: remember(s.applied_blob_ids, item.id) }));
}

export function dequeue(id: string) {
  mutate((s) => ({ outbox: s.outbox.filter((i) => i.id !== id) }));
}

export function wasApplied(id: string): boolean {
  return useStore.getState().applied_blob_ids.includes(id);
}

export function rememberApplied(id: string) {
  mutate((s) => ({ applied_blob_ids: remember(s.applied_blob_ids, id) }));
}

function remember(ids: string[], id: string): string[] {
  const next = [...ids, id];
  return next.length > 2000 ? next.slice(next.length - 2000) : next;
}

export function setLastSeq(seq: number) {
  mutate((s) => ({ last_seq: Math.max(s.last_seq, seq) }));
}

export function updateChatMeta(chatId: string, update: (meta: ChatMeta) => ChatMeta) {
  mutate((s) => ({ chats: s.chats.map((chat) => (chat.id === chatId ? { ...chat, ...update(chat) } : chat)) }));
}

export function setRunning(jobId: string, running: Running | null) {
  useStore.setState((s) => {
    const next = { ...s.running };
    if (running) next[jobId] = running;
    else delete next[jobId];
    return { running: next };
  });
}

export function setStatus(chatId: string, text: string | null) {
  useStore.setState((s) => {
    const next = { ...s.statuses };
    if (text) next[chatId] = text;
    else delete next[chatId];
    return { statuses: next };
  });
}

// MARK: - Hooks

export function useChat(id: string): Chat | undefined {
  return useStore((s) => s.chats.find((c) => c.id === id));
}

export function useBots(): Bot[] {
  return useStore((s) => s.bots);
}

export function useBotMap(): Map<string, Bot> {
  const bots = useStore((s) => s.bots);
  return new Map(bots.map((b) => [b.id, b]));
}

/// Bots with a turn running in this chat, in roster order; a room between turns yields none.
export function useWorkingBots(chatId: string): string[] {
  return useStore(
    useShallow((s) => {
      const ids = new Set<string>();
      for (const r of Object.values(s.running)) if (r.chatId === chatId && r.botId) ids.add(r.botId);
      return s.bots.filter((b) => ids.has(b.id)).map((b) => b.id);
    }),
  );
}

export function useIsWorking(chatId: string): boolean {
  return useStore((s) => Object.values(s.running).some((r) => r.chatId === chatId));
}

export function useWorkingBotIds(): Set<string> {
  return useStore(useShallow((s) => new Set(Object.values(s.running).map((r) => r.botId).filter(Boolean))));
}
