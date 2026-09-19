// The phone's view of the account: what the core last said. Pure data and reducers fed by the
// core's events (engine.ts applies them), the way the Mac app's AppStore mirrors the CLI. The
// core keeps the truth on disk; nothing here is persisted except the phone's own prefs.

import { useMemo } from "react";
import { create } from "zustand";
import { useShallow } from "zustand/react/shallow";
import type { AutoReview, Bot, Chat, ChatMeta, ChatUsage, Device, Message, ProviderStatus, Routine } from "./model";
import { savePrefs } from "./prefs";

export interface Running {
  chatId: string;
  /// Empty while a group exchange is between member turns.
  botId: string;
}

export interface StoreState {
  /// The core answered its first snapshot.
  ready: boolean;
  /// This phone holds an identity: it is paired.
  paired: boolean;
  deviceId: string | null;
  identityId: string | null;
  relayUrl: string | null;
  relayConnected: boolean;
  devices: Device[];
  /// Device id → last seen (unix seconds), from the devices list.
  device_seen: Record<string, number>;
  bots: Bot[];
  chats: Chat[];
  /// Every bot's routines, from the roster.
  routines: Routine[];
  /// Auto-review, from the roster.
  auto_review: AutoReview;
  /// The account's provider credentials, the same on every Device.
  providers: ProviderStatus[];
  /// Turns in flight, by job id.
  running: Record<string, Running>;
  /// "Chef stopped without replying", by chat id, after a turn ends with nothing said.
  statuses: Record<string, string>;
  /// The chat on screen: new replies there do not count as unread.
  openChatId: string | null;
  /// Attachment id → file URI, for the bytes this phone has.
  files: Record<string, string>;
  dictation_lang?: string;
}

function empty(): Omit<StoreState, "ready" | "dictation_lang"> {
  return {
    paired: false,
    deviceId: null,
    identityId: null,
    relayUrl: null,
    relayConnected: false,
    devices: [],
    device_seen: {},
    bots: [],
    chats: [],
    routines: [],
    auto_review: { is_enabled: true, rules: [] },
    providers: [],
    running: {},
    statuses: {},
    openChatId: null,
    files: {},
  };
}

export const useStore = create<StoreState>()(() => ({ ...empty(), ready: false }));

/// Applies a change to the phone's own prefs and saves them.
export function mutate(update: (s: StoreState) => Partial<StoreState>) {
  useStore.setState((s) => update(s));
  savePrefs({ dictation_lang: useStore.getState().dictation_lang });
}

/// Back to unpaired: everything the core told us goes; the phone's prefs stay.
export function resetStore() {
  useStore.setState({ ...empty() });
}

// MARK: - Lookup

export function thisDeviceId(): string | null {
  return useStore.getState().deviceId;
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
  const device = s.devices.find((d) => d.id === id);
  return device !== undefined && (device.is_this_device || device.status === "online");
}

// MARK: - Reducers

/// The whole account, as `bootstrap` answers it.
export function replaceSnapshot(snapshot: {
  has_identity: boolean;
  identity_id: string | null;
  this_device_id: string | null;
  relay_url: string | null;
  relay_connected: boolean;
  devices: Device[];
  bots: Bot[];
  chats: Chat[];
  routines?: Routine[];
  auto_review?: AutoReview;
  providers?: ProviderStatus[];
  running_turns: { job_id: string; chat_id: string; bot_id: string }[];
}) {
  const running: Record<string, Running> = {};
  for (const turn of snapshot.running_turns ?? []) running[turn.job_id] = { chatId: turn.chat_id, botId: turn.bot_id };
  useStore.setState({
    ready: true,
    paired: snapshot.has_identity,
    deviceId: snapshot.this_device_id,
    identityId: snapshot.identity_id,
    relayUrl: snapshot.relay_url,
    relayConnected: snapshot.relay_connected,
    devices: snapshot.devices,
    device_seen: seenOf(snapshot.devices),
    bots: snapshot.bots,
    // A snapshot carries each chat's newest messages. Older pages already loaded stay ahead of
    // them, so a resync does not throw an open transcript back to the last page.
    chats: snapshot.chats.map((c) => {
      const messages = c.messages ?? [];
      const old = useStore.getState().chats.find((o) => o.id === c.id);
      const at = old && messages.length ? old.messages.findIndex((m) => m.id === messages[0].id) : -1;
      const kept = at > 0 ? { messages: [...old!.messages.slice(0, at), ...messages], has_more: old!.has_more } : { messages };
      return { ...c, ...kept, unread_count: c.unread_count ?? 0 };
    }),
    routines: snapshot.routines ?? [],
    auto_review: snapshot.auto_review ?? { is_enabled: true, rules: [] },
    providers: snapshot.providers ?? [],
    running,
  });
}

function seenOf(devices: Device[]): Record<string, number> {
  const seen: Record<string, number> = {};
  for (const device of devices) seen[device.id] = device.last_seen;
  return seen;
}

/// `roster.changed`: bots replace, chat metadata merges over kept messages, chats not named
/// are gone.
export function applyRoster(roster: { devices: Device[]; bots: Bot[]; chats: (ChatMeta & { unread_count: number; usage?: ChatUsage })[]; routines?: Routine[]; auto_review?: AutoReview; providers?: ProviderStatus[] }): { removed: string[] } {
  const removed: string[] = [];
  useStore.setState((s) => {
    const incoming = new Set(roster.chats.map((c) => c.id));
    for (const chat of s.chats) if (!incoming.has(chat.id)) removed.push(chat.id);
    const existing = new Map(s.chats.map((c) => [c.id, c]));
    const chats: Chat[] = roster.chats.map((meta) => {
      const old = existing.get(meta.id);
      return { ...meta, is_pinned: meta.is_pinned ?? false, messages: old?.messages ?? [], has_more: old?.has_more, unread_count: meta.unread_count ?? old?.unread_count ?? 0, usage: meta.usage ?? old?.usage };
    });
    return { devices: roster.devices, device_seen: seenOf(roster.devices), bots: roster.bots, chats, routines: roster.routines ?? s.routines, auto_review: roster.auto_review ?? s.auto_review, providers: roster.providers ?? s.providers };
  });
  return { removed };
}

/// A routine as the phone shows it: the change applied before the core's roster confirms it.
export function patchRoutine(id: string, update: (routine: Routine) => Routine) {
  useStore.setState((s) => ({ routines: s.routines.map((r) => (r.id === id ? update(r) : r)) }));
}

export function removeRoutine(id: string) {
  useStore.setState((s) => ({ routines: s.routines.filter((r) => r.id !== id) }));
}

export function upsertMessage(message: Message): { added: boolean } {
  let added = false;
  useStore.setState((s) => {
    let chats = s.chats;
    if (!chats.some((c) => c.id === message.chat_id)) {
      // Roster not here yet: keep the message under a placeholder until it is.
      chats = [...chats, { id: message.chat_id, kind: "group", title: "Chat", bot_ids: [], is_pinned: false, created_at: message.created_at, messages: [], unread_count: 0 }];
    }
    chats = chats.map((chat) => {
      if (chat.id !== message.chat_id) return chat;
      const index = chat.messages.findIndex((m) => m.id === message.id);
      if (index < 0) {
        added = true;
        return { ...chat, messages: [...chat.messages, message] };
      }
      if (JSON.stringify(chat.messages[index]) === JSON.stringify(message)) return chat;
      const messages = chat.messages.slice();
      messages[index] = message;
      return { ...chat, messages };
    });
    // A bot that spoke is no longer "stopped without replying".
    const statuses = message.author.kind === "bot" && message.body.kind === "text" && s.statuses[message.chat_id] ? omit(s.statuses, message.chat_id) : s.statuses;
    return { chats, statuses };
  });
  return { added };
}

export function removeMessage(chatId: string, messageId: string) {
  useStore.setState((s) => ({
    chats: s.chats.map((chat) => (chat.id === chatId ? { ...chat, messages: chat.messages.filter((m) => m.id !== messageId) } : chat)),
  }));
}

export function removeChat(chatId: string) {
  useStore.setState((s) => ({ chats: s.chats.filter((c) => c.id !== chatId), statuses: omit(s.statuses, chatId) }));
}

export function setChatUsage(chatId: string, usage: ChatUsage) {
  useStore.setState((s) => ({ chats: s.chats.map((c) => (c.id === chatId ? { ...c, usage } : c)) }));
}

/// The chat is read here; the core clears it everywhere.
export function markRead(chatId: string) {
  const chat = chatById(chatId);
  if (!chat || chat.unread_count === 0) return;
  useStore.setState((s) => ({ chats: s.chats.map((c) => (c.id === chatId ? { ...c, unread_count: 0 } : c)) }));
  void import("../../modules/lorca-core").then(({ request }) => request("chats.mark_read", { chat_id: chatId }).catch(() => {}));
}

export function markFile(id: string, uri: string) {
  useStore.setState((s) => (s.files[id] === uri ? s : { files: { ...s.files, [id]: uri } }));
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

function omit(record: Record<string, string>, key: string): Record<string, string> {
  const next = { ...record };
  delete next[key];
  return next;
}

// MARK: - Hooks

export function useChat(id: string): Chat | undefined {
  return useStore((s) => s.chats.find((c) => c.id === id));
}

export function useBots(): Bot[] {
  return useStore((s) => s.bots);
}

/// Keyed by bot id; the same Map while the roster is unchanged, so memoized rows and lists
/// built from it (the transcript's FlashList data) keep their identity across renders.
export function useBotMap(): Map<string, Bot> {
  const bots = useStore((s) => s.bots);
  return useMemo(() => new Map(bots.map((b) => [b.id, b])), [bots]);
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

/// A bot's routines, oldest first.
export function useRoutines(botId: string | undefined): Routine[] {
  return useStore(useShallow((s) => s.routines.filter((r) => r.bot_id === botId).sort((a, b) => a.created_at - b.created_at)));
}
