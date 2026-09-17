// The phone's side of the core: starts it once with the app's folder and this phone's facts,
// turns its events into store changes, and offers the screens the same verbs the Mac app has.
// Every call is one request over the JSON API the desktop app speaks to the CLI; the Rust
// core does the keys, the relay, jobs, rooms, and questions to Runners.

import { AppState, type AppStateStatus } from "react-native";
import * as core from "../../modules/tinybot-core";
import { hostFacts } from "./host";
import type { Attachment, Bot, Chat, ChatMeta, ChatUsage, Message } from "./model";
import { coreHome, loadPrefs, pathOf, wipePrefs } from "./prefs";
import {
  applyRoster,
  botById,
  chatById,
  markFile,
  markRead,
  removeChat,
  removeMessage,
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
  }

  private onAppState(status: AppStateStatus) {
    // Back in the foreground: the poll that was in flight died with the suspension.
    if (status === "active") core.wake();
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

  /// The change shows at once; the core's roster event confirms it.
  private patchChat(chatId: string, update: (meta: ChatMeta) => ChatMeta) {
    useStore.setState((s) => ({ chats: s.chats.map((chat) => (chat.id === chatId ? { ...chat, ...update(chat) } : chat)) }));
  }

  // MARK: - This phone

  async renameDevice(name: string) {
    await core.request("device.rename", { name });
  }

  /// Joins the identity the pairing string names. The core posts the request and waits for
  /// the Mac to accept; `signal` only stops the wait on this side.
  async pair(pairingString: string, deviceName: string | undefined, onProgress?: (progress: PairProgress) => void, signal?: AbortSignal) {
    onProgress?.({ phase: "posting" });
    const accepted = new Promise<unknown>((resolve, reject) => {
      core.request("pair.accept", { pairing_string: pairingString, device_name: deviceName?.trim() || undefined }).then(resolve, reject);
      signal?.addEventListener("abort", () => reject(new Error("Pairing cancelled")));
    });
    onProgress?.({ phase: "waiting" });
    await accepted;
    replaceSnapshot(await core.request<Snapshot>("bootstrap"));
    onProgress?.({ phase: "done" });
    core.wake();
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
