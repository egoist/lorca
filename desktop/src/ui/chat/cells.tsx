// The transcript's rows, after the macOS app's ChatCells: bubbles (the user's on the right in the
// accent color, a bot's on the left; in a group the bot's name above its first bubble and its
// avatar beside the bubble's bottom edge), the working row, markers for messages between bots,
// notices, day separators, and status lines.

import { For, Show } from "solid-js";
import { files, preferences } from "../../host";
import { L } from "../../l10n";
import * as Format from "../../model/format";
import { isImage, sizeText, type Attachment, type Bot, type Message, type ToolInvocation } from "../../model/models";
import { track } from "../../model/reactive";
import { store } from "../../model/store";
import { Avatar, authorAvatar, botAvatar, type AvatarContent } from "../avatar";
import { Icon } from "../icons";
import { Markdown } from "../markdown";
import { firstLineOf, showTextPopover } from "../overlay";

export const ChatMetrics = {
  horizontalInset: 22,
  avatarSize: 26,
  avatarGutter: 10,
  groupTopPadding: 14,
  tightTopPadding: 4,
  maxBubbleWidth: 580,
  userLeftGutter: 72,
  get bubbleIndent() {
    return this.horizontalInset + this.avatarSize + this.avatarGutter;
  },
};

// MARK: - Attachments

const imageMax = 220;
const imageMin = 72;

/** An image's box, from the width and height the message carries, before the bytes are here. */
function imageSize(attachment: Attachment): { width: number; height: number } {
  const width = Math.max(attachment.width ?? 4, 1);
  const height = Math.max(attachment.height ?? 3, 1);
  const scale = Math.min(imageMax / width, imageMax / height, 1);
  return { width: Math.max(imageMin, Math.ceil(width * scale)), height: Math.max(imageMin, Math.ceil(height * scale)) };
}

/** The tiles by row: images side by side, wrapping at the bubble's edge, and each file card on a
 * row of its own. */
function tileRows(attachments: Attachment[]): Attachment[][] {
  const rows: Attachment[][] = [];
  for (const attachment of attachments) {
    const last = rows[rows.length - 1];
    if (last && isImage(attachment) && isImage(last[0]!)) last.push(attachment);
    else rows.push([attachment]);
  }
  return rows;
}

/** Thumbnails and file cards inside a bubble, above the text, 8 above it or 4 above a lone time.
 * A click opens the file. */
function AttachmentTiles(props: { attachments: Attachment[]; chatID: string; messageID: string; onUserBubble: boolean; below: "text" | "stamp" | "none" }) {
  return (
    <div class={["bubble-attachments", `below-${props.below}`]}>
      <For each={tileRows(props.attachments)}>
        {(row) => (
          <div class="attachment-row">
            <For each={row}>{(attachment) => <AttachmentTile attachment={attachment} chatID={props.chatID} messageID={props.messageID} onUserBubble={props.onUserBubble} />}</For>
          </div>
        )}
      </For>
    </div>
  );
}

function AttachmentTile(props: { attachment: Attachment; chatID: string; messageID: string; onUserBubble: boolean }) {
  const attachment = props.attachment;
  // The bytes of a file sent from another Device land later; the message's change brings them in.
  const file = () => {
    track.chat(props.chatID);
    return store.localFile(attachment, props.chatID, props.messageID);
  };
  const open = () => {
    const known = file();
    if (known) void files.open(known.path);
  };
  return isImage(attachment) ? (
    <button
      class="attachment-image"
      style={{ width: `${imageSize(attachment).width}px`, height: `${imageSize(attachment).height}px` }}
      title={file() ? attachment.name : L("%@ · fetching…", attachment.name)}
      aria-label={attachment.name}
      onClick={open}
    >
      <Show when={file()?.url}>{(url) => <img src={url()} alt="" draggable="false" />}</Show>
    </button>
  ) : (
    <button class={["attachment-file", { "on-user": props.onUserBubble }]} title={file() ? attachment.name : L("%@ · fetching…", attachment.name)} onClick={open}>
      <Icon name="doc.fill" size={20} strokeWidth={1.8} />
      <span class="attachment-file-text">
        <span class="attachment-file-name truncate">{attachment.name}</span>
        <span class="attachment-file-size">{sizeText(attachment.size)}</span>
      </span>
    </button>
  );
}

// MARK: - Message

export function MessageCell(props: { message: Message; groupStart: boolean; chatID: string; showsAvatar: boolean }) {
  const isUser = () => props.message.author.kind === "you";
  const text = () => (props.message.body.kind === "text" ? props.message.body.text : "");
  // A bot's name and look follow a rename or a new image.
  const bot = () => {
    track.roster();
    return props.message.author.kind === "bot" ? store.bot(props.message.author.botID) : undefined;
  };
  const avatar = () => {
    track.roster();
    return authorAvatar(props.message.author);
  };
  const showsName = () => props.showsAvatar && props.groupStart;
  const indent = () => (props.showsAvatar ? ChatMetrics.bubbleIndent : ChatMetrics.horizontalInset);
  return (
    <div
      class={["message-row", { user: isUser(), tight: !props.groupStart }]}
      style={{ "padding-left": isUser() ? `${ChatMetrics.horizontalInset + ChatMetrics.userLeftGutter}px` : `${indent()}px` }}
    >
      <div class="message-column">
        <Show when={showsName()}>
          <div class="message-author" style={{ color: bot() ? `var(--${bot()!.accent})` : "var(--label-2)" }}>
            {bot()?.name ?? L("Bot")}
          </div>
        </Show>
        <div class={["bubble", isUser() ? "user" : "bot"]}>
          <Show when={props.message.attachments.length > 0}>
            <AttachmentTiles
              attachments={props.message.attachments}
              chatID={props.chatID}
              messageID={props.message.id}
              onUserBubble={isUser()}
              below={text() !== "" ? "text" : preferences().showTimestamps ? "stamp" : "none"}
            />
          </Show>
          <Show when={text() !== "" || preferences().showTimestamps}>
            <div class={["bubble-body", { "attachments-only": text() === "" }]}>
              <Show when={text() !== ""}>
                <Markdown text={text()} onUserBubble={isUser()} class="bubble-text" />
              </Show>
              <Show when={preferences().showTimestamps}>
                <span class="bubble-stamp">{Format.time(props.message.createdAt)}</span>
              </Show>
            </div>
          </Show>
        </div>
      </div>
      <Show when={showsName()}>
        <span class="message-avatar">
          <Avatar content={avatar()} size={ChatMetrics.avatarSize} />
        </span>
      </Show>
    </div>
  );
}

// MARK: - Working row

/** What a tool row means while it runs, in the words of the status line. `pluginName` is the
 * plugin behind a `<plugin>__<tool>` row, so the line reads "Using GitHub" between two calls as
 * well as during one. */
export function toolActivity(tool: ToolInvocation, targetName: string | undefined, pluginName?: string): string {
  switch (tool.name) {
    case "read":
      return L("Reading a file");
    case "write":
    case "edit":
      return L("Drafting a file");
    case "bash":
      return tool.description ? L("Running command: %@", tool.description) : L("Running commands");
    case "web_search":
      return L("Searching the web");
    case "web_fetch":
      return L("Reading the web");
    case "grep":
    case "find":
    case "ls":
      return L("Searching files");
    case "message_bot":
      return targetName ? L("Messaging %@", targetName) : L("Messaging another bot");
    case "list_teammates":
      return L("Checking the team");
    case "create_bot":
      return L("Creating a bot");
    case "edit_bot":
      return L("Updating a bot");
    case "memory_update":
    case "memory_log":
      return L("Taking a note");
    case "search_plugins":
      return L("Searching plugins");
    case "install_plugin":
      return L("Installing a plugin");
    case "bash_input":
      return L("Answering a command");
    case "bash_output":
      return L("Waiting on a command");
    // A codemode script: the plugin of its latest plugin call, which the CLI names.
    case "codemode":
      return tool.description ? L("Using %@", tool.description) : L("Working");
    default:
      if (pluginName) return L("Using %@", pluginName);
      if (tool.name.includes("__") && tool.summary.startsWith("Using ")) return tool.summary.endsWith("…") ? tool.summary.slice(0, -1) : tool.summary;
      return L("Working");
  }
}

export function workingText(names: string[], activity: string | undefined, showsName: boolean): string {
  if (names.length > 1) return L("%@ and %@ are working…", names.slice(0, -1).join(L(", ")), names[names.length - 1]!);
  if (activity) return `${activity}…`;
  if (showsName && names[0]) return L("%@ is working…", names[0]);
  return `${L("Working")}…`;
}

/** "Chef is working…" after the last message, its words shimmering while a turn runs. A DM reads
 * "Working…"; with one bot at work the line says what it is doing. */
export function WorkingCell(props: { bots: Bot[]; activity?: string; showsName: boolean }) {
  const text = () => workingText(props.bots.map((bot) => bot.name), props.activity, props.showsName);
  return (
    <div class="working-row" aria-label={props.bots.length === 1 ? L("%@ is working", props.bots[0]!.name) : text()}>
      <span class="working-avatar">
        <Show when={props.bots[0]}>{(bot) => <Avatar content={botAvatar(bot())} size={ChatMetrics.avatarSize} />}</Show>
      </span>
      <span class="working-text shimmer truncate">{text()}</span>
    </div>
  );
}

// MARK: - Status and day rows

/** A quiet centered line, such as "Chef stopped without replying". */
export function StatusCell(props: { text: string }) {
  return <div class="transcript-status">{props.text}</div>;
}

export function DayCell(props: { at: number }) {
  return (
    <div class="day-row">
      <span class="day-pill">{Format.daySeparator(props.at)}</span>
    </div>
  );
}

export function NoticeCell(props: { text: string; groupStart: boolean }) {
  return (
    <div class={["notice-row", { tight: !props.groupStart }]}>
      <div class="notice-box">
        <Icon name="clock.badge.questionmark" size={13} strokeWidth={2} class="notice-icon" />
        <span class="notice-text">{props.text}</span>
      </div>
    </div>
  );
}

// MARK: - Bot-to-bot markers

export type HandoffMode = { kind: "incoming"; from?: Bot } | { kind: "outgoing"; to?: Bot } | { kind: "handoff"; from?: Bot; to?: Bot };

export function handoffSpokenText(mode: HandoffMode, reason: string): string {
  const text = reason.trim();
  switch (mode.kind) {
    case "incoming":
      return `${L("Message from")} ${mode.from?.name ?? "?"}: ${text}`;
    case "outgoing":
      return `${L("Messaged")} ${mode.to?.name ?? "?"}: ${text}`;
    case "handoff":
      return `${L("%@ handed off to %@", mode.from?.name ?? "?", mode.to?.name ?? "?")}: ${text}`;
  }
}

/** Bot-to-bot messages as markers: "Message from ◉ Name · first line" where one arrived,
 * "Messaged ◉ Name · first line" where one was sent, centered; a handoff between two bots in the
 * same chat keeps both avatars on one line. A click opens the whole message. */
export function HandoffCell(props: { mode: HandoffMode; reason: string; groupStart: boolean }) {
  let marker: HTMLButtonElement | undefined;
  const full = () => props.reason.trim();
  const first = () => firstLineOf(full());
  const avatar = (bot: Bot | undefined): AvatarContent => (bot ? botAvatar(bot) : { kind: "system" });
  return (
    <div class={["handoff-row", props.mode.kind === "handoff" ? "handoff" : "centered", { tight: !props.groupStart }]}>
      <button
        ref={(el) => (marker = el)}
        class={["handoff-marker", { clickable: full() !== "" }]}
        aria-label={handoffSpokenText(props.mode, props.reason)}
        onClick={() => {
          if (marker && full()) showTextPopover(marker, full());
        }}
      >
        <Show
          when={props.mode.kind === "handoff" ? (props.mode as Extract<HandoffMode, { kind: "handoff" }>) : null}
          fallback={
            <>
              <span class="handoff-lead">{props.mode.kind === "incoming" ? L("Message from") : L("Messaged")}</span>
              <Avatar
                content={avatar(props.mode.kind === "incoming" ? props.mode.from : props.mode.kind === "outgoing" ? props.mode.to : undefined)}
                size={16}
              />
              <span class="handoff-name">
                {(props.mode.kind === "incoming" ? props.mode.from?.name : props.mode.kind === "outgoing" ? props.mode.to?.name : undefined) ?? "?"}
              </span>
            </>
          }
        >
          {(mode) => (
            <>
              <Avatar content={avatar(mode().from)} size={18} />
              <Icon name="arrow.right" size={11} strokeWidth={2.4} class="handoff-arrow" />
              <Avatar content={avatar(mode().to)} size={18} />
              <span class="handoff-label">{L("%@ handed off to %@", mode().from?.name ?? "?", mode().to?.name ?? "?")}</span>
            </>
          )}
        </Show>
        <Show when={first() !== ""}>
          <span class="handoff-preview truncate">· {first()}</span>
        </Show>
      </button>
    </div>
  );
}
