// The phone's view of the account: what the core last said. Pure data and reducers fed by the
// core's events (engine.ts applies them), the way the macOS app's AppStore mirrors the CLI. The
// core keeps the truth on disk; nothing here is persisted except the phone's own prefs.

import { useMemo } from "react";
import { create } from "zustand";
import { useShallow } from "zustand/react/shallow";
import { groupOutputs, reviewIsOpen, runsInTerminal, taskOrder, type AutoReview, type ProjectContext, type BudgetState, type DurableTask, type ReviewItem, type OutputSeries, type Bot, type Chat, type ChatMeta, type ChatUsage, type Device, type Message, type ProviderModel, type ProviderStatus, type RelayProblem, type Routine } from "./model";
import { t } from "../i18n";
import { savePrefs } from "./prefs";
import { emptyAttention, type AttentionView } from "./attention";

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
  attention: AttentionView;
  /// The account's provider credentials, the same on every Device.
  providers: ProviderStatus[];
  /// The models the core's catalog offers, for the Model and Thinking pickers.
  models: ProviderModel[];
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
  /// Attachment id → why its bytes could not be fetched, kept until the user retries, so a
  /// scroll does not ask again.
  fileErrors: Record<string, string>;
  /// Chat id → every version of its outputs the core listed when the chat's details last opened.
  outputs: Record<string, Message[]>;
  /// Group chat id → its project context as the core last listed it, from when the group's
  /// details opened, and again on every `projects.changed` for it.
  projects: Record<string, ProjectContext>;
  /// The account's durable tasks, each at the newest revision this phone has.
  tasks: DurableTask[];
  /// What the account's bots left for review, each at the newest revision this phone has.
  reviews: ReviewItem[];
  /// Every Runner's limits and what its turns, tasks, and routines used of them.
  budgets: BudgetState[];
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
    attention: emptyAttention(),
    providers: [],
    models: [],
    running: {},
    thinking: {},
    retries: {},
    statuses: {},
    pendingTasks: {},
    openChatId: null,
    files: {},
    fileErrors: {},
    outputs: {},
    projects: {},
    tasks: [],
    reviews: [],
    budgets: [],
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
  tasks?: DurableTask[];
  reviews?: ReviewItem[];
  budgets?: BudgetState[];
  auto_review?: AutoReview;
  attention?: AttentionView;
  providers?: ProviderStatus[];
  models?: ProviderModel[];
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
    tasks: snapshot.tasks ?? [],
    reviews: snapshot.reviews ?? [],
    budgets: snapshot.budgets ?? [],
    auto_review: snapshot.auto_review ?? { is_enabled: true, rules: [] },
    attention: snapshot.attention ?? emptyAttention(),
    providers: snapshot.providers ?? [],
    models: snapshot.models ?? [],
    running,
    // What a turn that ended unheard was doing says nothing about the next one.
    thinking: pick(thinking, busy),
    retries: pick(retries, busy),
  });
}

/// The value held, when the incoming one says the same: a roster's lists are small, and a new
/// identity re-renders everything built from them.
function same<T>(held: T, incoming: T): T {
  return held === incoming || JSON.stringify(held) === JSON.stringify(incoming) ? held : incoming;
}

/// Two chats whose fields hold the same values, compared one level deep (`messages` by identity).
function sameFields(a: Chat, b: Chat): boolean {
  const keys = new Set([...Object.keys(a), ...Object.keys(b)]) as Set<keyof Chat>;
  for (const key of keys) {
    const x = a[key];
    const y = b[key];
    if (x === y || key === "messages") continue;
    if (x && y && typeof x === "object" && JSON.stringify(x) === JSON.stringify(y)) continue;
    return false;
  }
  return a.messages === b.messages;
}

function seenOf(devices: Device[]): Record<string, number> {
  const seen: Record<string, number> = {};
  for (const device of devices) seen[device.id] = device.last_seen;
  return seen;
}

/// `roster.changed`: bots replace, chat metadata merges over kept messages, chats not named
/// are gone.
export function applyRoster(roster: { devices: Device[]; bots: Bot[]; chats: (ChatMeta & { unread_count: number; usage?: ChatUsage })[]; routines?: Routine[]; auto_review?: AutoReview; providers?: ProviderStatus[]; models?: ProviderModel[] }): { removed: string[] } {
  const removed: string[] = [];
  useStore.setState((s) => {
    const incoming = new Set(roster.chats.map((c) => c.id));
    for (const chat of s.chats) if (!incoming.has(chat.id)) removed.push(chat.id);
    const existing = new Map(s.chats.map((c) => [c.id, c]));
    // The core sends the whole roster after every turn, read and pin. What did not change keeps
    // its identity, so the screens' memoized rows and lists built from it stay as they are.
    const next: Chat[] = roster.chats.map((meta) => {
      const old = existing.get(meta.id);
      const chat = { ...meta, is_pinned: meta.is_pinned ?? false, messages: old?.messages ?? [], has_more: old?.has_more, unread_count: meta.unread_count ?? old?.unread_count ?? 0, usage: meta.usage ?? old?.usage };
      return old && sameFields(old, chat) ? old : chat;
    });
    const chats = next.length === s.chats.length && next.every((chat, i) => chat === s.chats[i]) ? s.chats : next;
    return {
      devices: same(s.devices, roster.devices),
      device_seen: same(s.device_seen, seenOf(roster.devices)),
      bots: same(s.bots, roster.bots),
      chats,
      routines: same(s.routines, roster.routines ?? s.routines),
      auto_review: same(s.auto_review, roster.auto_review ?? s.auto_review),
      providers: same(s.providers, roster.providers ?? s.providers),
      models: same(s.models, roster.models ?? s.models),
    };
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
    outputs: s.outputs[chatId] ? { ...s.outputs, [chatId]: s.outputs[chatId].filter((m) => m.id !== messageId) } : s.outputs,
  }));
}

export function removeChat(chatId: string) {
  useStore.setState((s) => ({
    chats: s.chats.filter((c) => c.id !== chatId),
    statuses: omit(s.statuses, chatId),
    thinking: omit(s.thinking, chatId),
    retries: omit(s.retries, chatId),
    outputs: omit(s.outputs, chatId),
    projects: omit(s.projects, chatId),
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
  // Required here rather than at the top, which keeps the store free of the native module until
  // it is used. A dynamic import() would ask Metro for a separate chunk, which a bundle served
  // without a page location cannot load: the read never reached the core, and the count came back.
  const { request } = require("../../modules/lorca-core") as typeof import("../../modules/lorca-core");
  request("chats.mark_read", { chat_id: chatId }).catch(() => {});
}

export function markFile(id: string, uri: string) {
  useStore.setState((s) => (s.files[id] === uri ? s : { files: { ...s.files, [id]: uri }, fileErrors: omit(s.fileErrors, id) }));
}

/// The attachment's bytes could not be fetched, or (`null`) may be asked for again.
export function markFileError(id: string, error: string | null) {
  useStore.setState((s) => ({ fileErrors: error === null ? omit(s.fileErrors, id) : { ...s.fileErrors, [id]: error } }));
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

/// The chat's outputs, the latest published first: the output messages loaded in the chat and
/// what `outputs.list` answered for it, one series per output with every version.
export function useOutputs(chatId: string | undefined): OutputSeries[] {
  const messages = useStore((s) => (chatId ? s.chats.find((c) => c.id === chatId)?.messages : undefined));
  const listed = useStore((s) => (chatId ? s.outputs[chatId] : undefined));
  // The loaded message first: it is the newer copy of one the list also has.
  return useMemo(() => groupOutputs([...(messages ?? []), ...(listed ?? [])]), [messages, listed]);
}

/// The group's project context, once its details have listed it.
export function useProject(chatId: string | undefined): ProjectContext | undefined {
  return useStore((s) => (chatId ? s.projects[chatId] : undefined));
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
  const messages = s.chats.find((c) => c.id === chatId)?.messages;
  if (!messages) return [];
  // Asked on every store update while a chat is open; the answer changes only with the chat's
  // messages or the pending set.
  const cached = tasksCache.get(messages);
  if (cached && cached.pending === s.pendingTasks) return cached.tasks;
  const tasks = messages.filter((m) => runsInTerminal(m) && !s.pendingTasks[m.id]);
  tasksCache.set(messages, { pending: s.pendingTasks, tasks });
  return tasks;
}

const tasksCache = new WeakMap<Message[], { pending: StoreState["pendingTasks"]; tasks: Message[] }>();

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

/// Every Runner's limits, as `budgets.changed` sends them.
export function setBudgets(budgets: BudgetState[]) {
  useStore.setState({ budgets });
}

/// The allowance of a DM (`chat`), a task, or a routine on its Runner.
export function useBudget(kind: BudgetState["kind"], id: string | undefined, runnerId: string | undefined): BudgetState | undefined {
  return useStore((s) => s.budgets.find((b) => b.kind === kind && b.id === id && b.runner_id === runnerId));
}

/// The DM's newest turn when it stopped at a limit or was interrupted: the one to resume.
export function useStoppedTurn(chatId: string | undefined, runnerId: string | undefined): BudgetState | undefined {
  return useStore((s) => {
    let newest: BudgetState | undefined;
    for (const b of s.budgets) {
      if (b.kind === "job" && b.chat_id === chatId && b.runner_id === runnerId && (!newest || b.updated_at > newest.updated_at)) newest = b;
    }
    return newest && (newest.state === "budget_exhausted" || newest.state === "interrupted") ? newest : undefined;
  });
}

/// Takes a task from a reply or an event unless this phone already has a newer revision of it.
export function acceptDurableTask(task: DurableTask) {
  useStore.setState((s) => {
    const index = s.tasks.findIndex((each) => each.id === task.id);
    if (index < 0) return { tasks: [...s.tasks, task] };
    if (s.tasks[index].revision >= task.revision) return {};
    const tasks = s.tasks.slice();
    tasks[index] = task;
    return { tasks };
  });
}

/// The chat's durable tasks, what waits on the user first, then open work, each newest first.
export function useDurableTasks(chatId: string | undefined): DurableTask[] {
  const tasks = useStore((s) => s.tasks);
  return useMemo(
    () =>
      chatId
        ? tasks
            .filter((task) => task.chat_ids.includes(chatId))
            .sort((a, b) => taskOrder(a.state) - taskOrder(b.state) || b.updated_at - a.updated_at)
        : [],
    [tasks, chatId],
  );
}

export function useDurableTask(id: string | undefined): DurableTask | undefined {
  return useStore((s) => s.tasks.find((task) => task.id === id));
}

/// Takes a review item from a reply or an event unless this phone already has a newer revision.
export function acceptReview(item: ReviewItem) {
  useStore.setState((s) => {
    const index = s.reviews.findIndex((each) => each.id === item.id);
    if (index < 0) return { reviews: [...s.reviews, item] };
    if (s.reviews[index].revision > item.revision) return {};
    const reviews = s.reviews.slice();
    reviews[index] = item;
    return { reviews };
  });
}

/// What the chat's bots left for the user that waits or runs, oldest first. How each ended stays
/// in the chat.
export function useOpenReviews(chatId: string | undefined): ReviewItem[] {
  const reviews = useStore((s) => s.reviews);
  return useMemo(
    () => (chatId ? reviews.filter((item) => item.origin.chat_id === chatId && reviewIsOpen(item)).sort((a, b) => a.created_at - b.created_at) : []),
    [reviews, chatId],
  );
}

export function useReview(id: string | undefined): ReviewItem | undefined {
  return useStore((s) => s.reviews.find((item) => item.id === id));
}
