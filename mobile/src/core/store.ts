// The phone's view of the account: what the core last said. Pure data and reducers fed by the
// core's events (engine.ts applies them), the way the macOS app's AppStore mirrors the CLI. The
// core keeps the truth on disk; nothing here is persisted except the phone's own prefs.

import { useMemo } from "react";
import { create } from "zustand";
import { useShallow } from "zustand/react/shallow";
import { runsInTerminal, type AutoReview, type Bot, type Chat, type ChatMeta, type ChatUsage, type Device, type Message, type ProviderStatus, type RelayProblem, type Routine } from "./model";
import { t } from "../i18n";
import { savePrefs } from "./prefs";

export interface Running {
  chatId: string;
  /// Empty while a group exchange is between member turns.
  botId: string;
  /// Set when the turn is a run of a routine.
  routineId?: string;
}

/// A model call that failed in a way worth another try, asked again after `delay_ms`.
export interface Retry {
  attempt: number;
  max_attempts: number;
  delay_ms: number;
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
  /** The relay refused this build's protocol: it syncs again once the app is updated. */
  relayUpdateRequired: boolean;
  /** Why the last try to connect to the relay failed, until one goes through. */
  relayError: RelayProblem | null;
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
  /// The bot whose model is thinking, by chat id, until that bot's next message or the end of
  /// its turn.
  thinking: Record<string, string>;
  /// A model call waiting to be asked again, by chat id, until the chat hears more.
  retries: Record<string, Retry>;
  /// "Chef stopped without replying", by chat id, after a turn ends with nothing said.
  statuses: Record<string, string>;
  /// Commands that started running while the phone watched, by row id, until they have run for
  /// `TASK_DELAY_MS`: not running tasks yet, so a quick command never shows as one.
  pendingTasks: Record<string, true>;
  /// The chat on screen: new replies there do not count as unread.
  openChatId: string | null;
  /// Only foreground UI can acknowledge a reply as read.
  appActive: boolean;
  /// When the app last came to the foreground, in ms.
  activeSince: number;
  /// Attachment id → file URI, for the bytes this phone has.
  files: Record<string, string>;
  dictation_lang?: string;
}

function empty(): Omit<StoreState, "ready" | "dictation_lang" | "appActive" | "activeSince"> {
  return {
    paired: false,
    deviceId: null,
    identityId: null,
    relayUrl: null,
    relayConnected: false,
    relayUpdateRequired: false,
    relayError: null,
    devices: [],
    device_seen: {},
    bots: [],
    chats: [],
    routines: [],
    auto_review: { is_enabled: true, rules: [] },
    providers: [],
    running: {},
    thinking: {},
    retries: {},
    statuses: {},
    pendingTasks: {},
    openChatId: null,
    files: {},
  };
}

export const useStore = create<StoreState>()(() => ({ ...empty(), ready: false, appActive: false, activeSince: 0 }));

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
  relay_update_required?: boolean;
  relay_error?: RelayProblem | null;
  devices: Device[];
  bots: Bot[];
  chats: Chat[];
  routines?: Routine[];
  auto_review?: AutoReview;
  providers?: ProviderStatus[];
  running_turns: { job_id: string; chat_id: string; bot_id: string; routine_id?: string | null }[];
}) {
  const running: Record<string, Running> = {};
  for (const turn of snapshot.running_turns ?? []) running[turn.job_id] = { chatId: turn.chat_id, botId: turn.bot_id, routineId: turn.routine_id ?? undefined };
  const busy = new Set(Object.values(running).map((r) => r.chatId));
  const { thinking, retries } = useStore.getState();
  useStore.setState({
    ready: true,
    paired: snapshot.has_identity,
    deviceId: snapshot.this_device_id,
    identityId: snapshot.identity_id,
    relayUrl: snapshot.relay_url,
    relayConnected: snapshot.relay_connected,
    relayUpdateRequired: !!snapshot.relay_update_required,
    relayError: snapshot.relay_error ?? null,
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
    // What a turn that ended unheard was doing says nothing about the next one.
    thinking: pick(thinking, busy),
    retries: pick(retries, busy),
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

/// How long a command runs before it counts as a running task.
export const TASK_DELAY_MS = 2000;

/// A message the core stored or changed. `isNew` is the core's `message.added`: the bot's first
/// message after its thinking is what came of it.
export function upsertMessage(message: Message, isNew = true): { added: boolean } {
  let added = false;
  let previous: Message | undefined;
  let started = false;
  useStore.setState((s) => {
    let chats = s.chats;
    if (!chats.some((c) => c.id === message.chat_id)) {
      // Roster not here yet: keep the message under a placeholder until it is.
      chats = [...chats, { id: message.chat_id, kind: "group", title: t("Chat"), bot_ids: [], is_pinned: false, created_at: message.created_at, messages: [], unread_count: 0 }];
    }
    chats = chats.map((chat) => {
      if (chat.id !== message.chat_id) return chat;
      const index = chat.messages.findIndex((m) => m.id === message.id);
      previous = chat.messages[index];
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
    const retries = omit(s.retries, message.chat_id);
    const thought = isNew && message.author.kind === "bot" && s.thinking[message.chat_id] === message.author.bot_id;
    const thinking = thought ? omit(s.thinking, message.chat_id) : s.thinking;
    // A command that starts running here waits to count as a running task.
    started = runsInTerminal(message) && !runsInTerminal(previous);
    const pendingTasks = started ? { ...s.pendingTasks, [message.id]: true as const } : runsInTerminal(message) ? s.pendingTasks : omit(s.pendingTasks, message.id);
    return { chats, statuses, retries, thinking, pendingTasks };
  });
  if (started) setTimeout(() => useStore.setState((s) => ({ pendingTasks: omit(s.pendingTasks, message.id) })), TASK_DELAY_MS);
  return { added };
}

export function removeMessage(chatId: string, messageId: string) {
  useStore.setState((s) => ({
    chats: s.chats.map((chat) => (chat.id === chatId ? { ...chat, messages: chat.messages.filter((m) => m.id !== messageId) } : chat)),
    pendingTasks: omit(s.pendingTasks, messageId),
  }));
}

export function removeChat(chatId: string) {
  useStore.setState((s) => ({
    chats: s.chats.filter((c) => c.id !== chatId),
    statuses: omit(s.statuses, chatId),
    thinking: omit(s.thinking, chatId),
    retries: omit(s.retries, chatId),
  }));
}

export function setChatUsage(chatId: string, usage: ChatUsage) {
  useStore.setState((s) => ({ chats: s.chats.map((c) => (c.id === chatId ? { ...c, usage } : c)) }));
}

/// The chat is read here; the core clears it everywhere.
export function markRead(chatId: string) {
  if (!useStore.getState().appActive) return;
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

/// `job.thinking`: the bot's model started thinking.
export function setThinking(chatId: string, botId: string) {
  useStore.setState((s) => ({ thinking: { ...s.thinking, [chatId]: botId } }));
}

/// `job.retry`: a model call failed and waits to be asked again.
export function setRetry(chatId: string, retry: Retry) {
  useStore.setState((s) => ({ retries: { ...s.retries, [chatId]: retry } }));
}

/// A turn ended: its retry is over, and so is its bot's thinking. An empty `botId` is a group
/// exchange, which ends every member's.
export function endActivity(chatId: string, botId: string) {
  useStore.setState((s) => ({
    retries: omit(s.retries, chatId),
    thinking: !botId || s.thinking[chatId] === botId ? omit(s.thinking, chatId) : s.thinking,
  }));
}

export function setStatus(chatId: string, text: string | null) {
  useStore.setState((s) => {
    const next = { ...s.statuses };
    if (text) next[chatId] = text;
    else delete next[chatId];
    return { statuses: next };
  });
}

function omit<T>(record: Record<string, T>, key: string): Record<string, T> {
  if (!(key in record)) return record;
  const next = { ...record };
  delete next[key];
  return next;
}

function pick<T>(record: Record<string, T>, keys: Set<string>): Record<string, T> {
  return Object.fromEntries(Object.entries(record).filter(([key]) => keys.has(key)));
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

/// The chat's running tasks: its commands running in their terminals, here or on their Runners,
/// in the order they started. One that starts while the phone watches counts once it has run for
/// `TASK_DELAY_MS`; one that was running before counts at once.
export function runningTasks(s: StoreState, chatId: string): Message[] {
  return s.chats.find((c) => c.id === chatId)?.messages.filter((m) => runsInTerminal(m) && !s.pendingTasks[m.id]) ?? [];
}

/// What the Running tasks button counts and its sheet lists.
export function useRunningTasks(chatId: string): Message[] {
  return useStore(useShallow((s) => runningTasks(s, chatId)));
}

export function useIsWorking(chatId: string): boolean {
  return useStore((s) => Object.values(s.running).some((r) => r.chatId === chatId));
}

export function useWorkingBotIds(): Set<string> {
  return useStore(useShallow((s) => new Set(Object.values(s.running).map((r) => r.botId).filter(Boolean))));
}

/// A bot's routines, oldest first. One whose run is among the turns in flight is running, as on
/// the Mac, wherever the run was started.
export function useRoutines(botId: string | undefined): Routine[] {
  const routines = useStore(useShallow((s) => s.routines.filter((r) => r.bot_id === botId).sort((a, b) => a.created_at - b.created_at)));
  const running = useStore(useShallow((s) => Object.values(s.running).flatMap((r) => (r.routineId ? [r.routineId] : []))));
  return useMemo(() => routines.map((r) => (r.is_running || !running.includes(r.id) ? r : { ...r, is_running: true })), [routines, running]);
}
