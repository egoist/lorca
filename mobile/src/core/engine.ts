// The phone's side of the core: starts it once with the app's folder and this phone's facts,
// turns its events into store changes, and offers the screens the same verbs the macOS app has.
// Every call is one request over the JSON API the desktop app speaks to the CLI; the Rust
// core does the keys, the relay, jobs, rooms, and questions to Runners.

import * as WebBrowser from "expo-web-browser";
import { AppState, Platform, type AppStateStatus } from "react-native";
import * as core from "../../modules/lorca-core";
import { t } from "../i18n";
import { exactAnswer, ExactNumber, ExactObject, stringifyExact } from "./exactJson";
import { reviewEditParams } from "./reviewEdit";
import { hostFacts } from "./host";
import { providerConnectMethod, withReviewModel, type Attachment, type AutoReview, type Bot, type BrowserProfile, type Chat, type ChatMeta, type ChatSearchResults, type ChatUsage, type CustomAPI, type CustomModel, type DurableTask, type Message, type ReviewItem, type PluginDetail, type PluginStatus, type ProviderKind, type ProviderStatus } from "./model";
import { coreHome, loadPrefs, pathOf, wipePrefs } from "./prefs";
import { clearPushes, installPushHandlers, registerForPushes } from "./push";
import {
  acceptDurableTask,
  acceptReview,
  applyRoster,
  botById,
  chatById,
  endActivity,
  markFile,
  markFileError,
  markRead,
  patchRoutine,
  removeChat,
  removeMessage,
  removeRoutine,
  replaceSnapshot,
  resetStore,
  setChatUsage,
  setRetry,
  setRunning,
  setStatus,
  setThinking,
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
  /// The sign-in page up in the in-app browser: whose it is (a plugin sign-in's id, or
  /// "provider"), and whether this side is closing it.
  private authPage: { attempt: string; closing: boolean } | null = null;
  private readChatId: string | null = null;
  /// Snapshots on their way, and the events that arrived meanwhile (see `bootstrap`).
  private bootstraps = 0;
  private held: core.Frame[] = [];

  // MARK: - Lifecycle

  /// Starts the core and takes its first snapshot. Safe to call again.
  async start() {
    if (this.started) return;
    this.started = true;
    const active = AppState.currentState === "active";
    useStore.setState({ dictation_lang: loadPrefs().dictation_lang, appActive: active, activeSince: active ? Date.now() : 0 });
    core.onEvent((frame) => this.receive(frame));
    // The core starts off the JS thread and emits as soon as it runs: what it says before the
    // first snapshot waits for it, as during any snapshot.
    this.bootstraps += 1;
    try {
      await core.start(coreHome(), hostFacts());
      AppState.addEventListener("change", (status) => this.onAppState(status));
      // Read after every store update, including the roster's unread count that follows a
      // message event, a backlog snapshot, and returning to a chat already mounted on screen.
      useStore.subscribe(() => this.readVisibleChat());
      await this.bootstrap();
    } finally {
      this.bootstraps -= 1;
      if (!this.bootstraps) this.applyHeld();
    }
    installPushHandlers();
    if (useStore.getState().paired) void registerForPushes();
  }

  /// Takes the core's snapshot of the account. The core keeps emitting while it builds one, so
  /// an event can arrive ahead of a snapshot older than it: the relay connecting during launch,
  /// a reply. Those events wait and are applied after the snapshot, as the macOS app does; each
  /// says where a thing stands, so one the snapshot already has changes nothing.
  private async bootstrap() {
    this.bootstraps += 1;
    try {
      replaceSnapshot(await core.request<Snapshot>("bootstrap"));
    } finally {
      this.bootstraps -= 1;
      if (!this.bootstraps) this.applyHeld();
    }
  }

  private receive(frame: core.Frame) {
    if (this.bootstraps) this.held.push(frame);
    else this.apply(frame.event, frame.data);
  }

  /// Each held event on its own, as on arrival: one that fails keeps back none of the rest.
  private applyHeld() {
    for (const frame of this.held.splice(0)) {
      try {
        this.apply(frame.event, frame.data);
      } catch (error) {
        console.warn(`applying ${frame.event}`, error instanceof Error ? error.message : error);
      }
    }
  }

  private onAppState(status: AppStateStatus) {
    if (status !== "active") return useStore.setState({ appActive: false });
    useStore.setState({ appActive: true, activeSince: Date.now() });
    // Back in the foreground: the sync socket may have died with the suspension.
    core.wake();
    // The token can change, and permission may have been given in Settings meanwhile.
    if (useStore.getState().paired) void registerForPushes();
  }

  private readVisibleChat() {
    const { appActive, paired, openChatId } = useStore.getState();
    const chatId = appActive && paired ? openChatId : null;
    if (chatId !== this.readChatId) {
      this.readChatId = chatId;
      if (chatId) void clearPushes(chatId);
    }
    if (chatId) markRead(chatId);
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
      case "tasks.changed":
        acceptDurableTask((data as { task: DurableTask }).task);
        break;
      case "reviews.changed":
        acceptReview((data as { item: ReviewItem }).item);
        break;
      case "roster.changed": {
        const { removed } = applyRoster(data);
        for (const chatId of removed) removeChat(chatId);
        break;
      }
      case "message.added":
      case "message.updated": {
        const message = data.message as Message;
        upsertMessage(message, event === "message.added");
        break;
      }
      case "message.removed":
        removeMessage(data.chat_id, data.message_id);
        break;
      case "chat.removed":
        removeChat(data.chat_id);
        break;
      case "job.started":
        setRunning(data.job_id, { chatId: data.chat_id, botId: data.bot_id ?? "", routineId: data.routine_id ?? undefined });
        break;
      case "job.finished":
        setRunning(data.job_id, null);
        endActivity(data.chat_id, data.bot_id ?? "");
        if (!Object.values(useStore.getState().running).some((r) => r.chatId === data.chat_id)) this.noteSilence(data.chat_id);
        break;
      case "job.thinking":
        setThinking(data.chat_id, data.bot_id);
        break;
      case "job.retry":
        setRetry(data.chat_id, { attempt: data.attempt, max_attempts: data.max_attempts, delay_ms: data.delay_ms });
        break;
      case "chat.usage":
        setChatUsage(data.chat_id, data.usage as ChatUsage);
        break;
      case "relay.status":
        useStore.setState((s) => ({ relayConnected: !!data.connected, relayUpdateRequired: !!data.update_required, relayError: data.error ?? null, relayUrl: data.url ?? s.relayUrl }));
        break;
      case "provider.auth":
        void this.openAuthPage(data.url, "provider", () => core.request("providers.auth.cancel"));
        break;
      case "plugin.auth":
        void this.openAuthPage(data.url, data.sign_in, () => core.request("plugins.auth.cancel", { sign_in: data.sign_in }));
        break;
      case "plugin.auth.done":
        void this.closeAuthPage(data.sign_in);
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
      setStatus(chatId, t("{name} stopped without replying", { name: name ?? chatTitle(chat) }));
    }
  }

  // MARK: - Chats

  /// Sends the user's message with its files and the bots picked by `@`; the core starts the
  /// turns it calls for.
  /// `replyTo` is the message the user answers, which the bot reads quoted.
  async sendMessage(chatId: string, text: string, files: PickedFile[] = [], mentions: string[] = [], replyTo?: string): Promise<Message> {
    const attachments = files.map((file) => ({ path: pathOf(file.uri), name: file.name, mime: file.mime, width: file.width, height: file.height }));
    const { message } = await core.request<{ message: Message }>("chats.send", { chat_id: chatId, text, attachments, mentions, ...(replyTo ? { reply_to: replyTo } : {}) });
    upsertMessage(message);
    setStatus(chatId, null);
    return message;
  }

  /// Has the bot's turn read a message it holds for its next step now: a command it waits on goes
  /// to the background, and a reply in progress stops where it got to. The bot's Runner does it.
  async sendNow(chatId: string, messageId: string): Promise<void> {
    await core.request("chats.send_now", { chat_id: chatId, message_id: messageId });
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

  async searchChats(query: string): Promise<ChatSearchResults> {
    const value = query.trim();
    if (!value) return { chats: [], messages: [] };
    return core.request<ChatSearchResults>("chats.search", { query: value, limit: 24 });
  }

  /// The attachment's bytes, from this phone's copy or the relay, as a file URI in the store. A
  /// fetch that failed is not asked again until `retryFile`.
  async fetchFile(attachment: Attachment): Promise<void> {
    const { files, fileErrors } = useStore.getState();
    if (files[attachment.id] || fileErrors[attachment.id] || this.fetchingFiles.has(attachment.id)) return;
    this.fetchingFiles.add(attachment.id);
    try {
      const { path } = await core.request<{ path: string }>("files.path", { attachment });
      markFile(attachment.id, `file://${path}`);
    } catch (error) {
      markFileError(attachment.id, error instanceof Error ? error.message : String(error));
    } finally {
      this.fetchingFiles.delete(attachment.id);
    }
  }

  retryFile(attachment: Attachment) {
    markFileError(attachment.id, null);
    void this.fetchFile(attachment);
  }

  /// The path of the attachment as a file named for what it is, for Quick Look or another app:
  /// the bytes under their attachment id carry no extension, so the core keeps a private named
  /// copy.
  async namedFile(attachment: Attachment): Promise<string> {
    const { path } = await core.request<{ path: string }>("files.path", { attachment, named: true });
    return path;
  }

  /// A durable task method (`tasks.create`, `tasks.update`, `tasks.run`, `tasks.get`); the core
  /// sends a write to the task's authority Runner. The task it answers with is kept unless a
  /// newer revision arrived first.
  async taskRequest(method: string, params: Record<string, unknown>): Promise<DurableTask> {
    const task = await core.request<DurableTask>(method, params);
    acceptDurableTask(task);
    return task;
  }

  /// A review item as its Runner keeps it, with its numbers spelled as they were: the review
  /// screen shows and edits a call's arguments from it, since `JSON.parse` rounds an id past 2^53.
  async exactReview(id: string): Promise<ExactObject> {
    const item = exactAnswer(await core.requestText("reviews.get", JSON.stringify({ id })));
    if (!(item instanceof ExactObject)) throw new Error("No such review");
    return item;
  }

  /// Approves the version the user saw. `edited`, the field's text when the user changed it, is
  /// saved first as the next version, and that version is approved: what runs is what the field
  /// showed. A command's other arguments and a call's server and tool stay as they were.
  async approveReview(item: ReviewItem, edited?: string): Promise<ReviewItem> {
    let shown = item;
    if (edited !== undefined) {
      // The edit names the version the user saw; one changed elsewhere is refused.
      const exact = (await this.exactReview(item.id)).with("version", new ExactNumber(String(item.version)));
      const params = reviewEditParams(exact, edited, t("The arguments need to be a JSON object."));
      shown = JSON.parse(stringifyExact(exactAnswer(await core.requestText("reviews.edit", params)))) as ReviewItem;
      acceptReview(shown);
    }
    return this.reviewRequest("reviews.approve", { id: shown.id, expected_version: shown.version });
  }

  rejectReview(item: ReviewItem): Promise<ReviewItem> {
    return this.reviewRequest("reviews.reject", { id: item.id, expected_version: item.version });
  }

  /// A decision goes to the item's Runner through the core; the item it answers with is kept
  /// unless a newer revision arrived first.
  private async reviewRequest(method: string, params: { id: string; expected_version: number }): Promise<ReviewItem> {
    const item = await core.request<ReviewItem>(method, params);
    acceptReview(item);
    return item;
  }

  /// Every output version this phone has synced for the chat, for its details.
  async listOutputs(chatId: string): Promise<void> {
    try {
      const { outputs } = await core.request<{ outputs: Message[] }>("outputs.list", { chat_id: chatId });
      useStore.setState((s) => ({ outputs: { ...s.outputs, [chatId]: outputs } }));
    } catch (error) {
      console.warn("listing outputs", error instanceof Error ? error.message : error);
    }
  }

  async createBot(input: { name: string; description: string; symbol_name: string; accent: string; runner_id: string; provider: string; model?: string; thinking?: string }): Promise<{ bot: Bot; chatId: string }> {
    const { bot, chat_id } = await core.request<{ bot: Bot; chat_id: string }>("bots.create", input);
    return { bot, chatId: chat_id };
  }

  async updateBot(id: string, update: Partial<Pick<Bot, "name" | "description">>): Promise<Bot> {
    const { bot } = await core.request<{ bot: Bot }>("bots.update", { id, ...update });
    useStore.setState((s) => ({ bots: s.bots.map((current) => (current.id === id ? bot : current)) }));
    return bot;
  }

  /// Provider, model, and thinking level a bot runs with. Missing model/thinking means the
  /// provider default; empty strings on the wire clear a value the bot previously stored.
  setBotRuntime(id: string, provider: string, model?: string, thinking?: string) {
    const current = useStore.getState().bots.find((bot) => bot.id === id);
    if (!current || (current.provider === provider && current.model === model && current.thinking === thinking)) return;
    useStore.setState((s) => ({
      bots: s.bots.map((bot) => (bot.id === id ? { ...bot, provider, model, thinking } : bot)),
    }));
    void core.request("bots.update", { id, provider, model: model ?? "", thinking: thinking ?? "" }).catch((error) => {
      console.warn("updating bot runtime", error instanceof Error ? error.message : error);
    });
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

  renameGroup(chatId: string, title: string) {
    if (chatById(chatId)?.kind !== "group") return;
    this.patchChat(chatId, (meta) => ({ ...meta, title: title.trim() || null }));
    void core.request("chats.rename", { chat_id: chatId, title });
  }

  /// What a group is for; every member reads it in its system prompt.
  setGroupDescription(chatId: string, description: string) {
    const trimmed = description.trim();
    const chat = chatById(chatId);
    if (chat?.kind !== "group" || (chat.description ?? "") === trimmed) return;
    this.patchChat(chatId, (meta) => ({ ...meta, description: trimmed || null }));
    void core.request("chats.set_description", { chat_id: chatId, description: trimmed });
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

  /// Picks the provider whose review model Auto-review runs, or none for the bot's own. Only
  /// `provider` goes, so the switch, the rules, and the review models stay.
  setReviewProvider(provider: string | undefined) {
    const held = useStore.getState().auto_review.provider;
    useStore.setState((s) => ({ auto_review: { ...s.auto_review, provider } }));
    core.request("auto_review.set", { provider: provider ?? null }).catch(() => {
      // The provider disconnected meanwhile: the core kept what it had.
      useStore.setState((s) => ({ auto_review: { ...s.auto_review, provider: held } }));
    });
  }

  /// Picks the model Auto-review runs on a provider, or puts its default back (`undefined`). The
  /// patch names this provider alone, so the others' review models stay.
  setReviewModel(kind: string, model: string | undefined) {
    const held = useStore.getState().auto_review.models?.[kind];
    useStore.setState((s) => ({ auto_review: withReviewModel(s.auto_review, kind, model) }));
    core.request("auto_review.set", { models: { [kind]: model ?? null } }).catch(() => {
      useStore.setState((s) => ({ auto_review: withReviewModel(s.auto_review, kind, held) }));
    });
  }

  // MARK: - Providers

  /// Checks and saves an API key, or runs a subscription sign-in in the phone's in-app
  /// browser. The core writes the encrypted account credential and publishes it to every
  /// paired Device.
  async connectProvider(kind: ProviderKind, input: { apiKey?: string; baseURL?: string } = {}): Promise<void> {
    const params = input.apiKey === undefined ? {} : { api_key: input.apiKey, base_url: input.baseURL?.trim() || undefined };
    try {
      const { providers } = await core.request<{ providers: ProviderStatus[] }>(providerConnectMethod(kind), params);
      useStore.setState({ providers });
    } finally {
      if (kind === "chatgpt" || kind === "grok") void this.closeAuthPage("provider");
    }
  }

  /// Adds a custom provider, or saves the one `kind` names, once the core has reached its server
  /// with the key. With no model ids the core takes every chat model the server lists. The
  /// provider joins the account's encrypted credentials, shared with every paired Device.
  /// Answers its kind; rejects with what to fix.
  async saveCustomProvider(input: { kind?: string; name: string; api: CustomAPI; baseURL: string; apiKey: string; models: string[] }): Promise<string> {
    const params = { ...(input.kind ? { kind: input.kind } : {}), name: input.name, api: input.api, base_url: input.baseURL, api_key: input.apiKey, models: input.models };
    const { kind, providers } = await core.request<{ kind: string; providers: ProviderStatus[] }>("providers.connect_custom", params);
    useStore.setState({ providers });
    return kind;
  }

  /// The chat models a custom provider's server lists, in its order, with what it says of them;
  /// `listed` is false when the server publishes no list. Rejects with what is wrong: a key it
  /// refuses, a server out of reach, an answer that is not an API's. `name` goes in the messages.
  async listCustomModels(input: { name?: string; api: CustomAPI; baseURL: string; apiKey: string }): Promise<{ listed: boolean; models: CustomModel[] }> {
    const params = { ...(input.name ? { name: input.name } : {}), api: input.api, base_url: input.baseURL, api_key: input.apiKey };
    const { listed, models } = await core.request<{ listed: boolean; models?: CustomModel[] }>("providers.list_models", params);
    return { listed: !!listed, models: models ?? [] };
  }

  /// The saved key and base URL of an API-key or custom provider, for its form. Statuses carry
  /// only a masked key.
  async providerAPIKey(kind: string): Promise<{ api_key?: string | null; base_url?: string | null }> {
    return core.request("providers.api_key", { kind });
  }

  /// Disconnects a built-in provider, or deletes a custom one, for the whole account.
  async disconnectProvider(kind: string): Promise<void> {
    const { providers } = await core.request<{ providers: ProviderStatus[] }>("providers.disconnect", { kind });
    useStore.setState({ providers });
  }

  /// Opens a sign-in page, a provider's or a plugin's for its Runner, in the in-app browser
  /// while the core waits on its loopback callback; a page up for another sign-in closes first.
  /// Closing the page before the sign-in came back calls `cancel`.
  private async openAuthPage(url: string, attempt: string, cancel: () => Promise<unknown>) {
    if (this.authPage) await this.closeAuthPage(this.authPage.attempt);
    // iOS keeps the core alive behind SFSafariViewController. Android's auth-session
    // polyfill also watches AppState, so closing the custom tab can cancel the Rust wait.
    const page = { attempt, closing: false };
    this.authPage = page;
    try {
      const result = await (Platform.OS === "android" ? WebBrowser.openAuthSessionAsync(url) : WebBrowser.openBrowserAsync(url));
      if (!page.closing && (result.type === "cancel" || result.type === "dismiss")) void cancel().catch(() => {});
    } catch (error) {
      console.warn("opening a sign-in page", error instanceof Error ? error.message : error);
      void cancel().catch(() => {});
    } finally {
      if (this.authPage === page) this.authPage = null;
    }
  }

  /// Closes the page of sign-in `attempt` when it is the one up: the sign-in came back, or ended.
  private async closeAuthPage(attempt: string) {
    const page = this.authPage;
    if (!page || page.attempt !== attempt) return;
    page.closing = true;
    // Chrome Custom Tabs have no programmatic dismiss. Its success page stays up until the
    // user closes it; the page's promise then ends without cancelling the sign-in.
    if (Platform.OS === "android") return;
    try {
      await WebBrowser.dismissBrowser();
    } catch {
      // The browser may already be gone on this platform.
      page.closing = false;
    }
  }

  // MARK: - Plugins

  /// Answers a permission card; the core's message event confirms the decision. Sign in on a
  /// sign-in card for a bot on another Runner opens the sign-in page here. Rejects with why the
  /// answer did not reach the Runner, such as it being offline, and the card asks again.
  async answerPermission(chatId: string, messageId: string, decision: "allow" | "always" | "deny") {
    const decided: "always" | "denied" | "allowed" = decision === "always" ? "always" : decision === "deny" ? "denied" : "allowed";
    // A permission card shows the answer; a command's card moves on to running, or ends.
    const answered = (m: Message): Message => {
      if (m.body.kind === "permission") return { ...m, body: { ...m.body, decision: decided } };
      if (m.body.kind === "tool" && m.body.run?.state === "asking") {
        const run = { ...m.body.run, decision: decided, state: decided === "denied" ? ("denied" as const) : ("running" as const), rule: decided === "always" ? m.body.run.rule : undefined };
        return { ...m, body: { ...m.body, run } };
      }
      return m;
    };
    const asked = chatById(chatId)?.messages.find((m) => m.id === messageId);
    const shown = asked && answered(asked);
    // Puts back what the card showed, unless the core changed it meanwhile.
    const replace = (from: Message | undefined, to: Message | undefined) => {
      if (!from || !to) return;
      useStore.setState((s) => ({
        chats: s.chats.map((c) => (c.id === chatId ? { ...c, messages: c.messages.map((m) => (m === from ? to : m)) } : c)),
      }));
    };
    replace(asked, shown);
    try {
      await core.request("chats.permission", { chat_id: chatId, message_id: messageId, decision });
    } catch (error) {
      replace(shown, asked);
      throw error;
    }
  }

  /// A plugin on its Runner, with how each of its servers signs in; sealed to another Runner.
  /// The bot's browser profiles, oldest first, from its Runner through the relay.
  async browserProfiles(botId: string): Promise<BrowserProfile[]> {
    const { sessions } = await core.request<{ sessions: BrowserProfile[] }>("browser.sessions", { bot_id: botId });
    return sessions;
  }

  /// `browser.create` (`name`), `browser.takeover`, `browser.resume` (`revision`), `browser.stop`,
  /// `browser.delete`, or `browser.screenshot` (`chat_id`) for one of the bot's profiles
  /// (`session_id`), on its Runner. Windows open only there, so the phone never sends `browser.open`.
  async browserAction(method: string, botId: string, params: Record<string, string | number>): Promise<void> {
    await core.request(method, { ...params, bot_id: botId });
  }

  pluginDetail(runnerId: string, pluginId: string): Promise<PluginDetail> {
    return core.request<PluginDetail>("plugins.detail", { runner_id: runnerId, plugin_id: pluginId });
  }

  /// Signs a plugin in on its Runner. The page opens here (`plugin.auth`) and the Runner keeps the
  /// tokens; resolves once the sign-in has finished or failed.
  async connectPlugin(runnerId: string, pluginId: string) {
    await core.request("plugins.connect", { runner_id: runnerId, plugin_id: pluginId });
  }

  /// Forgets a plugin's sign-in on its Runner; its next use asks again.
  async signOutPlugin(runnerId: string, pluginId: string) {
    await core.request("plugins.sign_out", { runner_id: runnerId, plugin_id: pluginId });
  }

  /// Renames a named account (Gmail · Work). Its id, sign-in, and tools stay; the answer stands in
  /// the Runner's list until its next machine blob lists the new name.
  async renamePluginAccount(runnerId: string, pluginId: string, accountName: string) {
    const { status } = await core.request<{ status: PluginStatus }>("plugins.rename", { runner_id: runnerId, plugin_id: pluginId, account_name: accountName });
    useStore.setState((s) => ({
      devices: s.devices.map((d) => (d.id === runnerId ? { ...d, plugins: (d.plugins ?? []).map((plugin) => (plugin.id === status.id ? status : plugin)) } : d)),
    }));
    return status;
  }

  // MARK: - Commands

  /// Types the user's answer into a command running in its terminal, then Return. It goes
  /// through the core, sealed to the bot's Runner, and nothing keeps it. Rejects with why it
  /// could not, such as the Runner being offline.
  async answerCommand(chatId: string, messageId: string, text: string) {
    await core.request("bash.stdin", { chat_id: chatId, message_id: messageId, text });
  }

  /// Stops a running command; its card says so once the Runner has.
  async stopCommand(chatId: string, messageId: string) {
    await core.request("bash.stop", { chat_id: chatId, message_id: messageId });
  }

  /// Sends a command the bot is waiting on to the background: the bot's call returns and the
  /// command runs on, out of the way of Stop in the chat.
  async sendCommandToBackground(chatId: string, messageId: string) {
    await core.request("bash.background", { chat_id: chatId, message_id: messageId });
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
  /// marks that) and waits for the other Device to accept; `signal` aborts the wait in the core too,
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
    await this.bootstrap();
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

  /// Asks a self-updating Runner, through the relay, to install the latest release, which it
  /// restarts into once no bot is at work there. Rejects with the Runner's words, or when it
  /// does not answer. "Installing" shows at once; the Runner's `machine` blob confirms it.
  async updateDevice(id: string) {
    const answer = await core.request<{ version: string; latest?: string | null; installing?: boolean }>("device.update", { id });
    if (!answer.installing || !answer.latest) return;
    const latest = answer.latest;
    useStore.setState((s) => ({ devices: s.devices.map((d) => (d.id === id && d.update ? { ...d, update: { ...d.update, latest, state: "installing", error: undefined } } : d)) }));
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
  if (chat.kind === "group" && chat.title?.trim()) return chat.title.trim();
  const names = chat.bot_ids.map((id) => botById(id)?.name).filter((n): n is string => !!n);
  if (chat.kind === "dm") return names[0] ?? t("Chat");
  return names.length ? names.join(", ") : t("Group");
}
