// A chat, after the macOS app's ChatViewController: the transcript, which follows new messages
// while it rests at its end and otherwise holds what the user is reading in place, older pages
// asked for near the top, the composer floating over it, and the button back to the latest.

import { createEffect, createMemo, createSignal, For, Match, onSettled, Show, Switch } from "solid-js";
import { L, Lc } from "../../l10n";
import { fullCommand, isGroup, isSentMessage, verbPhrase, type Chat, type Message } from "../../model/models";
import { onStoreEvent, track } from "../../model/reactive";
import { store } from "../../model/store";
import { authorAvatar } from "../avatar";
import { shortcutText } from "../commands";
import { Icon } from "../icons";
import { chatActions } from "../root";
import { CommandCard, PermissionCard, presentCommandSheet } from "./cards";
import { DayCell, HandoffCell, MessageCell, NoticeCell, StatusCell, toolActivity, WorkingCell, type HandoffMode } from "./cells";
import { Composer, type ComposerHandle } from "./composer";
import { ChatEmptyState } from "./empty";
import { buildRows, type ChatRow } from "./rows";

/** How close to the end counts as resting there. */
const pinDistance = 48;

/** Keeps the rows that did not change as they were, so their views are left alone. */
function reuse(previous: ChatRow[], next: ChatRow[]): ChatRow[] {
  const byKey = new Map(previous.map((row) => [row.key, row]));
  return next.map((row) => {
    const old = byKey.get(row.key);
    if (!old || old.kind !== row.kind) return row;
    switch (row.kind) {
      case "message":
        return old.kind === "message" && old.message === row.message && old.groupStart === row.groupStart ? old : row;
      case "day":
        return old.kind === "day" && old.at === row.at ? old : row;
      case "working":
        return old.kind === "working" && old.botIDs.join() === row.botIDs.join() ? old : row;
      case "status":
        return old.kind === "status" && old.text === row.text ? old : row;
    }
  });
}

/** The plugin behind a `<plugin>__<tool>` row, by name, from the bot's Runner. */
function pluginName(toolName: string, botID: string): string | undefined {
  const at = toolName.indexOf("__");
  const bot = store.bot(botID);
  if (at < 0 || !bot) return undefined;
  const pluginID = toolName.slice(0, at);
  return store.device(bot.runnerID)?.plugins.find((plugin) => plugin.id === pluginID)?.name ?? pluginID.charAt(0).toUpperCase() + pluginID.slice(1);
}

/** What the one working bot is doing: its latest tool, running or just finished (so the line does
 * not flash back to its name between two commands), thinking, or a model call waiting to be asked
 * again, which outranks both. */
function activity(botIDs: string[], chat: Chat): string | undefined {
  let words: string | undefined;
  const only = botIDs.length === 1 ? botIDs[0]! : undefined;
  const last = chat.messages[chat.messages.length - 1];
  if (only && last && last.author.kind === "bot" && last.author.botID === only && last.body.kind === "tool" && !isSentMessage(last.body.tool)) {
    const tool = last.body.tool;
    const target = tool.targetBotID ? store.bot(tool.targetBotID) : undefined;
    words = toolActivity(tool, target?.name, pluginName(tool.name, only));
  }
  if (only && store.isThinking(only, chat.id)) words = Lc("Thinking", "status");
  const note = store.retryNote(chat.id);
  if (note) words = note;
  return words;
}

function botName(message: Message): string {
  return (message.author.kind === "bot" && store.bot(message.author.botID)?.name) || L("The bot");
}

function handoffMode(message: Message, chat: Chat): { mode: HandoffMode; reason: string } | undefined {
  const body = message.body;
  if (body.kind === "tool") return { mode: { kind: "outgoing", to: body.tool.targetBotID ? store.bot(body.tool.targetBotID) : undefined }, reason: body.tool.detail };
  if (body.kind === "handoff") {
    const incoming = !isGroup(chat) && chat.botIDs.includes(body.to);
    return {
      mode: incoming ? { kind: "incoming", from: store.bot(body.from) } : { kind: "handoff", from: store.bot(body.from), to: store.bot(body.to) },
      reason: body.reason,
    };
  }
  return undefined;
}

function RowView(props: { row: ChatRow; chat: Chat }) {
  const group = () => isGroup(props.chat);
  const message = () => (props.row.kind === "message" ? props.row.message : undefined);
  const groupStart = () => (props.row.kind === "message" ? props.row.groupStart : true);
  const showsAvatar = () => group() && message()?.author.kind === "bot";
  const cardAvatar = () => (showsAvatar() && message() ? authorAvatar(message()!.author) : undefined);
  return (
    <Switch>
      <Match when={props.row.kind === "day" && props.row}>{(row) => <DayCell at={(row() as Extract<ChatRow, { kind: "day" }>).at} />}</Match>
      <Match when={props.row.kind === "status" && props.row}>{(row) => <StatusCell text={(row() as Extract<ChatRow, { kind: "status" }>).text} />}</Match>
      <Match when={props.row.kind === "working" && props.row}>
        {(row) => {
          const botIDs = () => (row() as Extract<ChatRow, { kind: "working" }>).botIDs;
          return (
            <WorkingCell
              bots={botIDs().flatMap((id) => store.bot(id) ?? [])}
              activity={activity(botIDs(), props.chat)}
              showsName={group()}
            />
          );
        }}
      </Match>
      <Match when={message()?.body.kind === "text" && message()}>
        {(text) => <MessageCell message={text()} groupStart={groupStart()} chatID={props.chat.id} showsAvatar={showsAvatar()} />}
      </Match>
      <Match when={message()?.body.kind === "tool" && (message()!.body as { tool: { run?: unknown } }).tool.run !== undefined && message()}>
        {(command) => {
          const run = () => (command().body as Extract<Message["body"], { kind: "tool" }>).tool.run!;
          return (
            <CommandCard
              run={run()}
              messageID={command().id}
              botName={botName(command())}
              avatar={cardAvatar()}
              groupStart={groupStart()}
              onDecision={(decision) => store.answerPermission(props.chat.id, command().id, decision)}
              onSend={(text) => store.answerCommand(props.chat.id, command().id, text)}
              onStop={() => store.stopCommand(props.chat.id, command().id)}
              onShowCommand={() => presentCommandSheet(L("%@'s command", botName(command())), run().command)}
            />
          );
        }}
      </Match>
      <Match when={(message()?.body.kind === "tool" || message()?.body.kind === "handoff") && message()}>
        {(marker) => {
          const handoff = () => handoffMode(marker(), props.chat);
          return (
            <Show when={handoff()}>{(value) => <HandoffCell mode={value().mode} reason={value().reason} groupStart={groupStart()} />}</Show>
          );
        }}
      </Match>
      <Match when={message()?.body.kind === "notice" && message()}>
        {(notice) => <NoticeCell text={(notice().body as { text: string }).text} groupStart={groupStart()} />}
      </Match>
      <Match when={message()?.body.kind === "permission" && message()}>
        {(permission) => {
          const request = () => (permission().body as Extract<Message["body"], { kind: "permission" }>).request;
          return (
            <PermissionCard
              request={request()}
              botName={botName(permission())}
              avatar={cardAvatar()}
              groupStart={groupStart()}
              onDecision={(decision) => store.answerPermission(props.chat.id, permission().id, decision)}
              onShowCommand={() => presentCommandSheet(`${botName(permission())} ${verbPhrase(request())}`, fullCommand(request()))}
            />
          );
        }}
      </Match>
    </Switch>
  );
}

export function ChatView(props: { chatID: string; onRedirect: (chatID: string) => void }) {
  const [stoppedNotice, setStoppedNotice] = createSignal<string | null>(null);
  const [showsJump, setShowsJump] = createSignal(false);
  const [composerHeight, setComposerHeight] = createSignal(66);
  let scroller: HTMLDivElement | undefined;
  let content: HTMLDivElement | undefined;
  let composer: ComposerHandle | undefined;
  let pinned = true;
  /** What the user was reading when the rows above it changed: a message and its offset. */
  let anchor: { key: string; offset: number } | null = null;

  const chat = createMemo(() => {
    track.chat(props.chatID);
    track.chats();
    return store.chat(props.chatID);
  });
  let previousRows: ChatRow[] = [];
  const rows = createMemo(() => {
    track.chat(props.chatID);
    track.roster();
    const current = store.chat(props.chatID);
    const next = current ? reuse(previousRows, buildRows(current, store.workingBots(props.chatID), stoppedNotice())) : [];
    previousRows = next;
    return next;
  });
  const members = createMemo(() => {
    track.roster();
    const current = chat();
    return current ? store.botsIn(current) : [];
  });
  const placeholder = () => {
    const current = chat();
    if (!current) return "";
    const only = members()[0];
    if (current.kind === "dm" && only) return L("Message %@", only.name);
    const title = store.title(current);
    return members().length > 1 ? L("Message %@ — @ to address one bot", title) : L("Message %@", title);
  };
  /** Who `@` can address: members first, then every other bot. */
  const mentionable = () => {
    track.roster();
    const inChat = members();
    return [...inChat, ...store.bots.filter((bot) => !inChat.some((member) => member.id === bot.id))];
  };
  const isResponding = () => {
    track.chat(props.chatID);
    return store.isResponding(props.chatID);
  };

  const scrollToBottom = (animated: boolean) => {
    if (!scroller) return;
    scroller.scrollTo({ top: scroller.scrollHeight, behavior: animated ? "smooth" : "auto" });
    pinned = true;
    setShowsJump(false);
  };

  /** The first message starting on screen, and how far from the top it sits. */
  const captureAnchor = () => {
    if (!scroller || !content) return null;
    const top = scroller.getBoundingClientRect().top;
    for (const element of content.querySelectorAll<HTMLElement>("[data-row]")) {
      const rect = element.getBoundingClientRect();
      if (rect.bottom > top && element.dataset.message === "1") return { key: element.dataset.row!, offset: rect.bottom - top };
    }
    return null;
  };

  const restoreAnchor = () => {
    const held = anchor;
    anchor = null;
    if (!held || !scroller || !content) return;
    const element = content.querySelector<HTMLElement>(`[data-row="${CSS.escape(held.key)}"]`);
    if (!element) return;
    const top = scroller.getBoundingClientRect().top;
    scroller.scrollTop += element.getBoundingClientRect().bottom - top - held.offset;
  };

  // New rows land: a transcript resting at its end follows them; one scrolled back keeps the
  // message being read where it was.
  createEffect(rows, () => {
    if (anchor) restoreAnchor();
    else if (pinned) scrollToBottom(false);
  });

  const onScroll = () => {
    if (!scroller) return;
    const distance = scroller.scrollHeight - (scroller.scrollTop + scroller.clientHeight);
    pinned = distance < pinDistance;
    setShowsJump(!pinned && rows().length > 0);
    // Nearing the first message: ask for the page before it.
    if (scroller.scrollTop < 600) store.loadOlderMessages(props.chatID);
  };

  onSettled(() => {
    scrollToBottom(false);
    const offEvents = onStoreEvent((event) => {
      if (!("chatID" in event) || event.chatID !== props.chatID) return;
      switch (event.kind) {
        case "messageAdded":
          setStoppedNotice(null);
          break;
        case "respondingChanged": {
          const current = store.chat(props.chatID);
          const last = current?.messages[current.messages.length - 1];
          if (current && !store.isResponding(props.chatID) && last?.author.kind === "you") {
            setStoppedNotice(L("%@ stopped without replying", store.title(current)));
          }
          break;
        }
        case "olderMessagesLoaded":
          anchor = captureAnchor();
          break;
      }
    });
    // Images and wrapped text settle after the rows land: a pinned transcript keeps its end in view.
    const observer = new ResizeObserver(() => {
      if (pinned) scrollToBottom(false);
    });
    if (content) observer.observe(content);
    if (scroller) observer.observe(scroller);
    // Take the keyboard only when nothing else holds it: a click in the sidebar leaves it there.
    const active = document.activeElement;
    if (!active || active === document.body) composer?.focus();
    chatActions.focusComposer = () => composer?.focus();
    chatActions.prefill = (text) => {
      composer?.setText(text);
      composer?.focus();
    };
    chatActions.scrollToLatest = () => scrollToBottom(true);
    return () => {
      offEvents();
      observer.disconnect();
      chatActions.focusComposer = undefined;
      chatActions.prefill = undefined;
      chatActions.scrollToLatest = undefined;
    };
  });

  return (
    <div class="chat-view">
      <div class="transcript" ref={(el) => (scroller = el)} onScroll={onScroll} style={{ "padding-bottom": `${composerHeight()}px` }}>
        <div class="transcript-rows" ref={(el) => (content = el)} role="log" aria-label={L("Transcript")}>
          <Show when={chat()}>
            {(current) => (
              <For each={rows()} keyed={(row) => row.key}>
                {(row) => (
                  <div class="transcript-row" data-row={row().key} data-message={row().kind === "message" ? "1" : "0"}>
                    <RowView row={row()} chat={current()} />
                  </div>
                )}
              </For>
            )}
          </Show>
        </div>
      </div>
      <Show when={chat() && chat()!.messages.length === 0}>
        <div class="chat-empty-host" style={{ bottom: `${composerHeight()}px` }}>
          <ChatEmptyState
            chat={chat()!}
            bots={members()}
            onPick={(prompt) => {
              composer?.setText(prompt);
              composer?.focus();
            }}
          />
        </div>
      </Show>
      <Show when={showsJump()}>
        <button
          class="jump-button"
          style={{ bottom: `${composerHeight() + 14}px` }}
          title={L("Scroll to latest (%@)", shortcutText("CmdOrCtrl+J"))}
          aria-label={L("Scroll to latest")}
          onClick={() => scrollToBottom(true)}
        >
          <Icon name="arrow.down" size={13} strokeWidth={2.6} />
        </button>
      </Show>
      <Composer
        ref={(handle) => (composer = handle)}
        placeholder={placeholder()}
        bots={mentionable()}
        isResponding={isResponding()}
        onHeight={setComposerHeight}
        onStop={() => store.stopResponding(props.chatID)}
        onSend={(text, attachments, mentions) => {
          pinned = true;
          const destination = store.send(text, attachments, mentions, props.chatID);
          if (destination !== props.chatID) props.onRedirect(destination);
        }}
      />
    </div>
  );
}
