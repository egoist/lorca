// The sidebars, after the macOS app's: the chats (a search field that opens the command palette,
// a row per chat, and a footer with Settings, this computer, and the marketplace) and, while
// Settings is up, the settings panes (a search over the panes and their settings, and Back).

import { createMemo, createSignal, For, onSettled, Show } from "solid-js";
import { hostInfo, preferences } from "../host";
import { L } from "../l10n";
import * as Format from "../model/format";
import { deviceSymbol, isDM, isGroup, paneSymbol, paneTitle, settingsPanes, type SettingsPane } from "../model/models";
import { track } from "../model/reactive";
import { store } from "../model/store";
import { AvatarCluster, botAvatar, sameAvatar, type AvatarContent } from "./avatar";
import { commands, shortcutText } from "./commands";
import { HoverButton, SearchField } from "./controls";
import { Icon } from "./icons";
import { popupMenu, separator } from "./menu";
import {
  closeSettings,
  deleteChat,
  open,
  openDevice,
  renameChat,
  reveal,
  select,
  selection,
  settingsDeviceID,
  sidebarActions,
  togglePinChat,
} from "./root";
import { panesMatching, type SettingsEntry } from "./settings/search";

// MARK: - Chats

/** Everything a chat's row shows. A row redraws only when this changes. */
interface RowContent {
  avatars: AvatarContent[];
  isWorking: boolean;
  title: string;
  preview: string;
  stamp: string;
  isPinned: boolean;
  unreadCount: number;
  isGroup: boolean;
}

function sameContent(a: RowContent | undefined, b: RowContent | undefined): boolean {
  if (!a || !b) return a === b;
  return (
    a.isWorking === b.isWorking &&
    a.title === b.title &&
    a.preview === b.preview &&
    a.stamp === b.stamp &&
    a.isPinned === b.isPinned &&
    a.unreadCount === b.unreadCount &&
    a.isGroup === b.isGroup &&
    a.avatars.length === b.avatars.length &&
    a.avatars.every((avatar, index) => sameAvatar(avatar, b.avatars[index]!))
  );
}

function rowContent(chatID: string): RowContent | undefined {
  const chat = store.chat(chatID);
  if (!chat) return undefined;
  return {
    avatars: store.botsIn(chat).slice(0, 4).map(botAvatar),
    isWorking: chat.botIDs.some((id) => store.isWorking(id)),
    title: store.title(chat),
    preview: store.preview(chat),
    stamp: Format.stamp(chat.messages[chat.messages.length - 1]?.createdAt ?? chat.createdAt),
    isPinned: chat.isPinned,
    unreadCount: chat.unreadCount,
    isGroup: isGroup(chat),
  };
}

/** Ctrl (⌘ on a Mac) held alone for a moment shows each of the first nine rows' number in its
 * stamp; Ctrl+1 to Ctrl+9 open them. */
const [showsShortcuts, setShowsShortcuts] = createSignal(false);

function watchShortcutKey(): () => void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const hide = () => {
    clearTimeout(timer);
    timer = undefined;
    setShowsShortcuts(false);
  };
  const modifier = hostInfo().platform === "darwin" ? "Meta" : "Control";
  const down = (event: KeyboardEvent) => {
    if (event.key !== modifier || event.shiftKey || event.altKey || (modifier === "Control" ? event.metaKey : event.ctrlKey)) {
      hide();
      return;
    }
    if (timer === undefined) timer = setTimeout(() => setShowsShortcuts(true), 250);
  };
  window.addEventListener("keydown", down);
  window.addEventListener("keyup", hide);
  window.addEventListener("blur", hide);
  return () => {
    window.removeEventListener("keydown", down);
    window.removeEventListener("keyup", hide);
    window.removeEventListener("blur", hide);
  };
}

function ChatRow(props: { id: string; index: number }) {
  const content = createMemo(
    () => {
      track.chats();
      track.roster();
      return rowContent(props.id);
    },
    { equals: sameContent },
  );
  const selected = () => {
    const current = selection.read();
    return current?.kind === "chat" && current.id === props.id;
  };
  const shortcut = () => (showsShortcuts() && props.index < 9 ? shortcutText(`CmdOrCtrl+${props.index + 1}`) : null);
  const menu = async (event: MouseEvent) => {
    event.preventDefault();
    // Acting on the clicked row means selecting it first; the commands read the selection.
    select({ kind: "chat", id: props.id });
    const chat = store.chat(props.id);
    if (!chat) return;
    const picked = await popupMenu(
      [
        { id: "pin", label: chat.isPinned ? L("Unpin") : L("Pin") },
        ...(isGroup(chat) ? [{ id: "rename", label: L("Rename…") }, { id: "add", label: L("Add Bot…") }] : []),
        separator,
        { id: "delete", label: isDM(chat) ? L("Delete Bot") : L("Delete") },
      ],
      { x: event.clientX, y: event.clientY },
    );
    if (picked === "pin") togglePinChat();
    else if (picked === "rename") void renameChat();
    else if (picked === "add") commands.run("addBot");
    else if (picked === "delete") void deleteChat();
  };
  return (
    <Show when={content()}>
      {(row) => (
        <div
          class={["chat-row", { selected: selected() }]}
          role="option"
          aria-selected={selected() ? "true" : "false"}
          aria-label={[row().title, row().unreadCount > 0 ? L("%d unread", row().unreadCount) : null, row().isWorking ? L("Working") : null].filter(Boolean).join(", ")}
          data-chat={props.id}
          onMouseDown={(event) => {
            if (event.button === 0) select({ kind: "chat", id: props.id });
          }}
          onDblClick={() => {
            if (row().isGroup) void renameChat();
          }}
          onContextMenu={menu}
        >
          <AvatarCluster contents={row().avatars} slot={38} working={row().isWorking} />
          <div class="chat-row-text">
            <div class="chat-row-top">
              <span class="chat-row-title truncate">{row().title}</span>
              <Show when={row().isPinned}>
                <Icon name="pin.fill" size={10} class="chat-row-pin" label={L("Pinned")} />
              </Show>
              <span class="chat-row-stamp">{shortcut() ?? row().stamp}</span>
            </div>
            <div class="chat-row-bottom">
              <span class="chat-row-preview truncate">{row().preview}</span>
              <Show when={row().unreadCount > 0}>
                <span class="unread-pill" aria-label={L("%d unread", row().unreadCount)}>
                  {row().unreadCount > 999 ? "999+" : String(row().unreadCount)}
                </span>
              </Show>
            </div>
          </div>
        </div>
      )}
    </Show>
  );
}

function sameIDs(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((id, index) => id === b[index]);
}

/** The chat Ctrl-`number` opens: the row at that place in the list. */
export function chatForShortcut(number: number): string | null {
  if (number < 1 || number > 9) return null;
  return store.chats[number - 1]?.id ?? null;
}

export function ChatsSidebar() {
  const chatIDs = createMemo(
    () => {
      track.chats();
      return store.chats.map((chat) => chat.id);
    },
    { equals: sameIDs },
  );
  let list: HTMLDivElement | undefined;
  onSettled(() => watchShortcutKey());
  const move = (delta: number) => {
    const ids = chatIDs();
    const current = selection.get();
    const index = current?.kind === "chat" ? ids.indexOf(current.id) : -1;
    const next = ids[Math.min(ids.length - 1, Math.max(0, index + delta))];
    if (next) {
      select({ kind: "chat", id: next });
      list?.querySelector(`[data-chat="${CSS.escape(next)}"]`)?.scrollIntoView({ block: "nearest" });
    }
  };
  return (
    <div class="sidebar chats-sidebar">
      <div class="sidebar-search">
        <SearchField onActivate={() => commands.run("palette")} />
      </div>
      <div
        class="sidebar-list"
        role="listbox"
        tabindex={0}
        ref={(el) => (list = el)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            move(1);
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            move(-1);
          } else if (event.key === "Enter") {
            const current = selection.get();
            if (current?.kind === "chat") open(current.id);
          }
        }}
      >
        <For each={chatIDs()}>{(id, index) => <ChatRow id={id} index={index()} />}</For>
      </div>
      <SidebarFooter />
    </div>
  );
}

/** Settings, and this computer, whose icon turns red while the CLI is not answering and orange
 * while the CLI cannot reach the relay, with why in its tooltip; the marketplace at the other end. */
function SidebarFooter() {
  const state = createMemo(() => {
    track.connection();
    track.roster();
    const connected = store.isConnected;
    const device = store.thisDevice;
    const name = device?.name ?? L("This computer");
    const relayError = connected ? store.relayError : null;
    const status = !connected
      ? L("CLI not running · start it with: lorca serve").replace("lorca serve", hostInfo().cliCommand)
      : relayError
        ? L("Can’t connect to the relay: %@", relayError)
        : L("CLI on 127.0.0.1:%@", String(preferences().cliPort));
    const symbol = device ? deviceSymbol(device) : hostInfo().platform === "windows" ? "pc" : "laptopcomputer";
    return { name, status, symbol: symbol === "desktopcomputer" ? "display" : symbol, tint: !connected ? "var(--red)" : relayError ? "var(--orange)" : undefined };
  });
  return (
    <div class="sidebar-footer">
      <HoverButton symbol="gearshape" tooltip={L("Settings (%@)", shortcutText("CmdOrCtrl+,"))} onClick={() => select({ kind: "settings", pane: "general" })} />
      <HoverButton
        symbol={state().symbol}
        tooltip={`${state().name} · ${state().status}`}
        label={state().name}
        style={state().tint ? { color: state().tint } : undefined}
        onClick={() => {
          const id = store.thisDevice?.id;
          if (id) openDevice(id);
        }}
      />
      <div class="sidebar-footer-spacer" />
      <HoverButton symbol="circle.grid.2x2" tooltip={L("Marketplace (%@)", shortcutText("CmdOrCtrl+Shift+M"))} onClick={() => commands.run("marketplace")} />
    </div>
  );
}

// MARK: - Settings

type SettingsRow = { kind: "pane"; pane: SettingsPane } | { kind: "setting"; entry: SettingsEntry };

export function SettingsSidebar() {
  const [query, setQuery] = createSignal("");
  let list: HTMLDivElement | undefined;
  let field: HTMLInputElement | undefined;
  sidebarActions.focusSettingsSearch = () => field?.focus();
  sidebarActions.focusSettingsList = () => list?.focus();

  const rows = createMemo((): SettingsRow[] => {
    track.roster();
    const text = query().trim();
    if (text === "") return settingsPanes.map((pane) => ({ kind: "pane", pane }));
    const deviceID = settingsDeviceID.read();
    const device = deviceID ? store.device(deviceID) : undefined;
    return panesMatching(text, device, store).flatMap((result) => [
      { kind: "pane" as const, pane: result.pane },
      ...result.entries.map((entry) => ({ kind: "setting" as const, entry })),
    ]);
  });
  const selectedPane = () => {
    const current = selection.read();
    return current?.kind === "settings" ? current.pane : null;
  };
  const [picked, setPicked] = createSignal<SettingsEntry | null>(null);
  const pickRow = (row: SettingsRow) => {
    if (row.kind === "pane") {
      setPicked(null);
      select({ kind: "settings", pane: row.pane });
    } else {
      setPicked(row.entry);
      reveal(row.entry);
    }
  };
  /** Opens the best search result: the first matching setting, or else the first page listed. */
  const pickFirst = () => {
    if (query().trim() === "") return false;
    const all = rows();
    const first = all.find((row) => row.kind === "setting") ?? all[0];
    if (!first) return false;
    pickRow(first);
    return true;
  };
  const move = (delta: number) => {
    const panes = rows().filter((row): row is { kind: "pane"; pane: SettingsPane } => row.kind === "pane");
    const index = panes.findIndex((row) => row.pane === selectedPane());
    const next = panes[Math.min(panes.length - 1, Math.max(0, index + delta))];
    if (next) pickRow(next);
  };
  return (
    <div class="sidebar settings-sidebar">
      <div class="sidebar-search">
        <SearchField
          ref={(el) => (field = el)}
          value={query()}
          onInput={setQuery}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") {
              event.preventDefault();
              if (!pickFirst()) move(0);
              list?.focus();
            } else if (event.key === "Enter") {
              event.preventDefault();
              pickFirst();
            }
          }}
        />
      </div>
      <div
        class="sidebar-list"
        role="listbox"
        tabindex={0}
        ref={(el) => (list = el)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown") {
            event.preventDefault();
            move(1);
          } else if (event.key === "ArrowUp") {
            event.preventDefault();
            move(-1);
          }
        }}
      >
        <For each={rows()}>
          {(row) =>
            row.kind === "pane" ? (
              <div
                class={["pane-row", { selected: selectedPane() === row.pane && picked() === null }]}
                role="option"
                onMouseDown={(event) => {
                  if (event.button === 0) pickRow(row);
                }}
              >
                <span class="pane-row-icon">
                  <Icon name={paneSymbol(row.pane)} size={16} strokeWidth={1.7} />
                </span>
                <span class="truncate">{paneTitle(row.pane)}</span>
              </div>
            ) : (
              <div
                class={["setting-row", { selected: picked() === row.entry }]}
                role="option"
                title={row.entry.title}
                onMouseDown={(event) => {
                  if (event.button === 0) pickRow(row);
                }}
              >
                <span class="truncate">{row.entry.title}</span>
              </div>
            )
          }
        </For>
        <Show when={rows().length === 0}>
          <div class="sidebar-no-results">{L("No Results for “%@”", query().trim())}</div>
        </Show>
      </div>
      <div class="sidebar-footer">
        <HoverButton symbol="chevron.left" size={13} title={L("Back")} tooltip={L("Back to Chats (esc)")} label={L("Back to Chats")} onClick={closeSettings} />
      </div>
    </div>
  );
}
