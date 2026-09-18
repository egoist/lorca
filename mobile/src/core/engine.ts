// The phone's side of the core: starts it once with the app's folder and this phone's facts,
// turns its events into store changes, and offers the screens the same verbs the Mac app has.
// Every call is one request over the JSON API the desktop app speaks to the CLI; the Rust
// core does the keys, the relay, jobs, rooms, and questions to Runners.

import { AppState, type AppStateStatus } from "react-native";
import * as core from "../../modules/tinybot-core";
import { hostFacts } from "./host";
import type { Attachment, AutoReview, Bot, Chat, ChatMeta, ChatUsage, Message } from "./model";
import { coreHome, loadPrefs, pathOf, wipePrefs } from "./prefs";
import { installPushHandlers, registerForPushes } from "./push";
import {
  applyRoster,
  botById,
  chatById,
  markFile,
  markRead,
  patchRoutine,
  removeChat,
  removeMessage,
  removeRoutine,
  replaceSnapshot,
  resetStore,
  setChatUsage,
  setRunning,
  setStatus,
  upsertMessage,
  useStore,
} from "./store";

/// A file the composer picked, before it is read and stored.
export interface PickedFile {
  uri: string;
  name: string;
  mime: string;
  size?: number;
  width?: number;
  height?: number;
}

export interface PairProgress {
  phase: "posting" | "waiting" | "done";
}

type Snapshot = Parameters<typeof replaceSnapshot>[0];

class Engine {
  private started = false;
  private fetchingFiles = new Set<string>();
  private loadingOlder = new Set<string>();

  // MARK: - Lifecycle

  /// Starts the core and takes its first snapshot. Safe to call again.
  async start() {
    if (this.started) return;
    this.started = true;
    useStore.setState({ dictation_lang: loadPrefs().dictation_lang });
    core.onEvent((frame) => this.apply(frame.event, frame.data));
    core.start(coreHome(), hostFacts());
    AppState.addEventListener("change", (status) => this.onAppState(status));
    replaceSnapshot(await core.request<Snapshot>("bootstrap"));
    installPushHandlers();
    if (useStore.getState().paired) void registerForPushes();
  }

  private onAppState(status: AppStateStatus) {
    if (status !== "active") return;
    // Back in the foreground: the poll that was in flight died with the suspension.
    core.wake();
    // The token can change, and permission may have been given in Settings meanwhile.
    if (useStore.getState().paired) void registerForPushes();
  }

  /// Pull to refresh: ask the relay now.
  notify() {
    core.wake();
  }

  get deviceId(): string | null {
    return useStore.getState().deviceId;
  }

  // MARK: - Events in

  private apply(event: string, data: any) {
    switch (event) {
      case "snapshot":
        replaceSnapshot(data as Snapshot);
        break;
      case "roster.changed": {
        const { removed } = applyRoster(data);
        for (const chatId of removed) removeChat(chatId);
        break;
      }
      case "message.added":
      case "message.updated": {
        const message = data.message as Message;
        upsertMessage(message);
        // A reply into the chat on screen is read as it lands; the core counts it unread.
        if (message.author.kind === "bot" && useStore.getState().openChatId === message.chat_id) markRead(message.chat_id);
        break;
      }
      case "message.removed":
        removeMessage(data.chat_id, data.message_id);
        break;
      case "chat.removed":
        removeChat(data.chat_id);
        break;
      case "job.started":
        setRunning(data.job_id, { chatId: data.chat_id, botId: data.bot_id ?? "" });
        break;
      case "job.finished":
        setRunning(data.job_id, null);
        if (!Object.values(useStore.getState().running).some((r) => r.chatId === data.chat_id)) this.noteSilence(data.chat_id);
        break;
      case "chat.usage":
        setChatUsage(data.chat_id, data.usage as ChatUsage);
        break;
      case "relay.status":
        useStore.setState((s) => ({ relayConnected: !!data.connected, relayUrl: data.url ?? s.relayUrl }));
        break;
      case "identity.changed":
        if (!data.has_identity) resetStore();
        else useStore.setState({ paired: true });
        break;
      default:
        break;
    }
  }

  /// "Chef stopped without replying" when every turn in the chat ended and the user's message
  /// is still last.
  private noteSilence(chatId: string) {
    const chat = chatById(chatId);
    if (!chat) return;
    const last = [...chat.messages].reverse().find((m) => m.body.kind === "text" || m.body.kind === "handoff");
    if (last?.author.kind === "you") {
      const name = chat.kind === "dm" ? botById(chat.bot_ids[0] ?? "")?.name : undefined;
      setStatus(chatId, `${name ?? chatTitle(chat)} stopped without replying`);
    }
  }

  // MARK: - Chats

  /// Sends the user's message with its files; the core starts the turns it calls for.
  async sendMessage(chatId: string, text: string, files: PickedFile[] = []): Promise<Message> {
    const attachments = files.map((file) => ({ path: pathOf(file.uri), name: file.name, mime: file.mime, width: file.width, height: file.height }));
    const { message } = await core.request<{ message: Message }>("chats.send", { chat_id: chatId, text, attachments });
    upsertMessage(message);
    setStatus(chatId, null);
    return message;
  }

  /// The page of messages before the chat's first one, as the transcript nears its top. One
  /// request per chat at a time.
  async loadOlder(chatId: string): Promise<void> {
    const first = chatById(chatId)?.messages[0];
    if (!first || !chatById(chatId)?.has_more || this.loadingOlder.has(chatId)) return;
    this.loadingOlder.add(chatId);
    try {
      const page = await core.request<{ messages: Message[]; has_more: boolean }>("chats.messages", { chat_id: chatId, before: first.id });
      useStore.setState((s) => ({
        chats: s.chats.map((c) => {
          if (c.id !== chatId || c.messages[0]?.id !== first.id) return c;
          const known = new Set(c.messages.map((m) => m.id));
          return { ...c, messages: [...page.messages.filter((m) => !known.has(m.id)), ...c.messages], has_more: page.has_more };
        }),
      }));
    } catch (error) {
      console.warn("loading older messages", error instanceof Error ? error.message : error);
    } finally {
      this.loadingOlder.delete(chatId);
    }
  }

  /// The attachment's bytes, from this phone's copy or the relay, as a file URI in the store.
  async fetchFile(attachment: Attachment): Promise<void> {
    if (useStore.getState().files[attachment.id] || this.fetchingFiles.has(attachment.id)) return;
    this.fetchingFiles.add(attachment.id);
    try {
      const { path } = await core.request<{ path: string }>("files.path", { attachment });
      markFile(attachment.id, `file://${path}`);
    } catch (error) {
      console.warn("fetching attachment", error instanceof Error ? error.message : error);
    } finally {
      this.fetchingFiles.delete(attachment.id);
    }
  }

  async createBot(input: { name: string; label: string; description?: string; instructions: string; symbol_name: string; accent: string; runner_id: string; provider: string; model?: string; thinking?: string }): Promise<{ bot: Bot; chatId: string }> {
    const { bot, chat_id } = await core.request<{ bot: Bot; chat_id: string }>("bots.create", input);
    return { bot, chatId: chat_id };
  }

  updateBot(id: string, update: Partial<Bot>) {
    void core.request("bots.update", { id, ...update });
  }

  /// The bot's symbol and accent, the look under and behind its image.
  async setBotLook(id: string, look: { symbol_name?: string; accent?: string }): Promise<void> {
    await core.request("bots.update", { id, ...look });
  }

  /// A custom profile image from a picked file (null removes it). The core copies the file,
  /// uploads it as a `file` blob, and names it in the roster for every Device.
  async setBotAvatar(id: string, file: PickedFile | null): Promise<void> {
    const avatar = file ? { path: pathOf(file.uri), name: file.name, mime: file.mime, width: file.width, height: file.height } : null;
    const { bot } = await core.request<{ bot: Bot }>("bots.update", { id, avatar });
    // The picked file is the same picture; show it before the store copy is asked for.
    if (file && bot.avatar) markFile(bot.avatar.id, file.uri);
  }

  async createGroup(title: string, botIds: string[]): Promise<Chat> {
    const { chat } = await core.request<{ chat: Chat }>("chats.create", { kind: "group", title: title.trim() || undefined, bot_ids: botIds });
    return { ...chat, messages: chat.messages ?? [], unread_count: chat.unread_count ?? 0 };
  }

  renameChat(chatId: string, title: string) {
    this.patchChat(chatId, (meta) => ({ ...meta, title: title.trim() || null }));
    void core.request("chats.rename", { chat_id: chatId, title });
  }

  pinChat(chatId: string, pinned: boolean) {
    this.patchChat(chatId, (meta) => ({ ...meta, is_pinned: pinned }));
    void core.request("chats.pin", { chat_id: chatId, pinned });
  }

  deleteChat(chatId: string) {
    removeChat(chatId);
    void core.request("chats.delete", { chat_id: chatId });
  }

  addBot(chatId: string, botId: string) {
    void core.request("chats.add_bot", { chat_id: chatId, bot_id: botId });
  }

  removeBot(chatId: string, botId: string) {
    void core.request("chats.remove_bot", { chat_id: chatId, bot_id: botId });
  }

  setOwner(chatId: string, botId: string) {
    void core.request("chats.set_owner", { chat_id: chatId, bot_id: botId });
  }

  // MARK: - Auto-review

  /// Replaces Auto-review (the switch and the rules); the core's roster event confirms it and
  /// gives a new rule its id.
  setAutoReview(value: AutoReview) {
    useStore.setState({ auto_review: value });
    void core.request("auto_review.set", { is_enabled: value.is_enabled, rules: value.rules });
  }

  // MARK: - Plugins

  /// Answers a permission card; the core's message event confirms the decision.
  answerPermission(chatId: string, messageId: string, decision: "allow" | "always" | "deny") {
    const decided = decision === "always" ? "always" : decision === "deny" ? "denied" : "allowed";
    useStore.setState((s) => ({
      chats: s.chats.map((c) =>
        c.id === chatId ? { ...c, messages: c.messages.map((m) => (m.id === messageId && m.body.kind === "permission" ? { ...m, body: { ...m.body, decision: decided } } : m)) } : c,
      ),
    }));
    void core.request("chats.permission", { chat_id: chatId, message_id: messageId, decision });
  }

  // MARK: - Routines

  /// Pauses or resumes a routine; a resumed schedule counts from now.
  setRoutineEnabled(id: string, enabled: boolean) {
    patchRoutine(id, (r) => ({ ...r, is_enabled: enabled, paused_reason: undefined, next_run_at: enabled ? r.next_run_at : null }));
    void core.request("routines.update", { id, enabled });
  }

  /// Runs the routine now, on its bot's Runner.
  runRoutine(id: string) {
    patchRoutine(id, (r) => ({ ...r, is_running: true }));
    void core.request("routines.run", { id });
  }

  deleteRoutine(id: string) {
    removeRoutine(id);
    void core.request("routines.delete", { id });
  }

  /// The change shows at once; the core's roster event confirms it.
  private patchChat(chatId: string, update: (meta: ChatMeta) => ChatMeta) {
    useStore.setState((s) => ({ chats: s.chats.map((chat) => (chat.id === chatId ? { ...chat, ...update(chat) } : chat)) }));
  }

  // MARK: - This phone

  async renameDevice(name: string) {
    await core.request("device.rename", { name });
  }

  /// Joins the identity the pairing string names. The core posts the request (`pair.posted`
  /// marks that) and waits for the Mac to accept; `signal` aborts the wait in the core too,
  /// which answers "Pairing cancelled".
  async pair(pairingString: string, deviceName: string | undefined, onProgress?: (progress: PairProgress) => void, signal?: AbortSignal) {
    onProgress?.({ phase: "posting" });
    const stop = core.onEvent((frame) => {
      if (frame.event === "pair.posted") onProgress?.({ phase: "waiting" });
    });
    const abort = () => void core.request("pair.abort").catch(() => {});
    signal?.addEventListener("abort", abort);
    try {
      await core.request("pair.accept", { pairing_string: pairingString, device_name: deviceName?.trim() || undefined });
    } finally {
      stop();
      signal?.removeEventListener("abort", abort);
    }
    replaceSnapshot(await core.request<Snapshot>("bootstrap"));
    onProgress?.({ phase: "done" });
    core.wake();
    void registerForPushes();
  }

  /// Unpairs another Device. The core has the relay drop its key; the Device wipes its copy
  /// of the account the next time it connects. Rejects when the relay could not be told.
  async unpairDevice(id: string) {
    await core.request("device.unpair", { id });
    useStore.setState((s) => ({ devices: s.devices.filter((d) => d.id !== id) }));
  }

  /// Forgets the identity: keys, account key, and everything synced.
  async unpair() {
    await core.request("identity.forget");
    resetStore();
    wipePrefs();
  }
}

export const engine = new Engine();

// MARK: - Helpers

export function chatTitle(chat: ChatMeta): string {
  if (chat.title?.trim()) return chat.title.trim();
  const names = chat.bot_ids.map((id) => botById(id)?.name).filter((n): n is string => !!n);
  if (chat.kind === "dm") return names[0] ?? "Chat";
  return names.length ? names.join(", ") : "Group";
}
