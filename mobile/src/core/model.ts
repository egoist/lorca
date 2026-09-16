// Plaintext domain model, the same JSON the CLI reads and writes inside encrypted blobs
// (crates/cli/src/model.rs). Field names are the wire names.

export const MAX_GROUP_BOTS = 6;
export const ONLINE_WINDOW_SECS = 150;

export interface Device {
  /// The machine signing public key, base64url.
  id: string;
  name: string;
  model: string;
  os: string;
  os_version: string;
  box_pubkey: string;
  providers_connected: string[];
  updated_at: number;
}

export function isRunner(device: Device): boolean {
  return device.os === "macos" || device.os === "linux" || device.os === "windows";
}

export type Accent = "indigo" | "blue" | "teal" | "green" | "orange" | "pink" | "purple" | "red";

export interface Bot {
  id: string;
  name: string;
  tagline: string;
  symbol_name: string;
  accent: Accent | string;
  runner_id: string;
  provider: string;
  model?: string;
  instructions: string;
  workdir?: string;
  created_at: number;
}

export type Author = { kind: "you" } | { kind: "bot"; bot_id: string } | { kind: "system" };

/// A file sent with a message. Its bytes travel as a `file` blob under this id, encrypted with
/// the account key; the phone keeps a copy in its documents directory once it has them.
export interface Attachment {
  id: string;
  name: string;
  mime: string;
  size: number;
  width?: number;
  height?: number;
}

export function isImage(attachment: Attachment): boolean {
  return attachment.mime.startsWith("image/");
}

/// "Photo", "3 photos", "report.pdf", "2 files": the preview of a message with no text.
export function attachmentSummary(attachments: Attachment[]): string {
  const first = attachments[0];
  if (!first) return "";
  if (attachments.length === 1) return isImage(first) ? "Photo" : first.name;
  return attachments.every(isImage) ? `${attachments.length} photos` : `${attachments.length} files`;
}

export function fileSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

export const MAX_ATTACHMENT_BYTES = 20 * 1024 * 1024;
export const MAX_ATTACHMENTS = 10;

export type Body =
  | { kind: "text"; text: string; attachments?: Attachment[] }
  | {
      kind: "tool";
      name: string;
      summary: string;
      detail: string;
      is_running: boolean;
      call_id?: string;
      arguments?: unknown;
      result?: string | null;
      is_error?: boolean;
    }
  | { kind: "handoff"; from: string; to: string; reason: string }
  | { kind: "notice"; text: string };

export type MessageState =
  | { kind: "thinking" }
  | { kind: "streaming" }
  | { kind: "complete" }
  | { kind: "failed"; error: string };

export interface Message {
  id: string;
  chat_id: string;
  author: Author;
  body: Body;
  state: MessageState;
  /// Unix seconds.
  created_at: number;
}

export function isComplete(message: Message): boolean {
  return message.state.kind === "complete";
}

/// A finished message_bot call: the one tool the transcript shows, as "Messaged ◉ Name".
export function isSentMessage(body: Body): body is Extract<Body, { kind: "tool" }> {
  return body.kind === "tool" && body.name === "message_bot" && !body.is_running && body.summary.startsWith("Messaged ");
}

export function recipientName(body: Extract<Body, { kind: "tool" }>): string {
  return body.summary.slice("Messaged ".length);
}

export interface ChatMeta {
  id: string;
  /// `dm` or `group`.
  kind: "dm" | "group";
  title?: string | null;
  bot_ids: string[];
  owner_bot_id?: string;
  is_pinned: boolean;
  created_at: number;
}

export interface Chat extends ChatMeta {
  messages: Message[];
  unread_count: number;
}

// MARK: - Blob payloads

export interface RosterBlob {
  bots: Bot[];
  chats: ChatMeta[];
  updated_at: number;
}

export type ChatBlob =
  | { op: "upsert"; message: Message }
  | { op: "remove"; chat_id: string; message_id: string }
  | { op: "clear_unread"; chat_id: string };

export interface MachineBlob {
  device: Device;
}

export interface Job {
  id: string;
  chat_id: string;
  bot_id: string;
  /// `turn` for a user message in a DM, `room_turn` for one member's turn in a group,
  /// `message` for a teammate's message_bot.
  kind: "turn" | "room_turn" | "message";
  trigger_message_id: string;
  requested_by: string;
  from_bot_id?: string;
  hops: number;
  round: number;
  is_winding_down: boolean;
  created_at: number;
}

export interface JobResult {
  job_id: string;
  chat_id: string;
  bot_id: string;
  /// `sent`, `pass`, or `error`.
  outcome: "sent" | "pass" | "error" | string;
}

export interface PairRequest {
  machine_pubkey: string;
  box_pubkey: string;
  device: Device;
}

export interface PairReply {
  identity_pubkey: string;
  content_pubkey: string;
  account_dek: string;
  relay_url: string;
}

export const PROVIDER_LABELS: Record<string, string> = {
  deepseek: "DeepSeek",
  chatgpt: "ChatGPT",
};

export function providerLabel(kind: string): string {
  return PROVIDER_LABELS[kind] ?? kind;
}
