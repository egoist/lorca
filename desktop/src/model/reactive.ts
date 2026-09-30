// The store's events as Solid signals. The store is plain and synchronous; a view reads what it
// shows from the store inside a memo or JSX that also reads the signal for the events that change
// it, the way the macOS views observe the store's events.

import { createSignal } from "solid-js";
import { store, type StoreEvent } from "./store";

function counter() {
  const [read, write] = createSignal(0);
  return { read, bump: () => write((value) => value + 1) };
}

const chats = counter();
const roster = counter();
const connection = counter();
const everything = counter();
const perChat = new Map<string, ReturnType<typeof counter>>();
const listeners = new Set<(event: StoreEvent) => void>();

function chatCounter(id: string) {
  let found = perChat.get(id);
  if (!found) {
    found = counter();
    perChat.set(id, found);
  }
  return found;
}

store.subscribe((event) => {
  everything.bump();
  switch (event.kind) {
    case "snapshotReplaced":
      chats.bump();
      roster.bump();
      connection.bump();
      for (const chat of perChat.values()) chat.bump();
      break;
    case "rosterChanged":
      roster.bump();
      break;
    case "chatsChanged":
      chats.bump();
      break;
    case "chatChanged":
      chats.bump();
      chatCounter(event.chatID).bump();
      break;
    case "messageAdded":
    case "messageChanged":
    case "messageRemoved":
    case "respondingChanged":
    case "olderMessagesLoaded":
    case "runningTasksChanged":
    case "turnFinished":
      chatCounter(event.chatID).bump();
      break;
    case "connectionChanged":
    case "identityChanged":
      connection.bump();
      break;
  }
  for (const listener of listeners) listener(event);
});

/** Read inside a memo or JSX to recompute when the store's events say what it shows changed. */
export const track = {
  /** The chat list: order, previews, unread counts, working dots. */
  chats: () => chats.read(),
  /** Bots, Devices, providers, routines, Auto-review, the relay's state. */
  roster: () => roster.read(),
  /** Whether the CLI answers, and the identity. */
  connection: () => connection.read(),
  /** One chat's messages, turns, and usage. */
  chat: (id: string) => chatCounter(id).read(),
  /** Any change at all. */
  any: () => everything.read(),
};

/** Runs `listener` after the store's event has reached the signals. */
export function onStoreEvent(listener: (event: StoreEvent) => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}
