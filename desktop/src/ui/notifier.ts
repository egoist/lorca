// System notifications for replies, failed responses, and questions, after the macOS app's Notifier
// and ChatNotification: posted unless the message was read on a paired Device first, and a click
// brings the app forward on the chat. The same "looking at" fact goes to the CLI (`ui.watching`), so
// a Runner does not push a reply the user is watching arrive to their phone.

import { notices, watchWindowState, type WindowState } from "../host";
import { L } from "../l10n";
import { authorBotID, firstLine, isDM, isPending, type Chat, type Message } from "../model/models";
import { store } from "../model/store";
import { selection } from "./root";

/** The message behind an alert, kept so a delayed delivery can check its current state. */
interface ChatNotification {
  kind: "reply" | "permission" | "failure";
  messageID: string;
  botID: string;
  body: string;
}

function notificationFor(message: Message): ChatNotification | null {
  const botID = authorBotID(message.author);
  if (!botID) return null;
  const base = { messageID: message.id, botID };
  const body = message.body;
  if (body.kind === "permission" && isPending(body.request)) return { ...base, kind: "permission", body: L("Confirmation needed: %@", body.request.summary) };
  if (body.kind === "tool" && body.tool.run?.state === "asking") return { ...base, kind: "permission", body: L("Confirmation needed: %@", `$ ${firstLine(body.tool.run)}`) };
  if (body.kind === "text") {
    if (message.state.kind === "failed") return { ...base, kind: "failure", body: L("Reply failed: %@", message.state.error) };
    if (message.state.kind === "complete" && body.text.trim() !== "") return { ...base, kind: "reply", body: body.text };
  }
  return null;
}

function same(a: ChatNotification, b: ChatNotification): boolean {
  return a.kind === b.kind && a.messageID === b.messageID && a.botID === b.botID && a.body === b.body;
}

/** The last terminal reply of a turn wins, so a failure after partial output shows the error. */
function finishedTurn(chat: Chat, botID: string, startedAt: number): ChatNotification | null {
  for (let index = chat.messages.length - 1; index >= 0; index--) {
    const message = chat.messages[index]!;
    if (authorBotID(message.author) !== botID || message.createdAt < startedAt - 5000 || message.body.kind !== "text") continue;
    const notification = notificationFor(message);
    if (notification) return notification;
  }
  return null;
}

/** Still unread, not on screen, and the message still says what the alert would. */
function canDeliver(notification: ChatNotification, chat: Chat, watched: string | null): boolean {
  if (watched === chat.id || chat.unreadCount <= 0) return false;
  const message = chat.messages.find((each) => each.id === notification.messageID);
  const current = message ? notificationFor(message) : null;
  return current !== null && same(current, notification);
}

const delay = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export class Notifier {
  private started = false;
  private pendingPermissions = new Set<string>();
  private window: WindowState = { focused: false, visible: false, minimized: false, maximized: false };
  private stops: (() => void)[] = [];

  /** The chat the user is looking at: the main window is in front and shows it. */
  private get watchedChat(): string | null {
    const current = selection.get();
    if (!this.window.focused || !this.window.visible || this.window.minimized || current?.kind !== "chat") return null;
    return current.id;
  }

  start(): void {
    if (this.started) return;
    this.started = true;
    this.rememberPermissions();
    this.stops.push(
      store.subscribe((event) => {
        switch (event.kind) {
          case "connectionChanged":
            this.watchingChanged();
            break;
          case "snapshotReplaced":
          case "identityChanged":
            this.rememberPermissions();
            this.watchingChanged();
            break;
          case "messageAdded":
          case "messageChanged":
            this.permissionChanged(event.chatID, event.messageID);
            break;
          case "messageRemoved":
            this.clearPermission(event.messageID);
            break;
          case "turnFinished":
            void this.turnFinished(event.chatID, event.botID, event.startedAt);
            break;
        }
      }),
      watchWindowState((state) => {
        this.window = state;
        this.watchingChanged();
      }),
    );
    this.watchingChanged();
  }

  stop(): void {
    for (const stop of this.stops) stop();
    this.stops = [];
    this.started = false;
  }

  /** Call when the selection or the window's visibility changes. */
  watchingChanged(): void {
    if (!this.started) return;
    const watched = this.watchedChat;
    store.setWatchedChat(watched);
    // Whatever was posted for the chat now on screen has been seen.
    if (watched) void notices.clearChat(watched);
  }

  private rememberPermissions(): void {
    const current = new Set<string>();
    for (const chat of store.chats) {
      for (const message of chat.messages) {
        const body = message.body;
        if ((body.kind === "permission" && isPending(body.request)) || (body.kind === "tool" && body.tool.run?.state === "asking")) current.add(message.id);
      }
    }
    for (const id of this.pendingPermissions) if (!current.has(id)) this.clearPermission(id);
    this.pendingPermissions = current;
  }

  private clearPermission(messageID: string): void {
    this.pendingPermissions.delete(messageID);
    void notices.remove(messageID);
  }

  private permissionChanged(chatID: string, messageID: string): void {
    const message = store.chat(chatID)?.messages.find((each) => each.id === messageID);
    if (!message) return;
    // A permission card, or a command's card, asks.
    const body = message.body;
    let asks: boolean;
    if (body.kind === "permission") asks = isPending(body.request);
    else if (body.kind === "tool" && body.tool.run) asks = body.tool.run.state === "asking";
    else return;
    if (!asks) {
      this.clearPermission(messageID);
      return;
    }
    if (this.pendingPermissions.has(messageID)) return;
    this.pendingPermissions.add(messageID);
    const identityID = store.identityID;
    void delay(3000).then(() => {
      const current = store.chat(chatID)?.messages.find((each) => each.id === messageID);
      const notification = current ? notificationFor(current) : null;
      if (store.identityID !== identityID || notification?.kind !== "permission") return;
      this.post(notification, chatID);
    });
  }

  private async turnFinished(chatID: string, botID: string, startedAt: number): Promise<void> {
    // Give the final reply and read marks from paired Devices three seconds to arrive.
    const identityID = store.identityID;
    await delay(3000);
    if (store.identityID !== identityID) return;
    const chat = store.chat(chatID);
    const notification = chat ? finishedTurn(chat, botID, startedAt) : null;
    if (notification) this.post(notification, chatID);
  }

  private post(notification: ChatNotification, chatID: string): void {
    const chat = store.chat(chatID);
    if (!chat || !canDeliver(notification, chat, this.watchedChat)) return;
    void notices.show({
      id: notification.messageID,
      chatId: chatID,
      title: store.bot(notification.botID)?.name ?? store.title(chat),
      subtitle: isDM(chat) ? "" : store.title(chat),
      body: notification.body.split(/[\r\n]+/).filter((line) => line !== "").join(" ").slice(0, 280),
    });
  }
}
