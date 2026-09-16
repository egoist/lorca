// Behavior: relay sync (auth, outbox, presence, long-poll, applying blobs), sending a message
// and the turns it calls for (a DM turn, or a group room exchange run from this Device), and
// roster edits. The phone is a Device, never a Runner: every turn is a job sealed to the bot's
// Runner, and the Runner reports how the turn ended with a job_result sealed back to us.

import { AppState, type AppStateStatus } from "react-native";
import { b64, hex, nowSecs, nowUnix, randomBytes, unb64, utf8, uuid } from "./bytes";
import { decrypt, decryptJson, encrypt, encryptJson, sealJson, unsealJson } from "./crypto";
import { Machine, type MachineFile } from "./keys";
import {
  isComplete,
  isRunner,
  MAX_ATTACHMENT_BYTES,
  MAX_ATTACHMENTS,
  MAX_GROUP_BOTS,
  type Attachment,
  type Bot,
  type Chat,
  type ChatBlob,
  type ChatMeta,
  type Device,
  type Job,
  type JobResult,
  type MachineBlob,
  type Message,
  type RosterBlob,
} from "./model";
import { hostFacts } from "./host";
import { acceptPairing } from "./pairing";
import { RelayClient, RelayError, type BlobIn } from "./relay";
import { File } from "expo-file-system";
import { deleteOutboxCiphertext, fileFor, flushState, hasFile, readOutboxCiphertext, saveMachineFile, wipeState, writeFile, writeOutboxCiphertext } from "./storage";
import {
  applyRoster,
  botById,
  bumpUnread,
  chatById,
  dequeue,
  deviceById,
  deviceIsOnline,
  enqueue,
  markFile,
  mutate,
  rememberApplied,
  removeMessage,
  resetStore,
  setDeviceSeen,
  setLastSeq,
  setMachinePubkey,
  setRunning,
  setStatus,
  updateChatMeta,
  upsertDevice,
  upsertMessage,
  useStore,
  wasApplied,
} from "./store";
import { sha256 } from "@noble/hashes/sha2.js";

const POLL_WAIT_SECS = 25;
/// What the poll takes. `file` blobs are fetched by id when a transcript shows them, and jobs
/// never come to a phone.
const POLL_KINDS = "roster,chat,machine,job_result";
const REMOTE_TURN_TIMEOUT_MS = 300_000;
const MAX_ROOM_ROUNDS = 4;

type Outcome = "sent" | "pass" | "error";

/// A file the composer picked, before it is read and stored.
export interface PickedFile {
  uri: string;
  name: string;
  mime: string;
  size?: number;
  width?: number;
  height?: number;
}

class Engine {
  readonly relay = new RelayClient();
  private machine: Machine | null = null;
  private dek: Uint8Array | null = null;
  private relayUrl: string | null = null;
  private started = false;
  private paused = false;
  private wake: (() => void) | null = null;
  private poll: AbortController | null = null;
  private pending = new Map<string, (outcome: Outcome) => void>();
  private fetchingFiles = new Set<string>();
  private roomCancels = new Map<string, AbortController>();
  /// The chat lock: exchanges in one group run one after another, never overlapping.
  private roomQueue = new Map<string, Promise<void>>();

  // MARK: - Lifecycle

  /// Loads keys from the machine file and starts syncing. Safe to call again after pairing.
  bind(file: MachineFile | null) {
    if (!file) {
      this.machine = null;
      this.dek = null;
      this.relayUrl = null;
      setMachinePubkey(null);
      this.relay.forgetToken();
      return;
    }
    this.machine = Machine.fromSecretB64(file.machine_secret);
    this.dek = unb64(file.account_dek);
    this.relayUrl = file.relay_url;
    setMachinePubkey(this.machine.pubkey);
    this.relay.forgetToken();
    this.start();
    this.notify();
  }

  get deviceId(): string | null {
    return this.machine?.pubkey ?? null;
  }

  start() {
    if (this.started) return;
    this.started = true;
    AppState.addEventListener("change", (status) => this.onAppState(status));
    void this.run();
  }

  private onAppState(status: AppStateStatus) {
    if (status === "active") {
      if (this.paused) {
        this.paused = false;
        this.relay.forgetToken();
        this.notify();
      }
    } else if (status === "background") {
      this.paused = true;
      this.poll?.abort();
      flushState();
    }
  }

  /// Wakes the sync loop: something joined the outbox or the app came back.
  notify() {
    this.poll?.abort();
    this.wake?.();
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => {
      const timer = setTimeout(done, ms);
      const self = this;
      function done() {
        clearTimeout(timer);
        if (self.wake === done) self.wake = null;
        resolve();
      }
      this.wake = done;
    });
  }

  private async run() {
    let failures = 0;
    for (;;) {
      if (this.paused) {
        await this.sleep(60_000);
        continue;
      }
      try {
        await this.cycle();
        failures = 0;
      } catch (error) {
        if (useStore.getState().relayConnected) useStore.setState({ relayConnected: false });
        if (error instanceof RelayError && error.isUnauthorized) this.relay.forgetToken();
        if (!(error instanceof RelayError && error.message === "cancelled")) {
          failures = Math.min(failures + 1, 5);
          const delay = Math.min(2 ** failures, 60) * 1000;
          console.warn("relay", error instanceof Error ? error.message : error, `retry in ${delay / 1000}s`);
          await this.sleep(delay);
        }
      }
    }
  }

  private async cycle() {
    const { machine, dek, relayUrl: url } = this;
    if (!machine || !dek || !url) {
      await this.sleep(2000);
      return;
    }
    const token = await this.relay.tokenFor(url, machine);
    if (!useStore.getState().relayConnected) useStore.setState({ relayConnected: true });

    this.pushMachineBlobIfChanged();
    await this.drainOutbox(url, token);
    await this.refreshPresence(url, token);

    const since = useStore.getState().last_seq;
    this.poll = new AbortController();
    let result: { blobs: BlobIn[]; seq: number };
    try {
      result = await this.relay.listBlobs(url, token, since, POLL_KINDS, POLL_WAIT_SECS, this.poll.signal);
    } finally {
      this.poll = null;
    }
    for (const blob of result.blobs) {
      this.applyBlob(blob);
      setLastSeq(blob.seq);
    }
  }

  private async drainOutbox(url: string, token: string) {
    for (;;) {
      const item = useStore.getState().outbox[0];
      if (!item) return;
      try {
        const ciphertext = item.ciphertext_file ? readOutboxCiphertext(item.ciphertext_file) : item.ciphertext;
        await this.relay.putBlob(url, token, item.id, item.kind, item.recipient, ciphertext);
      } catch (error) {
        if ((error instanceof RelayError && error.isClientError && !error.isUnauthorized) || !(error instanceof RelayError)) {
          console.warn("relay rejected blob; dropping", item.kind, error instanceof Error ? error.message : error);
        } else {
          throw error;
        }
      }
      if (item.ciphertext_file) deleteOutboxCiphertext(item.ciphertext_file);
      dequeue(item.id);
    }
  }

  private async refreshPresence(url: string, token: string) {
    const { machines } = await this.relay.machines(url, token);
    const seen: Record<string, number> = {};
    const boxKeys: Record<string, string> = {};
    for (const m of machines) {
      seen[m.machine_pubkey] = m.last_seen;
      boxKeys[m.machine_pubkey] = m.box_pubkey;
    }
    setDeviceSeen(seen, boxKeys);
  }

  // MARK: - Blobs out

  private pushBlob(kind: string, recipient: string | null, ciphertext: Uint8Array): string {
    const id = uuid();
    enqueue({ id, kind, recipient, ciphertext: b64(ciphertext) });
    this.notify();
    return id;
  }

  private pushChatOp(op: ChatBlob) {
    if (!this.dek) return;
    this.pushBlob("chat", null, encryptJson(this.dek, "chat", op));
  }

  /// An attachment's bytes as a `file` blob under its own id, so any Device can fetch it by
  /// that id. The ciphertext waits on disk, not inside state.json.
  private pushFileBlob(id: string, ciphertext: Uint8Array) {
    const path = writeOutboxCiphertext(id, b64(ciphertext));
    enqueue({ id, kind: "file", recipient: null, ciphertext: "", ciphertext_file: path });
    this.notify();
  }

  // MARK: - Attachments

  /// Makes sure the attachment's bytes are on this phone, fetching them from the relay when
  /// another Device sent them; the store's `files` map gets the URI when they are.
  async fetchFile(attachment: Attachment): Promise<void> {
    const { id } = attachment;
    if (useStore.getState().files[id]) return;
    if (hasFile(id)) {
      markFile(id, fileFor(id).uri);
      return;
    }
    const { machine, dek, relayUrl: url } = this;
    if (!machine || !dek || !url || this.fetchingFiles.has(id)) return;
    this.fetchingFiles.add(id);
    try {
      const token = await this.relay.tokenFor(url, machine);
      const blob = await this.relay.getBlob(url, token, id);
      if (!blob) return;
      writeFile(id, decrypt(dek, "file", unb64(blob.ciphertext)));
      markFile(id, fileFor(id).uri);
    } catch (error) {
      console.warn("fetching", attachment.name, error instanceof Error ? error.message : error);
    } finally {
      this.fetchingFiles.delete(id);
    }
  }

  /// Reads a picked file into the store and returns its record; the bytes are queued for the
  /// relay ahead of the message that names them.
  private async storeAttachment(picked: PickedFile): Promise<Attachment> {
    if (!this.dek) throw new Error("Not paired");
    const bytes = await new File(picked.uri).bytes();
    if (bytes.length > MAX_ATTACHMENT_BYTES) throw new Error(`${picked.name} is larger than ${MAX_ATTACHMENT_BYTES / 1024 / 1024} MB`);
    const id = `att-${hex(randomBytes(6))}`;
    writeFile(id, bytes);
    markFile(id, fileFor(id).uri);
    this.pushFileBlob(id, encrypt(this.dek, "file", bytes));
    const attachment: Attachment = { id, name: picked.name, mime: picked.mime, size: bytes.length };
    if (picked.width && picked.height) {
      attachment.width = picked.width;
      attachment.height = picked.height;
    }
    return attachment;
  }

  private pushRoster() {
    if (!this.dek) return;
    const s = useStore.getState();
    const roster: RosterBlob = {
      bots: s.bots,
      chats: s.chats.map(({ messages: _m, unread_count: _u, ...meta }) => meta),
      updated_at: nowSecs(),
    };
    this.pushBlob("roster", null, encryptJson(this.dek, "roster", roster));
  }

  localDevice(): Device | null {
    const file = useStore.getState().machineFile;
    if (!file || !this.machine) return null;
    return {
      id: this.machine.pubkey,
      name: file.name,
      model: file.model,
      os: file.os,
      os_version: file.os_version,
      box_pubkey: this.machine.boxPubkey,
      providers_connected: [],
      updated_at: nowUnix(),
    };
  }

  private pushMachineBlobIfChanged() {
    const device = this.localDevice();
    if (!device || !this.dek) return;
    const fingerprint = `${device.id}|${device.name}|${device.model}|${device.os}|${device.os_version}|[]`;
    const hash = b64(sha256(utf8(fingerprint)));
    upsertDevice(device);
    if (useStore.getState().machine_blob_hash === hash) return;
    mutate(() => ({ machine_blob_hash: hash }));
    const blob: MachineBlob = { device };
    this.pushBlob("machine", null, encryptJson(this.dek, "machine", blob));
  }

  // MARK: - Blobs in

  private applyBlob(blob: BlobIn) {
    if (wasApplied(blob.id)) return;
    rememberApplied(blob.id);
    const { dek, machine } = this;
    if (!dek || !machine) return;
    let ciphertext: Uint8Array;
    try {
      ciphertext = unb64(blob.ciphertext);
    } catch {
      return;
    }
    try {
      switch (blob.kind) {
        case "roster":
          this.onRoster(decryptJson<RosterBlob>(dek, "roster", ciphertext));
          break;
        case "chat":
          this.onChatOp(decryptJson<ChatBlob>(dek, "chat", ciphertext));
          break;
        case "machine": {
          const { device } = decryptJson<MachineBlob>(dek, "machine", ciphertext);
          if (device.id !== machine.pubkey) upsertDevice(device);
          break;
        }
        case "job_result": {
          const result = unsealJson<JobResult>(machine.boxSecret, ciphertext);
          this.pending.get(result.job_id)?.(result.outcome === "sent" ? "sent" : result.outcome === "pass" ? "pass" : "error");
          break;
        }
        default:
          break;
      }
    } catch (error) {
      console.warn(`${blob.kind} blob`, error instanceof Error ? error.message : error);
    }
  }

  private onRoster(roster: RosterBlob) {
    const { removed } = applyRoster(roster);
    for (const chatId of removed) this.cancelChat(chatId);
  }

  private onChatOp(op: ChatBlob) {
    switch (op.op) {
      case "upsert": {
        const message = op.message;
        const { added } = upsertMessage(message);
        const fromBot = message.author.kind === "bot";
        if (added && fromBot && isComplete(message) && useStore.getState().openChatId !== message.chat_id) {
          bumpUnread(message.chat_id);
        }
        if (fromBot && message.body.kind === "text") setStatus(message.chat_id, null);
        break;
      }
      case "remove":
        removeMessage(op.chat_id, op.message_id);
        break;
      case "clear_unread":
        mutate((s) => ({ chats: s.chats.map((c) => (c.id === op.chat_id ? { ...c, unread_count: 0 } : c)) }));
        break;
    }
  }

  // MARK: - Sending

  private notice(chatId: string, text: string) {
    const message: Message = { id: `msg-${uuid()}`, chat_id: chatId, author: { kind: "system" }, body: { kind: "notice", text }, state: { kind: "complete" }, created_at: nowSecs() };
    upsertMessage(message);
    this.pushChatOp({ op: "upsert", message });
  }

  /// Appends the user's message, uploads it (files first, so the Runner has the bytes before
  /// the message that names them), and starts the turns it calls for.
  async sendMessage(chatId: string, text: string, files: PickedFile[] = []): Promise<Message | null> {
    const trimmed = text.trim();
    const chat = chatById(chatId);
    if ((!trimmed && files.length === 0) || !chat || !this.machine) return null;
    const attachments: Attachment[] = [];
    for (const file of files) attachments.push(await this.storeAttachment(file));
    const body: Message["body"] = attachments.length ? { kind: "text", text: trimmed, attachments } : { kind: "text", text: trimmed };
    const message: Message = {
      id: `msg-${uuid()}`,
      chat_id: chatId,
      author: { kind: "you" },
      body,
      state: { kind: "complete" },
      created_at: nowSecs(),
    };
    upsertMessage(message);
    setStatus(chatId, null);
    this.pushChatOp({ op: "upsert", message });

    if (chat.kind === "group") {
      const members = turnOrder(chat, trimmed);
      const previous = this.roomQueue.get(chatId) ?? Promise.resolve();
      const next = previous.then(() => this.runRoom(chatId, message.id, members));
      this.roomQueue.set(chatId, next);
      void next.finally(() => {
        if (this.roomQueue.get(chatId) === next) this.roomQueue.delete(chatId);
      });
    } else {
      const bot = chat.bot_ids[0] ? botById(chat.bot_ids[0]) : undefined;
      if (bot) void this.remoteTurn(this.userTurnJob(chatId, bot.id, message.id), new AbortController().signal, true);
    }
    return message;
  }

  private userTurnJob(chatId: string, botId: string, triggerMessageId: string): Job {
    return {
      id: `job-${uuid()}`,
      chat_id: chatId,
      bot_id: botId,
      kind: "turn",
      trigger_message_id: triggerMessageId,
      requested_by: this.machine?.pubkey ?? "",
      hops: 0,
      round: 0,
      is_winding_down: false,
      created_at: nowSecs(),
    };
  }

  /// Seals the job to the bot's Runner. `true` when it left for a Runner that can answer.
  private dispatch(job: Job): boolean {
    const bot = botById(job.bot_id);
    if (!bot) return false;
    const runner = deviceById(bot.runner_id);
    if (!runner || !runner.box_pubkey) {
      this.notice(job.chat_id, `${bot.name} is assigned to a Runner this Device does not know yet.`);
      return false;
    }
    try {
      this.pushBlob("job", runner.id, sealJson(runner.box_pubkey, job));
    } catch (error) {
      this.notice(job.chat_id, `Could not address the job to ${runner.name}: ${error instanceof Error ? error.message : error}`);
      return false;
    }
    if (!deviceIsOnline(runner.id)) {
      this.notice(job.chat_id, `${bot.name} runs on ${runner.name}, which is offline. This turn waits on the relay until it reconnects.`);
      return false;
    }
    return true;
  }

  /// One turn on another Runner: the job goes out, the Runner's job_result comes back.
  private async remoteTurn(job: Job, cancel: AbortSignal, noteSilence: boolean): Promise<Outcome> {
    if (!this.dispatch(job)) return "error";
    setRunning(job.id, { chatId: job.chat_id, botId: job.bot_id });
    const outcome = await new Promise<Outcome>((resolve) => {
      const timer = setTimeout(() => finish("error"), REMOTE_TURN_TIMEOUT_MS);
      const onCancel = () => finish("error");
      const finish = (o: Outcome) => {
        clearTimeout(timer);
        cancel.removeEventListener("abort", onCancel);
        this.pending.delete(job.id);
        resolve(o);
      };
      cancel.addEventListener("abort", onCancel);
      this.pending.set(job.id, finish);
    });
    setRunning(job.id, null);
    if (noteSilence) this.noteSilence(job.chat_id);
    return outcome;
  }

  /// "Chef stopped without replying" when a turn ends and the user's message is still last.
  private noteSilence(chatId: string) {
    const chat = chatById(chatId);
    if (!chat) return;
    const last = [...chat.messages].reverse().find((m) => m.body.kind === "text" || m.body.kind === "handoff");
    if (last?.author.kind === "you") {
      const name = chat.kind === "dm" ? botById(chat.bot_ids[0] ?? "")?.name : undefined;
      setStatus(chatId, `${name ?? chatTitle(chat)} stopped without replying`);
    }
  }

  /// One group exchange, run from this Device: each member gets a turn in order and either
  /// speaks or passes; the room goes another round while anyone spoke. A later message in
  /// the same group waits for this exchange, as it does on a Runner.
  private async runRoom(chatId: string, trigger: string, members: Bot[]) {
    if (members.length === 0 || !chatById(chatId)) return;
    const cancel = new AbortController();
    this.roomCancels.set(chatId, cancel);
    const roomId = `room-${uuid()}`;
    setRunning(roomId, { chatId, botId: "" });
    const seen = new Map<string, number>();
    try {
      rounds: for (let round = 1; round <= MAX_ROOM_ROUNDS; round++) {
        let anySent = false;
        for (const bot of members) {
          if (cancel.signal.aborted) break rounds;
          const chat = chatById(chatId);
          if (!chat) break rounds;
          if (!chat.bot_ids.includes(bot.id)) continue;
          if (round > 1 && seen.get(bot.id) === heardCount(chat, bot.id)) continue;
          const job: Job = {
            id: `job-${uuid()}`,
            chat_id: chatId,
            bot_id: bot.id,
            kind: "room_turn",
            trigger_message_id: trigger,
            requested_by: this.machine?.pubkey ?? "",
            hops: 0,
            round,
            is_winding_down: round === MAX_ROOM_ROUNDS,
            created_at: nowSecs(),
          };
          const outcome = await this.remoteTurn(job, cancel.signal, false);
          const after = chatById(chatId);
          if (after) seen.set(bot.id, heardCount(after, bot.id));
          if (outcome === "sent") anySent = true;
        }
        if (!anySent) break;
      }
    } finally {
      setRunning(roomId, null);
      if (this.roomCancels.get(chatId) === cancel) this.roomCancels.delete(chatId);
      this.noteSilence(chatId);
    }
  }

  /// Ends this Device's part in a chat's turns: the room loop and any wait for a result. Used
  /// when a chat goes away; there is no Stop in the UI, as in Grok Bot.
  private cancelChat(chatId: string) {
    this.roomCancels.get(chatId)?.abort();
    this.roomQueue.delete(chatId);
    for (const [jobId, running] of Object.entries(useStore.getState().running)) {
      if (running.chatId === chatId) this.pending.get(jobId)?.("error");
    }
  }

  // MARK: - Roster edits

  createBot(input: { name: string; tagline: string; instructions: string; symbol_name: string; accent: string; runner_id: string; provider: string; model?: string }): { bot: Bot; chat: Chat } {
    const runner = deviceById(input.runner_id);
    if (!runner) throw new Error("Unknown Runner");
    if (!isRunner(runner)) throw new Error(`${runner.name} runs ${runner.os} and cannot run bots`);
    const bot: Bot = {
      id: `bot-${uuid().slice(0, 8)}`,
      name: input.name.trim(),
      tagline: input.tagline.trim() || "New bot",
      symbol_name: input.symbol_name,
      accent: input.accent,
      runner_id: input.runner_id,
      provider: input.provider,
      instructions: input.instructions.trim(),
      created_at: nowSecs(),
    };
    if (input.model) bot.model = input.model;
    const chat: Chat = {
      id: `chat-${uuid().slice(0, 8)}`,
      kind: "dm",
      title: null,
      bot_ids: [bot.id],
      owner_bot_id: bot.id,
      is_pinned: false,
      created_at: nowSecs(),
      messages: [],
      unread_count: 0,
    };
    mutate((s) => ({ bots: [...s.bots, bot], chats: [chat, ...s.chats] }));
    this.pushRoster();
    return { bot, chat };
  }

  updateBot(id: string, update: Partial<Bot>) {
    mutate((s) => ({ bots: s.bots.map((b) => (b.id === id ? { ...b, ...update } : b)) }));
    this.pushRoster();
  }

  createGroup(title: string, botIds: string[]): Chat {
    const ids = botIds.slice(0, MAX_GROUP_BOTS);
    if (ids.length === 0) throw new Error("A chat needs at least one bot");
    const chat: Chat = {
      id: `chat-${uuid().slice(0, 8)}`,
      kind: "group",
      title: title.trim() || null,
      bot_ids: ids,
      owner_bot_id: ids[0],
      is_pinned: false,
      created_at: nowSecs(),
      messages: [],
      unread_count: 0,
    };
    mutate((s) => ({ chats: [chat, ...s.chats] }));
    this.pushRoster();
    return chat;
  }

  renameChat(chatId: string, title: string) {
    updateChatMeta(chatId, (meta) => ({ ...meta, title: title.trim() || null }));
    this.pushRoster();
  }

  pinChat(chatId: string, pinned: boolean) {
    updateChatMeta(chatId, (meta) => ({ ...meta, is_pinned: pinned }));
    this.pushRoster();
  }

  deleteChat(chatId: string) {
    this.cancelChat(chatId);
    mutate((s) => ({ chats: s.chats.filter((c) => c.id !== chatId) }));
    this.pushRoster();
  }

  addBot(chatId: string, botId: string) {
    updateChatMeta(chatId, (meta) => (meta.bot_ids.includes(botId) || meta.bot_ids.length >= MAX_GROUP_BOTS ? meta : { ...meta, bot_ids: [...meta.bot_ids, botId] }));
    this.pushRoster();
  }

  removeBot(chatId: string, botId: string) {
    updateChatMeta(chatId, (meta) => {
      if (meta.bot_ids.length <= 1) return meta;
      const bot_ids = meta.bot_ids.filter((id) => id !== botId);
      const owner_bot_id = meta.owner_bot_id === botId ? bot_ids[0] : meta.owner_bot_id;
      return { ...meta, bot_ids, owner_bot_id };
    });
    this.pushRoster();
  }

  setOwner(chatId: string, botId: string) {
    updateChatMeta(chatId, (meta) => (meta.bot_ids.includes(botId) ? { ...meta, owner_bot_id: botId } : meta));
    this.pushRoster();
  }

  // MARK: - This Device

  async renameDevice(name: string) {
    const file = useStore.getState().machineFile;
    if (!file) return;
    const next = { ...file, name: name.trim() || hostFacts().name };
    await saveMachineFile(next);
    useStore.setState({ machineFile: next });
    this.pushMachineBlobIfChanged();
  }

  async pair(pairingString: string, deviceName: string | undefined, onProgress?: Parameters<typeof acceptPairing>[3], signal?: AbortSignal) {
    const facts = hostFacts();
    const host = { ...facts, name: deviceName?.trim() || facts.name };
    const { file, device } = await acceptPairing(this.relay, pairingString, host, onProgress, signal);
    await saveMachineFile(file);
    resetStore(file);
    upsertDevice(device);
    this.bind(file);
  }

  /// Forgets the identity: keys, account key, and everything synced.
  async unpair() {
    for (const cancel of this.roomCancels.values()) cancel.abort();
    this.roomCancels.clear();
    this.pending.clear();
    await saveMachineFile(null);
    wipeState();
    this.bind(null);
    resetStore(null);
  }
}

export const engine = new Engine();

// MARK: - Helpers

/// Members in chat order, with the ones the message names by `@` first.
export function turnOrder(chat: ChatMeta, text: string): Bot[] {
  const members = chat.bot_ids.map(botById).filter((b): b is Bot => !!b);
  const lowered = text.toLowerCase();
  const mentioned = members.filter((b) => lowered.includes(`@${b.name.toLowerCase()}`));
  const rest = members.filter((b) => !mentioned.includes(b));
  return [...mentioned, ...rest];
}

/// Messages in the chat that `botId` did not write itself.
function heardCount(chat: Chat, botId: string): number {
  return chat.messages.filter((m) => isComplete(m) && (m.body.kind === "text" || m.body.kind === "handoff") && !(m.author.kind === "bot" && m.author.bot_id === botId)).length;
}

export function chatTitle(chat: ChatMeta): string {
  if (chat.title?.trim()) return chat.title.trim();
  const names = chat.bot_ids.map((id) => botById(id)?.name).filter((n): n is string => !!n);
  if (chat.kind === "dm") return names[0] ?? "Chat";
  return names.length ? names.join(", ") : "Group";
}
