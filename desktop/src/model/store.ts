// The app's model, after the macOS app's AppStore. Everything comes from the CLI over 127.0.0.1;
// mutations are applied optimistically and confirmed by the events the CLI sends back.
// `LORCA_MOCK=1` (or `?mock=1` in a browser tab) runs the seeded demo with the in-process reply
// engine instead.
//
// The store is plain and synchronous, like the Swift one: its state is read right after it is
// written. Views learn what changed from its events (reactive.ts turns them into signals).

import { cliTransport, hostInfo, type CLIState, type FileInfo, type Transport } from "../host";
import { L } from "../l10n";
import {
  attachmentSummary,
  authorBotID,
  canAddBot,
  canRemoveBot,
  chatOwner,
  isCustomKind,
  isDM,
  isGroup,
  quoteOf,
  isRunner,
  isSentMessage,
  lastActivity,
  newMessageID,
  providerKinds,
  providerName,
  takesInput,
  verbPhrase,
  you,
  type Accent,
  type Attachment,
  type AutoReview,
  type Bot,
  type BotMemory,
  type BotTemplate,
  type BuiltInProviderKind,
  type Chat,
  type CommandState,
  type CustomAPI,
  type CustomModel,
  type CustomProviderKind,
  type Device,
  type InstalledPlugin,
  type Marketplace,
  type Message,
  type PluginDetail,
  type ProviderCredential,
  type ProviderKind,
  type ProviderModel,
  type Routine,
  commandRunOf,
  runsInForeground,
} from "./models";
import { parseLocally, type McpEntry, type McpFile, type McpServer, type ParsedServer } from "./mcp";
import { ReplyEngine } from "./replies";
import {
  toAutoReview,
  toBot,
  toBotMemory,
  toChat,
  toCustomModel,
  toDevice,
  toMarketplace,
  toMcpFile,
  toMcpServer,
  toMessage,
  toParsedServers,
  toPlugin,
  toPluginDetail,
  toModels,
  toProviders,
  toRoutine,
  toUsage,
  type WireChatUsage,
  type WireJobEvent,
  type WireJobRetry,
  type WireMcpServer,
  type WireMessage,
  type WireMessagePage,
  type WireModelList,
  type WirePairStart,
  type WirePairStatus,
  type WireRelayStatus,
  type WireRosterChanged,
  type WireSearchResults,
  type WireSnapshot,
  type WireSyncAccount,
} from "./wire";

export type StoreEvent =
  | { kind: "snapshotReplaced" }
  | { kind: "rosterChanged" }
  | { kind: "chatsChanged" }
  | { kind: "chatChanged"; chatID: string }
  | { kind: "messageAdded"; chatID: string; messageID: string }
  | { kind: "messageChanged"; chatID: string; messageID: string }
  | { kind: "messageRemoved"; chatID: string; messageID: string }
  | { kind: "respondingChanged"; chatID: string }
  /** A page of older messages was put ahead of the chat's first one. */
  | { kind: "olderMessagesLoaded"; chatID: string }
  /** A bot's turn ended; `startedAt` is when this app saw it start. */
  | { kind: "turnFinished"; chatID: string; botID: string; startedAt: number }
  /** A command in the chat has run long enough to count as a running task. */
  | { kind: "runningTasksChanged"; chatID: string }
  | { kind: "connectionChanged" }
  | { kind: "identityChanged" };

/** A file picked, dropped, or pasted into the composer, before the CLI stores it. The id is minted
 * here so the bubble the app shows right away and the CLI's copy agree. */
export interface OutgoingAttachment {
  attachment: Attachment;
  path: string;
  url: string;
}

interface RunningJob {
  id: string;
  chatID: string;
  /** Empty while a group exchange is between member turns. */
  botID: string;
  routineID?: string;
}

export class RequestError extends Error {}

/** The words of an error the app or the CLI raised, in the app's language when it has them. */
export function errorText(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);
  return L(message);
}

export function shortID(prefix: string, length = 8): string {
  return `${prefix}-${crypto.randomUUID().toLowerCase().slice(0, length)}`;
}

export function attachmentID(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(6));
  return `att-${[...bytes].map((byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

export class AppStore {
  /** The seeded demo (`LORCA_MOCK=1`), which runs without a CLI. */
  get isMock(): boolean {
    return hostInfo().isMock;
  }
  private transport: Transport | null = null;

  devices: Device[] = [];
  bots: Bot[] = [];
  chats: Chat[] = [];
  /** Every bot's routines, from the roster. */
  routines: Routine[] = [];
  /** Auto-review, shared through the roster. */
  autoReview: AutoReview = { isEnabled: true, rules: [] };
  /** The account's provider credentials, the same on every Device. */
  providers: ProviderCredential[] = [];
  /** The models the CLI's catalog offers, for the Model and Thinking pickers. */
  models: ProviderModel[] = [];

  /** True when the CLI answers on localhost (mock: toggled from the Debug menu). */
  isConnected = false;
  /** The first connection is still loading. The window is visible throughout this phase. */
  isStarting = true;
  /** null until the CLI has supplied the initial snapshot. */
  hasIdentity: boolean | null = null;
  isIdentityDevice = false;
  identityID: string | null = null;
  relayConnected = false;
  /** The relay refused this build's protocol: it syncs again once Lorca is updated. */
  relayUpdateRequired = false;
  /** Why the last try to connect to the relay failed, as the CLI words it, until one goes through. */
  relayError: string | null = null;
  relayURL: string | null = null;
  /** Where the connection and the launcher stand, for the offline state. */
  cliState: CLIState = { connection: "disconnected", launcher: { kind: "idle" }, starting: true };

  /** Turns in flight, by job id: the chat and the bot, and the routine when the turn is one of its
   * runs. Drives the "is working" row, the presence dot on avatars, and a routine's spinner. */
  private runningJobs: RunningJob[] = [];
  /** When each turn in flight was first seen, so a finished turn's reply can be told from what the
   * bot said before it. */
  private jobStarts = new Map<string, number>();
  /** When this app saw each command start running in its terminal, by row. */
  private commandStarts = new Map<string, number>();
  /** How long a command runs before it counts as a running task. */
  static readonly taskDelay = 2000;
  /** The chat last reported to the CLI as on screen; `undefined` is none reported yet. */
  private reportedWatchedChat: string | null | undefined = undefined;
  private loadingOlder = new Set<string>();
  replyEngine: { respond(prompt: string, chat: Chat, messageID: string): void; cancel(chatID: string): void; sendNow(chatID: string): void } | null = null;
  private started = false;
  private startupTimer: ReturnType<typeof setTimeout> | null = null;
  private bootstrapGeneration = 0;
  private isBootstrapping = true;
  private bootstrapEvents: { name: string; data: unknown }[] = [];
  private isApplyingBootstrap = false;
  private listeners = new Set<(event: StoreEvent) => void>();

  /** "Retrying (2 of 3) in 4 s", while a turn's model call waits to be asked again. */
  private retryNotes = new Map<string, string>();
  /** The bot whose model is reasoning in a chat, until its next message or the end of its turn. */
  private thinkingBots = new Map<string, string>();

  /** Where each attachment's bytes are on this computer, and the address a page shows them from. */
  private attachmentFiles = new Map<string, { path: string; url: string }>();
  private fetchingAttachments = new Set<string>();

  // MARK: - Lifecycle

  start(): void {
    if (this.started) return;
    this.started = true;
    if (this.isMock) {
      this.replyEngine = new ReplyEngine(this);
      this.resetMockData();
      this.isConnected = true;
      this.finishStartup();
      this.hasIdentity = true;
      this.isIdentityDevice = true;
      this.emit({ kind: "connectionChanged" });
      this.emit({ kind: "identityChanged" });
      return;
    }

    // A slow or silent CLI leaves the user with the offline recovery controls. This deadline
    // bounds the loading state, never the time until a window is shown.
    this.startupTimer = setTimeout(() => {
      this.finishStartup();
      this.emit({ kind: "connectionChanged" });
    }, 2500);

    const transport = cliTransport();
    this.transport = transport;
    transport.onState((state) => this.cliStateChanged(state));
    transport.onEvent((name, data) => {
      if (this.isBootstrapping) this.bootstrapEvents.push({ name, data });
      else this.handle(name, data);
    });
    void transport.state().then((state) => this.cliStateChanged(state));
  }

  private cliStateChanged(state: CLIState): void {
    const wasConnected = this.cliState.connection === "connected";
    this.cliState = state;
    if (state.launcher.kind === "failed") this.finishStartup();
    if (state.connection === "connected") {
      if (!wasConnected) {
        this.bootstrapGeneration += 1;
        const generation = this.bootstrapGeneration;
        void this.bootstrap(generation);
      }
    } else {
      this.bootstrapGeneration += 1;
      this.isBootstrapping = true;
      this.bootstrapEvents = [];
      this.reportedWatchedChat = undefined;
      if (this.isConnected) {
        this.isConnected = false;
        this.runningJobs = [];
        this.thinkingBots.clear();
      }
    }
    this.emit({ kind: "connectionChanged" });
  }

  private finishStartup(): void {
    if (this.startupTimer) clearTimeout(this.startupTimer);
    this.startupTimer = null;
    this.isStarting = false;
  }

  reconnect(): void {
    if (this.isMock) {
      this.setConnected(true);
      return;
    }
    this.transport?.reconnect();
  }

  private async request<T>(method: string, params: Record<string, unknown> = {}): Promise<T> {
    if (!this.transport) throw new RequestError(L("The Lorca CLI is not running"));
    try {
      return (await this.transport.request(method, params)) as T;
    } catch (error) {
      throw new RequestError(errorText(error));
    }
  }

  private async bootstrap(generation: number): Promise<void> {
    try {
      // The snapshot includes identity and connection state as well as messages. Publish them
      // together, so roster events cannot expose empty previews before it arrives.
      const snapshot = await this.request<WireSnapshot>("bootstrap");
      if (generation !== this.bootstrapGeneration) return;
      this.isApplyingBootstrap = true;
      this.apply(snapshot);
      for (const event of this.bootstrapEvents) this.handle(event.name, event.data);
      this.bootstrapEvents = [];
      this.isApplyingBootstrap = false;
      this.isBootstrapping = false;
      this.isConnected = true;
      this.finishStartup();
      this.emit({ kind: "snapshotReplaced" });
      this.emit({ kind: "connectionChanged" });
      this.emit({ kind: "identityChanged" });
    } catch (error) {
      this.isApplyingBootstrap = false;
      if (generation !== this.bootstrapGeneration) return;
      this.finishStartup();
      this.emit({ kind: "connectionChanged" });
      console.error("bootstrap failed:", errorText(error));
    }
  }

  private apply(snapshot: WireSnapshot): void {
    this.hasIdentity = snapshot.has_identity;
    this.isIdentityDevice = snapshot.is_identity_device;
    this.identityID = snapshot.identity_id ?? null;
    this.relayURL = snapshot.relay_url ?? null;
    this.relayConnected = snapshot.relay_connected;
    this.relayUpdateRequired = snapshot.relay_update_required ?? false;
    this.relayError = snapshot.relay_error?.message ?? null;
    this.devices = snapshot.devices.map(toDevice);
    this.bots = snapshot.bots.map(toBot);
    // A snapshot carries each chat's newest messages. Older pages this app already loaded stay
    // ahead of them, so a resync does not throw the transcript back to the last page.
    const loaded = this.chats;
    this.chats = snapshot.chats.map((incoming) => {
      const chat = toChat(incoming);
      const existing = loaded.find((candidate) => candidate.id === chat.id);
      const first = chat.messages[0];
      if (existing && first) {
        const index = existing.messages.findIndex((message) => message.id === first.id);
        if (index > 0) return { ...chat, messages: [...existing.messages.slice(0, index), ...chat.messages], hasMore: existing.hasMore };
      }
      return chat;
    });
    this.routines = (snapshot.routines ?? []).map(toRoutine);
    this.autoReview = toAutoReview(snapshot.auto_review);
    this.providers = toProviders(snapshot.providers);
    this.models = toModels(snapshot.models);
    this.runningJobs = (snapshot.running_turns ?? []).map((turn) => ({
      id: turn.job_id,
      chatID: turn.chat_id,
      botID: turn.bot_id,
      routineID: turn.routine_id ?? undefined,
    }));
    for (const id of snapshot.running_chat_ids) {
      if (!this.runningJobs.some((job) => job.chatID === id)) this.runningJobs.push({ id: `chat:${id}`, chatID: id, botID: "" });
    }
    this.sortChats();
    for (const chat of this.chats) for (const message of chat.messages) this.noteCommand(message, chat.id);
    this.emit({ kind: "snapshotReplaced" });
  }

  // MARK: - Events from the CLI

  private handle(name: string, data: unknown): void {
    switch (name) {
      case "snapshot":
        this.apply(data as WireSnapshot);
        break;

      case "roster.changed": {
        const roster = data as WireRosterChanged;
        this.devices = roster.devices.map(toDevice);
        this.bots = roster.bots.map(toBot);
        if (roster.routines) this.routines = roster.routines.map(toRoutine);
        if (roster.auto_review) this.autoReview = toAutoReview(roster.auto_review);
        if (roster.providers) this.providers = toProviders(roster.providers);
        if (roster.models) this.models = toModels(roster.models);
        const changed: string[] = [];
        this.chats = roster.chats.map((summary) => {
          const existing = this.chat(summary.id);
          const chat = toChat(summary, {
            messages: existing?.messages ?? [],
            unreadCount: existing?.unreadCount ?? 0,
            hasMore: existing?.hasMore ?? false,
          });
          if (existing && (existing.botIDs.join() !== chat.botIDs.join() || existing.customTitle !== chat.customTitle)) {
            changed.push(chat.id);
          }
          return chat;
        });
        this.sortChats();
        this.emit({ kind: "rosterChanged" });
        this.emit({ kind: "chatsChanged" });
        for (const id of changed) this.emit({ kind: "chatChanged", chatID: id });
        break;
      }

      case "message.added":
      case "message.updated": {
        const payload = data as { chat_id: string; message: WireMessage };
        if (this.retryNotes.has(payload.chat_id)) {
          this.retryNotes.delete(payload.chat_id);
          this.emit({ kind: "respondingChanged", chatID: payload.chat_id });
        }
        // The thinking bot's next message (a tool call, a reply) is where its thinking went.
        const botID = payload.message.author.bot_id;
        if (name === "message.added" && botID && this.thinkingBots.get(payload.chat_id) === botID) {
          this.thinkingBots.delete(payload.chat_id);
        }
        this.upsert(toMessage(payload.message), payload.chat_id);
        break;
      }

      case "message.removed": {
        const payload = data as { chat_id: string; message_id: string };
        if (!this.chat(payload.chat_id)) return;
        this.updateChat(payload.chat_id, (chat) => ({ ...chat, messages: chat.messages.filter((message) => message.id !== payload.message_id) }));
        this.commandStarts.delete(payload.message_id);
        this.emit({ kind: "messageRemoved", chatID: payload.chat_id, messageID: payload.message_id });
        break;
      }

      case "chat.removed": {
        const payload = data as { chat_id: string };
        this.chats = this.chats.filter((chat) => chat.id !== payload.chat_id);
        this.runningJobs = this.runningJobs.filter((job) => job.chatID !== payload.chat_id);
        this.emit({ kind: "chatsChanged" });
        break;
      }

      case "job.started": {
        const job = data as WireJobEvent;
        // A snapshot may already list it.
        this.runningJobs = this.runningJobs.filter((running) => running.id !== `pending:${job.chat_id}` && running.id !== job.job_id);
        this.runningJobs.push({ id: job.job_id, chatID: job.chat_id, botID: job.bot_id, routineID: job.routine_id ?? undefined });
        if (!this.jobStarts.has(job.job_id)) this.jobStarts.set(job.job_id, Date.now());
        this.emit({ kind: "respondingChanged", chatID: job.chat_id });
        this.emit({ kind: "chatsChanged" });
        break;
      }

      case "job.finished": {
        const job = data as WireJobEvent;
        this.runningJobs = this.runningJobs.filter((running) => running.id !== job.job_id);
        this.retryNotes.delete(job.chat_id);
        if (!job.bot_id || this.thinkingBots.get(job.chat_id) === job.bot_id) this.thinkingBots.delete(job.chat_id);
        this.emit({ kind: "respondingChanged", chatID: job.chat_id });
        this.emit({ kind: "chatsChanged" });
        const startedAt = this.jobStarts.get(job.job_id);
        this.jobStarts.delete(job.job_id);
        if (startedAt !== undefined && job.bot_id) {
          this.emit({ kind: "turnFinished", chatID: job.chat_id, botID: job.bot_id, startedAt });
        }
        break;
      }

      case "job.retry": {
        const retry = data as WireJobRetry;
        const seconds = Math.max(1, Math.round(retry.delay_ms / 1000));
        this.retryNotes.set(retry.chat_id, L("Retrying (%d of %d) in %d s", retry.attempt, retry.max_attempts, seconds));
        this.emit({ kind: "respondingChanged", chatID: retry.chat_id });
        break;
      }

      case "job.thinking": {
        const job = data as { chat_id: string; bot_id: string };
        this.thinkingBots.set(job.chat_id, job.bot_id);
        this.emit({ kind: "respondingChanged", chatID: job.chat_id });
        break;
      }

      case "chat.usage": {
        const payload = data as { chat_id: string; usage: WireChatUsage };
        if (!this.chat(payload.chat_id)) return;
        this.updateChat(payload.chat_id, (chat) => ({ ...chat, usage: toUsage(payload.usage) }));
        this.emit({ kind: "chatChanged", chatID: payload.chat_id });
        break;
      }

      case "relay.status": {
        const status = data as WireRelayStatus;
        this.relayConnected = status.connected;
        this.relayUpdateRequired = status.update_required ?? false;
        this.relayError = status.error?.message ?? null;
        this.relayURL = status.url ?? this.relayURL;
        this.emit({ kind: "rosterChanged" });
        break;
      }

      case "identity.changed": {
        const payload = data as { has_identity: boolean };
        this.hasIdentity = payload.has_identity;
        this.emit({ kind: "identityChanged" });
        break;
      }
    }
  }

  private upsert(message: Message, chatID: string): void {
    const chat = this.chat(chatID);
    if (!chat) return;
    this.noteCommand(message, chatID);
    const index = chat.messages.findIndex((existing) => existing.id === message.id);
    if (index >= 0) {
      const messages = chat.messages.slice();
      messages[index] = message;
      this.replaceChat({ ...chat, messages });
      this.emit({ kind: "messageChanged", chatID, messageID: message.id });
      if (message.state.kind === "complete") this.refreshChatList();
    } else {
      this.replaceChat({ ...chat, messages: [...chat.messages, message] });
      this.emit({ kind: "messageAdded", chatID, messageID: message.id });
      this.sortChats();
      this.emit({ kind: "chatsChanged" });
    }
  }

  private replaceChat(chat: Chat): void {
    const index = this.chats.findIndex((existing) => existing.id === chat.id);
    if (index < 0) return;
    const chats = this.chats.slice();
    chats[index] = chat;
    this.chats = chats;
  }

  private updateChat(id: string, transform: (chat: Chat) => Chat): void {
    const chat = this.chat(id);
    if (chat) this.replaceChat(transform(chat));
  }

  private updateBot(id: string, transform: (bot: Bot) => Bot): void {
    this.bots = this.bots.map((bot) => (bot.id === id ? transform(bot) : bot));
  }

  // MARK: - Observation

  subscribe(listener: (event: StoreEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  private emit(event: StoreEvent): void {
    if (this.isApplyingBootstrap) return;
    for (const listener of [...this.listeners]) listener(event);
  }

  // MARK: - Lookup

  bot(id: string): Bot | undefined {
    return this.bots.find((bot) => bot.id === id);
  }

  device(id: string): Device | undefined {
    return this.devices.find((device) => device.id === id);
  }

  /** Devices that can be assigned bots: those with a desktop `os`. */
  get runners(): Device[] {
    return this.devices.filter(isRunner);
  }

  chat(id: string): Chat | undefined {
    return this.chats.find((chat) => chat.id === id);
  }

  botsIn(chat: Chat): Bot[] {
    return chat.botIDs.flatMap((id) => {
      const bot = this.bot(id);
      return bot ? [bot] : [];
    });
  }

  botsOn(runnerID: string): Bot[] {
    return this.bots.filter((bot) => bot.runnerID === runnerID);
  }

  routine(id: string): Routine | undefined {
    return this.routines.find((routine) => routine.id === id);
  }

  /** A bot's routines, oldest first, with the running state from the turns in flight. */
  routinesFor(botID: string): Routine[] {
    return this.routines
      .filter((routine) => routine.botID === botID)
      .sort((a, b) => a.createdAt - b.createdAt)
      .map((routine) => {
        const running = routine.isRunning || this.runningJobs.some((job) => job.routineID === routine.id);
        return running === routine.isRunning ? routine : { ...routine, isRunning: running };
      });
  }

  get thisDevice(): Device | undefined {
    return this.devices.find((device) => device.isThisDevice);
  }

  title(chat: Chat): string {
    if (isGroup(chat) && chat.customTitle) return chat.customTitle;
    const names = chat.botIDs.flatMap((id) => {
      const bot = this.bot(id);
      return bot ? [bot.name] : [];
    });
    return names.length === 0 ? L("New Chat") : names.join(", ");
  }

  subtitle(chat: Chat): string {
    const members = this.botsIn(chat);
    const only = members[0];
    if (isDM(chat) && only) {
      const host = this.device(only.runnerID)?.name ?? L("unassigned");
      return L("%@ on %@", providerName(only.provider, this.providers), host);
    }
    const hosts = new Set(members.flatMap((bot) => this.device(bot.runnerID)?.name ?? []));
    const runnerLabel = hosts.size === 1 ? [...hosts][0]! : L("%d Runners", hosts.size);
    const botLabel = members.length === 1 ? L("1 bot") : L("%d bots", members.length);
    return L("Group · %@ · %@", botLabel, runnerLabel);
  }

  preview(chat: Chat): string {
    // The last thing worth previewing: tool calls never are, except a sent message.
    let last: Message | undefined;
    for (let index = chat.messages.length - 1; index >= 0; index--) {
      const message = chat.messages[index]!;
      if (message.body.kind !== "tool" || isSentMessage(message.body.tool)) {
        last = message;
        break;
      }
    }
    if (!last) return L("No messages yet");
    let body: string;
    const content = last.body;
    switch (content.kind) {
      case "text":
        body = content.text === "" ? attachmentSummary(last.attachments) : content.text;
        break;
      case "tool":
        body = L("Messaged %@: %@", (content.tool.targetBotID && this.bot(content.tool.targetBotID)?.name) || L("a teammate"), content.tool.detail);
        break;
      case "handoff":
        body =
          !isGroup(chat) && chat.botIDs.includes(content.to)
            ? L("Message from %@: %@", this.bot(content.from)?.name ?? L("a teammate"), content.reason)
            : L("Handed off to %@", this.bot(content.to)?.name ?? L("a teammate"));
        break;
      case "notice":
        body = content.text;
        break;
      case "permission": {
        const who = this.bot(authorBotID(last.author) ?? "")?.name ?? L("A bot");
        body = `${who} ${verbPhrase(content.request)}`;
        break;
      }
    }
    const flattened = body.replaceAll("\n", " ").replaceAll("**", "").replaceAll("`", "").trim();
    if (isGroup(chat) && last.author.kind === "bot" && content.kind === "text") {
      return `${this.bot(last.author.botID)?.name ?? L("Bot")}: ${flattened}`;
    }
    return flattened;
  }

  // MARK: - Connection

  setConnected(connected: boolean): void {
    if (this.isConnected === connected) return;
    this.isConnected = connected;
    this.emit({ kind: "connectionChanged" });
  }

  // MARK: - Requests

  /** Fire-and-forget request. A failure is logged and the store re-syncs from the CLI, so an
   * optimistic change that the CLI rejected gets rolled back. */
  private perform(method: string, params: Record<string, unknown> = {}): void {
    if (this.isMock) return;
    void (async () => {
      try {
        await this.request(method, params);
      } catch (error) {
        console.error(`${method} failed:`, errorText(error));
        try {
          const snapshot = await this.request<WireSnapshot>("bootstrap");
          this.apply(snapshot);
        } catch {}
      }
    })();
  }

  // MARK: - Chat mutation

  private sortChats(): void {
    this.chats = this.chats.slice().sort((lhs, rhs) => {
      if (lhs.isPinned !== rhs.isPinned) return lhs.isPinned ? -1 : 1;
      const a = lastActivity(lhs);
      const b = lastActivity(rhs);
      if (a === b) return lhs.id < rhs.id ? -1 : lhs.id > rhs.id ? 1 : 0;
      return b - a;
    });
  }

  /** The one DM with this bot, created on first use. DMs are keyed by the bot, so opening one twice
   * lands in the same thread. */
  dm(botID: string): string {
    const existing = this.chats.find((chat) => isDM(chat) && chat.botIDs.length === 1 && chat.botIDs[0] === botID);
    if (existing) return existing.id;
    return this.createChat("dm", [botID], null);
  }

  createChat(kind: Chat["kind"], botIDs: string[], title: string | null): string {
    const members = kind === "dm" ? botIDs.slice(0, 1) : botIDs.slice(0, 6);
    const chat: Chat = {
      id: shortID("chat"),
      kind,
      customTitle: kind === "group" ? (title ?? undefined) : undefined,
      botIDs: members,
      messages: [],
      unreadCount: 0,
      isPinned: false,
      createdAt: Date.now(),
      hasMore: false,
      groupDescription: "",
    };
    this.chats = [chat, ...this.chats];
    this.sortChats();
    this.emit({ kind: "chatsChanged" });
    this.perform("chats.create", { id: chat.id, kind, bot_ids: members, title: title ?? "" });
    return chat.id;
  }

  createBot(options: {
    name: string;
    description?: string;
    symbolName: string;
    accent: Accent;
    runnerID: string;
    provider: ProviderKind;
    model?: string;
    thinking?: string;
    templateID?: string;
    greeting?: string;
  }): string {
    const bot: Bot = {
      id: shortID("bot"),
      name: options.name,
      description: options.description ?? "",
      symbolName: options.symbolName,
      accent: options.accent,
      runnerID: options.runnerID,
      provider: options.provider,
      model: options.model,
      thinking: options.thinking,
      createdAt: Date.now(),
    };
    this.bots = [...this.bots, bot];
    this.emit({ kind: "rosterChanged" });
    this.emit({ kind: "chatsChanged" });

    // The CLI gives every bot its direct chat; create it under the id the app will open.
    if (!this.isMock) {
      const chatID = shortID("chat");
      const chat: Chat = {
        id: chatID,
        kind: "dm",
        botIDs: [bot.id],
        messages: [],
        unreadCount: 0,
        isPinned: false,
        createdAt: Date.now(),
        hasMore: false,
        groupDescription: "",
      };
      this.chats = [chat, ...this.chats];
      this.sortChats();
      this.emit({ kind: "chatsChanged" });
      const params: Record<string, unknown> = {
        id: bot.id,
        name: options.name,
        description: options.description ?? "",
        symbol_name: options.symbolName,
        accent: options.accent,
        runner_id: options.runnerID,
        provider: options.provider,
        model: options.model ?? "",
        thinking: options.thinking ?? "",
        chat_id: chatID,
      };
      if (options.templateID) {
        params.template_id = options.templateID;
        params.greeting = options.greeting ?? "";
      }
      this.perform("bots.create", params);
    }
    return bot.id;
  }

  /** Adds a bot from a marketplace template on `runnerID` and answers with its direct chat. The CLI
   * gives it the template's routines, paused, and a first turn that answers the user's greeting. */
  addBotFromTemplate(template: BotTemplate, runnerID: string): string {
    const botID = this.createBot({
      name: template.name,
      description: template.description,
      symbolName: template.symbolName,
      accent: template.accent,
      runnerID,
      provider: this.preferredProvider,
      templateID: template.id,
      greeting: L("Hi %@, introduce yourself.", template.name),
    });
    return this.dm(botID);
  }

  /** The provider a bot made without asking runs with: the first one the account connected. */
  get preferredProvider(): ProviderKind {
    return this.providerKinds.find((kind) => this.credential(kind)?.isConnected) ?? "deepseek";
  }

  /** Every provider a bot can run with: the built-in ones, then the ones the user added. */
  get providerKinds(): ProviderKind[] {
    return [...providerKinds, ...this.providers.map((provider) => provider.kind).filter(isCustomKind)];
  }

  updateBotProfile(id: string, name: string, description?: string, provider?: ProviderKind): void {
    if (!this.bot(id)) return;
    this.updateBot(id, (bot) => ({
      ...bot,
      name,
      description: description ?? bot.description,
      provider: provider ?? bot.provider,
    }));
    this.emit({ kind: "rosterChanged" });
    this.emit({ kind: "chatsChanged" });
    const params: Record<string, unknown> = { id, name };
    if (description !== undefined) params.description = description;
    if (provider !== undefined) params.provider = provider;
    this.perform("bots.update", params);
  }

  /** The bot's symbol and accent, the look under and behind its image. */
  setBotLook(id: string, symbolName: string, accent: Accent): void {
    if (!this.bot(id)) return;
    this.updateBot(id, (bot) => ({ ...bot, symbolName, accent }));
    this.emit({ kind: "rosterChanged" });
    this.emit({ kind: "chatsChanged" });
    this.perform("bots.update", { id, symbol_name: symbolName, accent });
  }

  /** A custom profile image from a file on this computer (null removes the current one). The CLI
   * copies it into its store, uploads it as a `file` blob, and names it in the roster, which comes
   * back as the bot's `avatar` for every Device. */
  setBotAvatar(id: string, file: FileInfo | null): void {
    if (!this.bot(id)) return;
    if (file) {
      const attachment: Attachment = { id: attachmentID(), name: file.name, mime: file.mime.startsWith("image/") ? file.mime : "image/png", size: file.size };
      this.attachmentFiles.set(attachment.id, { path: file.path, url: file.url });
      this.updateBot(id, (bot) => ({ ...bot, avatar: attachment }));
      this.emit({ kind: "rosterChanged" });
      this.emit({ kind: "chatsChanged" });
      this.perform("bots.update", { id, avatar: { id: attachment.id, path: file.path, name: attachment.name, mime: attachment.mime } });
    } else {
      this.updateBot(id, (bot) => ({ ...bot, avatar: undefined }));
      this.emit({ kind: "rosterChanged" });
      this.emit({ kind: "chatsChanged" });
      this.perform("bots.update", { id, avatar: null });
    }
  }

  /** Provider, model, and thinking level a bot runs with. None means the provider's default. */
  setBotRuntime(id: string, provider: ProviderKind, model: string | undefined, thinking: string | undefined): void {
    if (!this.bot(id)) return;
    this.updateBot(id, (bot) => ({ ...bot, provider, model, thinking }));
    this.emit({ kind: "rosterChanged" });
    this.emit({ kind: "chatsChanged" });
    for (const chat of this.chats) if (chat.botIDs.includes(id)) this.emit({ kind: "chatChanged", chatID: chat.id });
    this.perform("bots.update", { id, provider, model: model ?? "", thinking: thinking ?? "" });
  }

  /** Summarizes the chat's older part for its bot now, on the Runner. */
  compactChat(id: string): void {
    this.perform("chats.compact", { chat_id: id });
  }

  // MARK: - Plugins

  private mockMarketplace: Marketplace | null = null;

  /** The marketplace: its plugins, each with the Runners that have it, and its bots. */
  async marketplace(): Promise<Marketplace> {
    if (this.isMock) {
      const { mockMarketplace } = await import("./mock");
      this.mockMarketplace ??= await mockMarketplace();
      return this.mockMarketplace;
    }
    return toMarketplace(await this.request("marketplace"));
  }

  /** Installs a marketplace plugin on a Runner (here, or sealed to that Runner). */
  async installPlugin(pluginID: string, runnerID: string): Promise<InstalledPlugin> {
    if (this.isMock) return { id: pluginID, name: pluginID, description: "", version: "", icon: "", state: "ready", detail: "Ready" };
    const reply = await this.request<{ status: Parameters<typeof toPlugin>[0] }>("plugins.install", { runner_id: runnerID, plugin_id: pluginID });
    return toPlugin(reply.status);
  }

  async uninstallPlugin(pluginID: string, runnerID: string): Promise<void> {
    if (this.isMock) return;
    await this.request("plugins.uninstall", { runner_id: runnerID, plugin_id: pluginID });
  }

  async pluginDetail(pluginID: string, runnerID: string): Promise<PluginDetail> {
    if (this.isMock) {
      const status = this.device(runnerID)?.plugins.find((plugin) => plugin.id === pluginID) ?? {
        id: pluginID,
        name: pluginID,
        description: "",
        version: "",
        icon: "",
        state: "ready" as const,
        detail: "Ready",
      };
      return {
        status,
        variables: [{ name: "GITHUB_TOKEN", description: "A personal access token, instead of signing in.", secret: true, required: false, isSet: false }],
        servers: [{ name: "github", kind: "http", url: "https://api.githubcopilot.com/mcp/", oauth: true, signedIn: status.state === "ready" }],
        skills: [],
      };
    }
    return toPluginDetail(await this.request("plugins.detail", { runner_id: runnerID, plugin_id: pluginID }));
  }

  /** Sets variables on the Runner; a secret goes out in the request and is never read back. */
  async setPluginVariables(pluginID: string, runnerID: string, variables: Record<string, string>): Promise<InstalledPlugin> {
    if (this.isMock) return { id: pluginID, name: pluginID, description: "", version: "", icon: "", state: "ready", detail: "Ready" };
    const reply = await this.request<{ status: Parameters<typeof toPlugin>[0] }>("plugins.set_variables", {
      runner_id: runnerID,
      plugin_id: pluginID,
      variables,
    });
    return toPlugin(reply.status);
  }

  /** Starts a plugin's sign-in for the Runner; the browser opens on this computer. */
  async connectPlugin(pluginID: string, runnerID: string): Promise<void> {
    if (this.isMock) return;
    await this.request("plugins.connect", { runner_id: runnerID, plugin_id: pluginID });
  }

  /** Forgets a plugin server's sign-in on its Runner. Nothing is revoked at the server; the
   * plugin's next use asks for a sign-in again. */
  async signOutPlugin(pluginID: string, runnerID: string, server: string): Promise<void> {
    if (this.isMock) return;
    await this.request("plugins.sign_out", { runner_id: runnerID, plugin_id: pluginID, server });
  }

  // MARK: - MCP servers

  /** The demo's mcp.json files, by Runner. */
  private mockMcp = new Map<string, McpServer[]>();

  private async mockMcpServers(runnerID: string): Promise<McpServer[]> {
    if (!this.mockMcp.has(runnerID)) {
      const { mcpServers } = await import("./mock");
      this.mockMcp.set(runnerID, runnerID === "dev-workbench" ? mcpServers() : []);
    }
    return this.mockMcp.get(runnerID)!;
  }

  /** The demo's Runner takes its servers as the CLI would: the ones that run are its plugins. */
  private setMockMcpServers(runnerID: string, servers: McpServer[]): void {
    this.mockMcp.set(runnerID, servers);
    const device = this.device(runnerID);
    if (!device) return;
    const plugins = [...device.plugins.filter((plugin) => plugin.source !== "mcp.json"), ...servers.flatMap((server) => (server.enabled && server.status ? [server.status] : []))];
    this.devices = this.devices.map((each) => (each.id === runnerID ? { ...each, plugins } : each));
    this.emit({ kind: "rosterChanged" });
  }

  /** A Runner's mcp.json: every server in it, usable or not, here or sealed to that Runner. */
  async mcpServers(runnerID: string): Promise<McpFile> {
    if (this.isMock) return { path: "~/.lorca/mcp.json", servers: await this.mockMcpServers(runnerID) };
    return toMcpFile(await this.request("mcp.list", { runner_id: runnerID }));
  }

  /** One server with the tools it offered when it last connected. */
  async mcpServer(name: string, runnerID: string): Promise<McpServer> {
    if (this.isMock) {
      const server = (await this.mockMcpServers(runnerID)).find((each) => each.name === name);
      if (!server) throw new RequestError(L("No server named %@ in mcp.json.", name));
      return server;
    }
    return toMcpServer((await this.request<{ server: WireMcpServer }>("mcp.get", { runner_id: runnerID, name })).server);
  }

  /** Adds a server, or saves the one `previousName` names under `name`. The CLI checks the entry
   * and writes it to the Runner's mcp.json, where the server starts once to list its tools. */
  async saveMcpServer(runnerID: string, name: string, entry: McpEntry, previousName?: string): Promise<McpServer> {
    if (this.isMock) {
      const servers = await this.mockMcpServers(runnerID);
      if ((previousName === undefined || previousName !== name) && servers.some((each) => each.name === name)) {
        throw new RequestError(L("mcp.json already has a server named %@.", name));
      }
      const id = name.toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "server";
      const remote = typeof entry.url === "string";
      const status: InstalledPlugin = { id, name, description: entry.description ?? "", version: "", icon: remote ? "globe" : "terminal", state: "ready", detail: "Ready", source: "mcp.json" };
      const enabled = entry.disabled !== true;
      const server: McpServer = { name, id, enabled, entry, status: enabled ? status : undefined, signsIn: false, signedIn: false, toolCount: 2, tools: [{ name: "echo", description: "Echo back what you send.", readOnly: true }, { name: "write_note", description: "Write a note.", readOnly: false }] };
      const index = servers.findIndex((each) => each.name === (previousName ?? name));
      this.setMockMcpServers(runnerID, index >= 0 ? servers.map((each, at) => (at === index ? server : each)) : [...servers, server]);
      return server;
    }
    const reply = await this.request<{ server: WireMcpServer }>("mcp.save", { runner_id: runnerID, name, config: entry, ...(previousName ? { previous_name: previousName } : {}) });
    return toMcpServer(reply.server);
  }

  /** Removes a server from the Runner's mcp.json, with its sign-in. */
  async removeMcpServer(runnerID: string, name: string): Promise<void> {
    if (this.isMock) {
      this.setMockMcpServers(runnerID, (await this.mockMcpServers(runnerID)).filter((each) => each.name !== name));
      return;
    }
    await this.request("mcp.remove", { runner_id: runnerID, name });
  }

  /** Turns a server on or off; off, no bot sees it and it never starts. */
  async setMcpServerEnabled(runnerID: string, name: string, enabled: boolean): Promise<McpServer> {
    if (this.isMock) {
      const servers = await this.mockMcpServers(runnerID);
      const updated = servers.map((each) => {
        if (each.name !== name) return each;
        const entry = { ...each.entry };
        if (enabled) delete entry.disabled;
        else entry.disabled = true;
        const status: InstalledPlugin = each.status ?? { id: each.id, name, description: entry.description ?? "", version: "", icon: typeof entry.url === "string" ? "globe" : "terminal", state: "ready", detail: "Ready", source: "mcp.json" };
        return { ...each, enabled, entry, status: enabled ? status : undefined };
      });
      this.setMockMcpServers(runnerID, updated);
      return updated.find((each) => each.name === name)!;
    }
    return toMcpServer((await this.request<{ server: WireMcpServer }>("mcp.set_enabled", { runner_id: runnerID, name, enabled })).server);
  }

  /** Connects a server and waits for it: from scratch (`fresh`), or taking a connection it has or
   * one under way. Its state says how it went. */
  async reconnectMcpServer(runnerID: string, name: string, fresh = true): Promise<McpServer> {
    if (this.isMock) {
      await new Promise((resolve) => setTimeout(resolve, 900));
      return this.mcpServer(name, runnerID);
    }
    return toMcpServer((await this.request<{ server: WireMcpServer }>("mcp.reconnect", { runner_id: runnerID, name, fresh })).server);
  }

  /** Reads the Runner's mcp.json again, after an edit made outside Lorca, and answers the file as
   * it reads now. */
  async reloadMcpServers(runnerID: string): Promise<McpFile> {
    if (this.isMock) return this.mcpServers(runnerID);
    return toMcpFile(await this.request("mcp.reload", { runner_id: runnerID }));
  }

  /** Offers one of a server's tools to bots, or keeps it from them, in the Runner's mcp.json. The
   * server keeps its connection. */
  async setMcpToolHidden(runnerID: string, name: string, tool: string, hidden: boolean): Promise<McpServer> {
    if (this.isMock) {
      const servers = await this.mockMcpServers(runnerID);
      const server = servers.find((each) => each.name === name);
      if (!server) throw new RequestError(L("No server named %@ in mcp.json.", name));
      const changed: McpServer = { ...server, tools: server.tools?.map((each) => (each.name === tool ? { ...each, hidden } : each)) };
      this.setMockMcpServers(runnerID, servers.map((each) => (each.name === name ? changed : each)));
      return changed;
    }
    return toMcpServer((await this.request<{ server: WireMcpServer }>("mcp.hide_tool", { runner_id: runnerID, name, tool, hidden })).server);
  }

  /** Forgets a remote server's sign-in on its Runner; its next use asks for one again. */
  async signOutMcpServer(runnerID: string, name: string): Promise<McpServer> {
    if (this.isMock) {
      const servers = await this.mockMcpServers(runnerID);
      const server = servers.find((each) => each.name === name);
      if (!server) throw new RequestError(L("No server named %@ in mcp.json.", name));
      const signedOut: McpServer = { ...server, signedIn: false, status: server.status && { ...server.status, state: "needs_auth", detail: "Sign in" } };
      this.setMockMcpServers(runnerID, servers.map((each) => (each.name === name ? signedOut : each)));
      return signedOut;
    }
    return toMcpServer((await this.request<{ server: WireMcpServer }>("mcp.sign_out", { runner_id: runnerID, name })).server);
  }

  /** The servers pasted JSON holds, in any app's spelling; the CLI on this computer reads it. */
  async parseMcpJSON(text: string): Promise<ParsedServer[]> {
    if (this.isMock) return parseLocally(text);
    return toParsedServers(await this.request("mcp.parse", { text }));
  }

  /** Replaces Auto-review (the switch and the rules); the change shows at once and the CLI's roster
   * event confirms it. A new rule gets its id from the CLI. */
  setAutoReview(value: AutoReview): void {
    this.autoReview = value;
    this.emit({ kind: "rosterChanged" });
    this.perform("auto_review.set", {
      is_enabled: value.isEnabled,
      rules: value.rules.map((rule) => ({ id: rule.id, text: rule.text, behavior: rule.behavior, ...(rule.tool ? { tool: rule.tool } : {}) })),
    });
  }

  /** Answers a question: a permission card's, or a command card's. `allow`, `always`, or `deny`.
   * The CLI confirms with the card's new state. */
  answerPermission(chatID: string, messageID: string, decision: string): void {
    this.update(messageID, chatID, (message) => {
      const body = message.body;
      if (body.kind === "permission") {
        const request = { ...body.request, decision: decision === "always" ? ("always" as const) : decision === "deny" ? ("denied" as const) : ("allowed" as const) };
        if (request.tool === "connect" && request.decision === "allowed") request.summary = L("Starting the sign-in…");
        return { ...message, body: { kind: "permission", request } };
      }
      if (body.kind === "tool" && body.tool.run?.state === "asking") {
        const run = { ...body.tool.run, state: (decision === "deny" ? "denied" : "running") as CommandState };
        return { ...message, body: { kind: "tool", tool: { ...body.tool, run } } };
      }
      return message;
    });
    this.perform("chats.permission", { chat_id: chatID, message_id: messageID, decision });
  }

  // MARK: - Commands

  /** Types the user's answer into a command running in its terminal, then Return. The CLI writes it
   * to the terminal, or seals it to the bot's Runner, and keeps nothing. Throws why it could not,
   * such as that Runner being offline. */
  async answerCommand(chatID: string, messageID: string, text: string): Promise<void> {
    if (this.isMock) {
      this.finishMockCommand(chatID, messageID, "exited");
      return;
    }
    await this.request("bash.stdin", { chat_id: chatID, message_id: messageID, text });
  }

  /** Stops a running command; its card leaves once the Runner has. */
  async stopCommand(chatID: string, messageID: string): Promise<void> {
    if (this.isMock) {
      this.finishMockCommand(chatID, messageID, "stopped");
      return;
    }
    await this.request("bash.stop", { chat_id: chatID, message_id: messageID });
  }

  /** Sends a command the bot is waiting on to the background (`bash.background`): the bot's call
   * returns and the command runs on, out of the way of Stop in the chat. */
  async sendCommandToBackground(chatID: string, messageID: string): Promise<void> {
    if (this.isMock) {
      this.update(messageID, chatID, (message) => {
        if (message.body.kind !== "tool" || !message.body.tool.run) return message;
        const run = { ...message.body.tool.run, background: true };
        return { ...message, body: { kind: "tool", tool: { ...message.body.tool, isRunning: false, run } } };
      });
      return;
    }
    await this.request("bash.background", { chat_id: chatID, message_id: messageID });
  }

  /** The commands in `chatID` that a bot's call still waits on, which Run in Background sends
   * there. */
  foregroundCommands(chatID: string): Message[] {
    return this.chat(chatID)?.messages.filter(runsInForeground) ?? [];
  }

  /** The demo has no Runner: an answer or a Stop ends the command at once. */
  private finishMockCommand(chatID: string, messageID: string, state: CommandState): void {
    this.update(messageID, chatID, (message) => {
      if (message.body.kind !== "tool" || !message.body.tool.run) return message;
      const run = { ...message.body.tool.run, state, prompt: undefined };
      return { ...message, body: { kind: "tool", tool: { ...message.body.tool, run } } };
    });
  }

  // MARK: - Routines

  /** Pauses or resumes a routine. A resumed schedule counts from now. */
  setRoutineEnabled(id: string, enabled: boolean): void {
    if (!this.routine(id)) return;
    this.routines = this.routines.map((routine) =>
      routine.id === id ? { ...routine, isEnabled: enabled, pausedReason: undefined, nextRunAt: enabled ? routine.nextRunAt : undefined } : routine,
    );
    this.emit({ kind: "rosterChanged" });
    this.perform("routines.update", { id, enabled });
  }

  /** Runs the routine now, on its bot's Runner. */
  runRoutine(id: string): void {
    if (!this.routine(id)) return;
    this.routines = this.routines.map((routine) => (routine.id === id ? { ...routine, isRunning: true } : routine));
    this.emit({ kind: "rosterChanged" });
    this.perform("routines.run", { id });
  }

  deleteRoutine(id: string): void {
    this.routines = this.routines.filter((routine) => routine.id !== id);
    this.emit({ kind: "rosterChanged" });
    this.perform("routines.delete", { id });
  }

  /** A bot's memory, read from its Runner. Answers with `here == false` when the bot runs on another
   * Device, whose disk this computer cannot read. */
  async botMemory(id: string): Promise<BotMemory> {
    if (this.isMock) {
      return {
        botID: id,
        here: true,
        runner: "This computer",
        path: `~/.lorca/workspaces/${id}`,
        text: "- 2026-09-10 · from your chat with the user · the user prefers short replies\n- 2026-09-12 · invoices are reconciled on Mondays\n",
        hash: "mock",
        lines: 2,
        bytes: 128,
        truncated: false,
        maxLines: 200,
        maxBytes: 24_000,
        topics: ["clients.md"],
        logs: ["2026-09-12", "2026-09-15"],
      };
    }
    return toBotMemory(await this.request("bots.memory", { bot_id: id }));
  }

  /** Replaces the bot's MEMORY.md. With `expectedHash`, the write is refused when the file changed
   * since it was read, so an edit never silently overwrites what the bot wrote. */
  async writeBotMemory(id: string, text: string, expectedHash: string | null): Promise<string> {
    if (this.isMock) return "mock";
    const params: Record<string, unknown> = { bot_id: id, text };
    if (expectedHash !== null) params.expected_hash = expectedHash;
    const reply = await this.request<{ hash: string }>("bots.memory.write", params);
    return reply.hash;
  }

  retryNote(chatID: string): string | undefined {
    return this.retryNotes.get(chatID);
  }

  isThinking(botID: string, chatID: string): boolean {
    return this.thinkingBots.get(chatID) === botID;
  }

  deleteChat(id: string): void {
    const chat = this.chat(id);
    if (!chat) return;

    // A bot owns its DM, so deleting that row deletes the bot as one roster operation. Groups keep
    // their other members; a group with nobody left is removed too.
    const botID = chat.botIDs[0];
    if (isDM(chat) && botID && this.bot(botID)) {
      for (const related of this.chats) if (related.botIDs.includes(botID)) this.replyEngine?.cancel(related.id);
      const removed = new Set<string>();
      const changed: string[] = [];
      this.chats = this.chats.flatMap((existing) => {
        if (!existing.botIDs.includes(botID)) return [existing];
        if (isDM(existing)) {
          removed.add(existing.id);
          return [];
        }
        const botIDs = existing.botIDs.filter((member) => member !== botID);
        if (botIDs.length === 0) {
          removed.add(existing.id);
          return [];
        }
        changed.push(existing.id);
        return [{ ...existing, botIDs }];
      });
      this.bots = this.bots.filter((bot) => bot.id !== botID);
      this.routines = this.routines.filter((routine) => routine.botID !== botID);
      const cancelled = this.runningJobs.filter((job) => job.botID === botID || removed.has(job.chatID));
      this.runningJobs = this.runningJobs.filter((job) => !cancelled.includes(job));
      for (const job of cancelled) this.jobStarts.delete(job.id);
      for (const chatID of removed) this.retryNotes.delete(chatID);
      for (const [chatID, thinking] of [...this.thinkingBots]) {
        if (thinking === botID || removed.has(chatID)) this.thinkingBots.delete(chatID);
      }
      this.emit({ kind: "rosterChanged" });
      for (const chatID of changed) this.emit({ kind: "chatChanged", chatID });
      this.emit({ kind: "chatsChanged" });
      this.perform("bots.delete", { id: botID });
      return;
    }

    this.replyEngine?.cancel(id);
    this.chats = this.chats.filter((existing) => existing.id !== id);
    const cancelled = this.runningJobs.filter((job) => job.chatID === id);
    this.runningJobs = this.runningJobs.filter((job) => job.chatID !== id);
    for (const job of cancelled) this.jobStarts.delete(job.id);
    this.retryNotes.delete(id);
    this.thinkingBots.delete(id);
    this.emit({ kind: "chatsChanged" });
    this.perform("chats.delete", { chat_id: id });
  }

  togglePin(id: string): void {
    const chat = this.chat(id);
    if (!chat) return;
    this.replaceChat({ ...chat, isPinned: !chat.isPinned });
    this.sortChats();
    this.emit({ kind: "chatsChanged" });
    this.perform("chats.pin", { chat_id: id, pinned: !chat.isPinned });
  }

  /** Asks the CLI for the page of messages before the chat's first one. The transcript calls this
   * as it nears the top; one request per chat at a time. */
  loadOlderMessages(id: string): void {
    const chat = this.chat(id);
    const first = chat?.messages[0];
    if (this.isMock || this.loadingOlder.has(id) || !chat || !chat.hasMore || !first) return;
    this.loadingOlder.add(id);
    void (async () => {
      try {
        const page = await this.request<WireMessagePage>("chats.messages", { chat_id: id, before: first.id });
        const current = this.chat(id);
        if (!current || current.messages[0]?.id !== first.id) return;
        const known = new Set(current.messages.map((message) => message.id));
        const older = page.messages.map(toMessage).filter((message) => !known.has(message.id));
        this.replaceChat({ ...current, messages: [...older, ...current.messages], hasMore: page.has_more });
        for (const message of older) this.noteCommand(message, id);
        this.emit({ kind: "olderMessagesLoaded", chatID: id });
      } catch {
      } finally {
        this.loadingOlder.delete(id);
      }
    })();
  }

  /** Full-text chat and message matches from the local SQLite index. */
  async searchChats(query: string): Promise<WireSearchResults> {
    if (this.isMock) return { chats: [], messages: [] };
    return this.request<WireSearchResults>("chats.search", { query, limit: 24 });
  }

  /** Tells the CLI which chat the user is looking at (null when none, or the app is not in front),
   * so a reply they watch arrive is not pushed to their phone. */
  setWatchedChat(id: string | null): void {
    if (this.isMock || !this.isConnected || this.reportedWatchedChat === id) return;
    this.reportedWatchedChat = id;
    void this.request("ui.watching", { chat_id: id }).catch(() => {});
  }

  markRead(id: string): void {
    const chat = this.chat(id);
    if (!chat || chat.unreadCount <= 0) return;
    this.replaceChat({ ...chat, unreadCount: 0 });
    this.emit({ kind: "chatsChanged" });
    this.perform("chats.mark_read", { chat_id: id });
  }

  rename(id: string, title: string): void {
    const chat = this.chat(id);
    if (!chat || !isGroup(chat)) return;
    const trimmed = title.trim();
    this.replaceChat({ ...chat, customTitle: trimmed });
    this.emit({ kind: "chatChanged", chatID: id });
    this.emit({ kind: "chatsChanged" });
    this.perform("chats.rename", { chat_id: id, title: trimmed });
  }

  /** What a group is for; every member reads it in its system prompt. */
  setDescription(text: string, chatID: string): void {
    const chat = this.chat(chatID);
    const trimmed = text.trim();
    if (!chat || !isGroup(chat) || chat.groupDescription === trimmed) return;
    this.replaceChat({ ...chat, groupDescription: trimmed });
    this.emit({ kind: "chatChanged", chatID });
    this.perform("chats.set_description", { chat_id: chatID, description: trimmed });
  }

  addBot(botID: string, chatID: string): void {
    const chat = this.chat(chatID);
    if (!chat || !canAddBot(chat) || chat.botIDs.includes(botID)) return;
    this.replaceChat({ ...chat, botIDs: [...chat.botIDs, botID] });
    if (this.isMock) {
      const name = this.bot(botID)?.name ?? "A bot";
      this.append({ id: newMessageID(), author: { kind: "system" }, body: { kind: "notice", text: `${name} joined the chat.` }, state: { kind: "complete" }, createdAt: Date.now(), attachments: [] }, chatID);
    }
    this.emit({ kind: "chatChanged", chatID });
    this.emit({ kind: "chatsChanged" });
    this.perform("chats.add_bot", { chat_id: chatID, bot_id: botID });
  }

  removeBot(botID: string, chatID: string): void {
    const chat = this.chat(chatID);
    if (!chat || !canRemoveBot(chat)) return;
    this.replaceChat({ ...chat, botIDs: chat.botIDs.filter((member) => member !== botID) });
    this.emit({ kind: "chatChanged", chatID });
    this.emit({ kind: "chatsChanged" });
    this.perform("chats.remove_bot", { chat_id: chatID, bot_id: botID });
  }

  /** Makes a group member the bot holding the work. */
  setOwner(botID: string, chatID: string): void {
    const chat = this.chat(chatID);
    if (!chat || !isGroup(chat) || !chat.botIDs.includes(botID) || chatOwner(chat) === botID) return;
    this.replaceChat({ ...chat, ownerBotID: botID });
    this.emit({ kind: "chatChanged", chatID });
    this.perform("chats.set_owner", { chat_id: chatID, bot_id: botID });
  }

  // MARK: - Messages

  append(message: Message, chatID: string): string | undefined {
    const chat = this.chat(chatID);
    if (!chat) return undefined;
    this.replaceChat({ ...chat, messages: [...chat.messages, message] });
    this.noteCommand(message, chatID);
    this.emit({ kind: "messageAdded", chatID, messageID: message.id });
    this.sortChats();
    this.emit({ kind: "chatsChanged" });
    return message.id;
  }

  /** Called for every streamed step, so this stays off the chat-list path. The sidebar preview
   * refreshes when a step finishes instead. */
  update(messageID: string, chatID: string, transform: (message: Message) => Message): void {
    const chat = this.chat(chatID);
    const index = chat?.messages.findIndex((message) => message.id === messageID) ?? -1;
    if (!chat || index < 0) return;
    const messages = chat.messages.slice();
    const updated = transform(messages[index]!);
    messages[index] = updated;
    this.replaceChat({ ...chat, messages });
    this.noteCommand(updated, chatID);
    this.emit({ kind: "messageChanged", chatID, messageID });
  }

  refreshChatList(): void {
    this.emit({ kind: "chatsChanged" });
  }

  /** Sends the message and returns the chat it landed in. `mentions` are the bots picked from the `@`
   * menu, which the CLI hands the bot by id; `replyTo` is the message the user answers, which the bot
   * reads quoted. */
  send(text: string, attachments: OutgoingAttachment[], mentions: string[], chatID: string, replyTo?: string): string {
    const trimmed = text.trim();
    const chat = this.chat(chatID);
    if ((trimmed === "" && attachments.length === 0) || !chat) return chatID;
    const original = replyTo ? chat.messages.find((message) => message.id === replyTo) : undefined;
    const quote = original ? quoteOf(original) : undefined;

    // The files are known here already; the CLI keeps the ids the bubble shows.
    for (const outgoing of attachments) this.attachmentFiles.set(outgoing.attachment.id, { path: outgoing.path, url: outgoing.url });
    const message: Message = {
      id: newMessageID(),
      author: you,
      body: { kind: "text", text: trimmed },
      state: { kind: "complete" },
      createdAt: Date.now(),
      attachments: attachments.map((outgoing) => outgoing.attachment),
      replyTo: quote,
    };
    this.append(message, chatID);

    if (this.isMock) {
      this.replyEngine?.respond(trimmed, chat, message.id);
      return chatID;
    }

    // Expect a turn to start; the CLI's job events confirm or clear this. A DM names its bot right
    // away so the working row appears with the send.
    if (chat.botIDs.length > 0) {
      const pendingID = `pending:${chatID}`;
      this.runningJobs.push({ id: pendingID, chatID, botID: isDM(chat) ? chat.botIDs[0]! : "" });
      this.emit({ kind: "respondingChanged", chatID });
      this.emit({ kind: "chatsChanged" });
      setTimeout(() => {
        if (!this.runningJobs.some((job) => job.id === pendingID)) return;
        // Nothing started (no Runner answered); stop showing the bot at work.
        this.runningJobs = this.runningJobs.filter((job) => job.id !== pendingID);
        this.emit({ kind: "respondingChanged", chatID });
        this.emit({ kind: "chatsChanged" });
      }, 4000);
    }
    this.perform("chats.send", {
      chat_id: chatID,
      text: trimmed,
      message_id: message.id,
      mentions,
      attachments: attachments.map((outgoing) => ({
        id: outgoing.attachment.id,
        path: outgoing.path,
        name: outgoing.attachment.name,
        mime: outgoing.attachment.mime,
        width: outgoing.attachment.width ?? null,
        height: outgoing.attachment.height ?? null,
      })),
      ...(quote ? { reply_to: quote.messageID } : {}),
    });
    return chatID;
  }

  // MARK: - Attachments

  /** The bot's profile image once this computer has it. The first ask for one that is not here
   * fetches the blob and redraws the roster when it lands; until then callers draw the symbol and
   * accent. */
  avatarURL(bot: Bot): string | undefined {
    const attachment = bot.avatar;
    if (!attachment) return undefined;
    const known = this.attachmentFiles.get(attachment.id);
    if (known) return known.url || undefined;
    this.fetchAttachment(attachment, () => {
      this.emit({ kind: "rosterChanged" });
      this.emit({ kind: "chatsChanged" });
      for (const chat of this.chats) if (chat.botIDs.includes(bot.id)) this.emit({ kind: "chatChanged", chatID: chat.id });
    });
    return undefined;
  }

  /** Where an attachment's bytes are on this computer. A file sent from here is known at once; one
   * sent from another Device is fetched through the CLI, and the message reloads when it lands. */
  localFile(attachment: Attachment, chatID: string, messageID: string): { path: string; url: string } | undefined {
    const known = this.attachmentFiles.get(attachment.id);
    if (known) return known;
    this.fetchAttachment(attachment, () => this.emit({ kind: "messageChanged", chatID, messageID }));
    return undefined;
  }

  private fetchAttachment(attachment: Attachment, landed: () => void): void {
    if (this.isMock || this.fetchingAttachments.has(attachment.id)) return;
    this.fetchingAttachments.add(attachment.id);
    // Off the caller's frame: a view asks while it renders, and the answer changes the store.
    queueMicrotask(async () => {
      try {
        const reply = await this.request<{ path: string }>("files.path", {
          attachment: { id: attachment.id, name: attachment.name, mime: attachment.mime, size: attachment.size },
        });
        const { files } = await import("../host");
        this.attachmentFiles.set(attachment.id, { path: reply.path, url: await files.url(reply.path) });
        landed();
      } catch (error) {
        // Left in the fetching set: the relay does not have it, and every scroll would ask again.
        // A relaunch retries.
        console.error(`fetching ${attachment.name} failed:`, errorText(error));
      }
    });
  }

  isResponding(chatID: string): boolean {
    return this.runningJobs.some((job) => job.chatID === chatID);
  }

  /** The chat's running tasks: its commands running in their terminals, here or on their Runners,
   * in the order they started, once each has run for `taskDelay`. */
  runningCommands(chatID: string): Message[] {
    const now = Date.now();
    return (
      this.chat(chatID)?.messages.filter((message) => {
        const run = commandRunOf(message);
        if (!run || !takesInput(run)) return false;
        // One that was running before this app heard of it has run long enough.
        return now - (this.commandStarts.get(message.id) ?? 0) >= AppStore.taskDelay;
      }) ?? []
    );
  }

  /** Notes when a command starts running in its terminal, and tells the observers once it has run
   * for `taskDelay`. */
  private noteCommand(message: Message, chatID: string): void {
    const run = commandRunOf(message);
    if (!run || !takesInput(run)) {
      this.commandStarts.delete(message.id);
      return;
    }
    if (this.commandStarts.has(message.id)) return;
    this.commandStarts.set(message.id, Date.now());
    setTimeout(() => this.emit({ kind: "runningTasksChanged", chatID }), AppStore.taskDelay);
  }

  /** Bots with a turn running in this chat, in the order they started. */
  workingBots(chatID: string): string[] {
    const seen: string[] = [];
    for (const job of this.runningJobs) {
      if (job.chatID === chatID && job.botID && !seen.includes(job.botID)) seen.push(job.botID);
    }
    return seen;
  }

  /** Whether the bot has a turn running anywhere: the green dot on its avatar. */
  isWorking(botID: string): boolean {
    return this.runningJobs.some((job) => job.botID === botID);
  }

  /** The mock reply engine's turns, so the demo shows the same working state as the CLI. */
  setMockWorking(botID: string, chatID: string, working: boolean): void {
    const id = `mock:${chatID}:${botID}`;
    this.runningJobs = this.runningJobs.filter((job) => job.id !== id);
    if (working) this.runningJobs.push({ id, chatID, botID });
    this.emit({ kind: "respondingChanged", chatID });
    this.emit({ kind: "chatsChanged" });
  }

  /** Has the bot's turn read a message it holds for its next step now: a command it waits on goes
   * to the background, and a reply in progress stops where it got to. */
  sendNow(messageID: string, chatID: string): void {
    if (this.isMock) {
      this.replyEngine?.sendNow(chatID);
      return;
    }
    this.perform("chats.send_now", { chat_id: chatID, message_id: messageID });
  }

  /** Marks a message the mock turn holds, or no longer holds. */
  setMockQueued(messageID: string, chatID: string, queued: boolean): void {
    this.update(messageID, chatID, (message) => ({ ...message, queued }));
  }

  stopResponding(chatID: string): void {
    if (this.isMock) {
      this.replyEngine?.cancel(chatID);
      return;
    }
    this.perform("chats.stop", { chat_id: chatID });
  }

  // MARK: - Identity, pairing, providers

  async createIdentity(): Promise<string[]> {
    const created = await this.request<{ phrase: string[] }>("identity.create");
    this.hasIdentity = true;
    this.isIdentityDevice = true;
    this.emit({ kind: "identityChanged" });
    return created.phrase;
  }

  /** Unpairs another Device. The CLI has the relay drop its key; the Device wipes its copy of the
   * account the next time it connects. Throws when the relay could not be told. */
  async unpairDevice(id: string): Promise<void> {
    if (!this.isMock) await this.request("device.unpair", { id });
    this.devices = this.devices.filter((device) => device.id !== id);
    this.emit({ kind: "rosterChanged" });
  }

  /** Has a self-updating CLI, this Device's or another's through the relay, install the latest
   * release, which it restarts into once no bot is at work there. Its `machine` blob brings the
   * state back; the demo, which has no CLI, waits to restart. Throws with the CLI's reason. */
  async updateDevice(id: string): Promise<void> {
    if (!this.isMock) {
      await this.request("device.update", { id });
      return;
    }
    this.devices = this.devices.map((device) => (device.id === id && device.update ? { ...device, update: { ...device.update, state: "restarting" } } : device));
    this.emit({ kind: "rosterChanged" });
  }

  /** Unpairs this Device. The CLI asks the relay to drop its key, best effort, then forgets the
   * identity here; the `identity.changed` it sends brings back onboarding. The demo has no CLI, so
   * its Unpair opens onboarding instead (`confirmUnpair`). */
  async forgetIdentity(): Promise<void> {
    await this.request("identity.forget");
  }

  /** Deletes the account on the relay and on this Device; the other Devices forget it as the relay
   * drops them. Throws when the relay could not be told, and nothing is deleted then. */
  async deleteAccount(): Promise<void> {
    await this.request("identity.delete");
  }

  async restoreIdentity(phrase: string): Promise<void> {
    await this.request("identity.restore", { phrase });
    this.hasIdentity = true;
    this.isIdentityDevice = true;
    this.emit({ kind: "identityChanged" });
  }

  startPairing(): Promise<WirePairStart> {
    if (this.isMock) {
      return Promise.resolve({ nonce: "482913", pairing_string: "lorca://pair?relay=https%3A%2F%2Florca.app&id=idk_9f2c41ab&ek=ek_57ca0d3b&n=482913" });
    }
    return this.request<WirePairStart>("pair.start");
  }

  pairingStatus(nonce: string): Promise<WirePairStatus> {
    if (this.isMock) return Promise.resolve({ state: "waiting" });
    return this.request<WirePairStatus>("pair.status", { nonce });
  }

  cancelPairing(nonce: string): void {
    this.perform("pair.cancel", { nonce });
  }

  async acceptPairing(pairingString: string): Promise<void> {
    await this.request("pair.accept", { pairing_string: pairingString });
    this.hasIdentity = true;
    this.emit({ kind: "identityChanged" });
  }

  /** Stops waiting on the other Device: `acceptPairing` fails with "Pairing cancelled". */
  abortPairing(): void {
    this.perform("pair.abort");
  }

  /** Whether the account this computer just joined has a provider connected. The CLI answers
   * once its first pull from the relay has brought the account's credentials, or after half a
   * minute (`sync.account`). */
  async accountHasProvider(): Promise<boolean> {
    try {
      const synced = await this.request<WireSyncAccount>("sync.account");
      return toProviders(synced.providers).some((provider) => provider.isConnected);
    } catch {
      // A CLI that cannot answer (an older one, or one that restarted) leaves what the store has.
      return this.providers.some((provider) => provider.isConnected);
    }
  }

  /** The saved key of an API-key provider or a custom one, for its sheet. */
  providerAPIKey(kind: ProviderKind): Promise<{ api_key?: string | null; base_url?: string | null }> {
    if (this.isMock) return Promise.resolve({});
    return this.request("providers.api_key", { kind });
  }

  /** Connects an API-key provider. An empty `baseURL` means the provider's own API. */
  async connectAPIKey(kind: BuiltInProviderKind, apiKey: string, baseURL = ""): Promise<void> {
    const params: Record<string, unknown> = { api_key: apiKey };
    const trimmed = baseURL.trim();
    if (trimmed) params.base_url = trimmed;
    await this.request(`providers.connect_${kind === "opencode-go" ? "opencode_go" : kind}`, params);
  }

  /** Runs a subscription sign-in (`providers.connect_chatgpt`, `providers.connect_grok`): the CLI
   * opens the browser on this computer and the tokens go to the whole account. */
  async connectSignIn(kind: BuiltInProviderKind): Promise<void> {
    await this.request(`providers.connect_${kind}`);
  }

  /** Stops a sign-in that is waiting on the browser. */
  cancelSignIn(): void {
    this.perform("providers.auth.cancel");
  }

  credential(kind: ProviderKind): ProviderCredential | undefined {
    return this.providers.find((provider) => provider.kind === kind);
  }

  /** Disconnects a provider for the whole account, or deletes a custom one. */
  async disconnectProvider(kind: ProviderKind): Promise<void> {
    if (this.isMock && isCustomKind(kind)) {
      this.providers = this.providers.filter((provider) => provider.kind !== kind);
      this.emit({ kind: "rosterChanged" });
      this.emit({ kind: "chatsChanged" });
      return;
    }
    await this.request("providers.disconnect", { kind });
  }

  /** The chat models a custom provider's server lists, in its order (`providers.list_models`), for
   * the sheet to pick from; the base URL is read as the CLI saves it. Null when the server publishes
   * no list. Throws why the server could not be asked: a key it refused, no answer, or an answer
   * that is not an API's. */
  async listCustomModels(options: { name: string; api: CustomAPI; baseURL: string; apiKey: string }): Promise<CustomModel[] | null> {
    if (this.isMock) {
      await new Promise((resolve) => setTimeout(resolve, 300));
      const { listedModels } = await import("./mock");
      return listedModels(options.baseURL);
    }
    const reply = await this.request<WireModelList>("providers.list_models", {
      name: options.name,
      api: options.api,
      base_url: options.baseURL.trim(),
      api_key: options.apiKey,
    });
    return reply.listed ? reply.models.map(toCustomModel) : null;
  }

  /** Adds a custom provider, or saves the one `kind` names, once the CLI has heard from its server
   * (`providers.connect_custom`). `models` are the ids it offers, the default first; empty takes
   * every model the server lists. Answers the provider's kind. The provider reaches the account's
   * Devices, and this store, in the roster. */
  async saveCustomProvider(options: {
    kind?: CustomProviderKind;
    name: string;
    api: CustomAPI;
    baseURL: string;
    apiKey: string;
    models: string[];
  }): Promise<CustomProviderKind> {
    const name = options.name.trim();
    const baseURL = options.baseURL.trim();
    const models = options.models.map((id) => id.trim()).filter((id) => id !== "");
    if (this.isMock) {
      const kind = options.kind ?? (`custom:${name.toLowerCase().replaceAll(" ", "-")}` as const);
      const saved: ProviderCredential = { kind, isConnected: true, detail: baseURL, baseURL, name, api: options.api, models: models.map((id) => ({ id, levels: ["low", "medium", "high"] })) };
      this.providers = this.providers.some((provider) => provider.kind === kind)
        ? this.providers.map((provider) => (provider.kind === kind ? saved : provider))
        : [...this.providers, saved];
      this.emit({ kind: "rosterChanged" });
      this.emit({ kind: "chatsChanged" });
      return kind;
    }
    const params: Record<string, unknown> = { name, api: options.api, base_url: baseURL, api_key: options.apiKey.trim(), models };
    if (options.kind) params.kind = options.kind;
    const saved = await this.request<{ kind: string }>("providers.connect_custom", params);
    return isCustomKind(saved.kind) ? saved.kind : `custom:${saved.kind}`;
  }

  setRelayURL(url: string): void {
    this.relayURL = url === "" ? null : url;
    this.perform("config.set", { relay_url: url });
  }

  /** Replays the seeded conversations so the demo can be restarted from the Debug menu. */
  resetMockData(): void {
    if (!this.isMock) return;
    for (const chat of this.chats) this.replyEngine?.cancel(chat.id);
    void import("./mock").then((mock) => {
      this.devices = mock.devices();
      this.bots = mock.bots();
      this.chats = mock.chats();
      this.routines = mock.routines();
      this.autoReview = mock.autoReview();
      this.providers = mock.providers();
      this.models = mock.models();
      this.sortChats();
      this.emit({ kind: "snapshotReplaced" });
    });
  }

  /** What the offline state says while the CLI is not answering: where the launcher stands, or why
   * the CLI is not running. */
  get offlineStatus(): string {
    const status = this.cliState.launcher;
    switch (status.kind) {
      case "probing":
        return L("Looking for the CLI…");
      case "starting":
        return L("Starting the CLI…");
      case "running":
        return status.external ? L("Using the CLI already running on this computer") : L("CLI started by the app");
      case "failed": {
        const failure = status.failure;
        switch (failure?.kind) {
          case "missing_binary":
            return L("The lorca CLI is not bundled with this build and is not on PATH.");
          case "launch":
            return L("Could not start %@: %@", failure.binary ?? "lorca", failure.reason ?? "");
          case "exited":
            return L("The CLI exited with code %d. See %@.", failure.code ?? 0, failure.log ?? "");
          case "startup_closed":
            return L("The CLI closed its startup channel before becoming ready. See %@.", failure.log ?? "");
          default:
            return failure?.reason ?? L("Waiting to start the CLI");
        }
      }
      default:
        return L("Waiting to start the CLI");
    }
  }
}

export const store = new AppStore();
