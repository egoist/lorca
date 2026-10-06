// The main window, after the macOS app's MainWindowController and RootSplitViewController: a toolbar
// (the sidebar and create buttons, back and forward in Settings, the title, the Device picker on the
// Device panes, the chat's running tasks, and the inspector's toggle) over a split of the sidebar,
// the content, and the inspector. It is the layout of the `/` routes; the content is theirs: a chat
// (`/chat/:id`), a settings pane (`/settings/:pane`), or with nothing selected, a state (`/`).

import { useLocation, useNavigate, type RouteSectionProps } from "@solidjs/router";
import { createEffect, createMemo, createSignal, onCleanup, onSettled, Show } from "solid-js";
import { frameStyle, host, hostInfo, onOpenChat, setPreferences, watchWindowState } from "../host";
import { L } from "../l10n";
import { deviceSymbol, isDeviceScoped, paneTitle, type SettingsPane } from "../model/models";
import { onStoreEvent, track } from "../model/reactive";
import { store } from "../model/store";
import { addBotToChat, newBot, newGroupChat, presentMarketplace, presentPairing } from "./actions";
import { ChatView } from "./chat/ChatView";
import { RunningTasks } from "./chat/runningTasks";
import { commands, setGoToChat, shortcutText } from "./commands";
import { HoverButton, PopUpButton } from "./controls";
import { Inspector } from "./inspector";
import { popupMenu, separator } from "./menu";
import { Notifier } from "./notifier";
import { alert, closePopover, hasSheet, isPopoverOpen, showPopover } from "./overlay";
import { palette, PaletteHost } from "./palette";
import {
  attachNavigator,
  canGoBack,
  canGoForward,
  chatActions,
  clamp,
  closeSettings,
  deleteChat,
  followRoute,
  followStore,
  goBack,
  goForward,
  inspectorWidth,
  open,
  paneLayout,
  renameChat,
  restoreSelection,
  select,
  selectedChatID,
  selectionAt,
  selection,
  settingsDeviceID,
  showSettings,
  showSettingsDevice,
  sidebarActions,
  sidebarCollapsed,
  sidebarWidth,
  squeezesContent,
  togglePinChat,
  toggleInspector,
  toggleSidebar,
  userWantsInspector,
} from "./root";
import { SettingsPage } from "./settings/panes";
import { chatForShortcut, ChatsSidebar, SettingsSidebar } from "./sidebar";
import { Loading, Offline, Placeholder } from "./states";
import { setupWindow, setWindowTitle } from "./window";

/** The narrowest the content gets before the side panes give way. */
const contentMinWidth = 460;

/** The side panes' widths in a window `available` wide, each with its divider's pixel, as they give
 * way to the content down to their own minimums, the inspector first; `fits` is whether the
 * content keeps its own. */
function fitPanes(available: number, sidebar: number, inspector: number) {
  let short = contentMinWidth - (available - sidebar - inspector);
  const giveWay = (width: number, low: number) => {
    const taken = Math.max(0, Math.min(short, width - low));
    short -= taken;
    return width - taken;
  };
  if (inspector) inspector = giveWay(inspector, 268 + 1);
  if (sidebar) sidebar = giveWay(sidebar, 232 + 1);
  return { sidebar, inspector, fits: short <= 0 };
}

/** The relay turned this build away. Said once per launch, since every sync attempt gets the same
 * answer until Lorca is updated. */
let saidUpdateRequired = false;

export function MainWindow(props: RouteSectionProps) {
  const navigate = useNavigate();
  const location = useLocation();
  // Selecting replaces the route, keeping its query (`?mock=1` in a browser tab).
  attachNavigator((path) => navigate(`${path}${window.location.search}`, { replace: true, scroll: false }));
  // The selection follows the route; with none to show, it goes back to where the window was.
  createEffect(
    () => location.pathname,
    (pathname) => followRoute(pathname),
  );

  const isSettings = () => selection.read()?.kind === "settings";
  /** A chat has been on screen since the window opened. */
  let shownChat = false;
  const chatID = createMemo(() => {
    track.connection();
    track.chats();
    const current = selection.read();
    const id = current?.kind === "chat" && store.isConnected && store.chat(current.id) ? current.id : null;
    if (id) shownChat = true;
    return id;
  });
  /** Until a chat has shown (loading, or a first start that failed), the inspector keeps its saved
   * width for the chat the window is about to show, so the transcript never widens and then
   * narrows. */
  const holdsInspector = () => {
    track.connection();
    return !shownChat && !store.isConnected && selection.read()?.kind === "chat";
  };
  const showsInspector = () => userWantsInspector.read() && (chatID() !== null || holdsInspector());

  // As the window narrows, the side panes give way to the content down to their own minimums, the
  // inspector first; past that the inspector steps aside until there is room again, as AppKit's
  // split view collapses it.
  const [windowWidth, setWindowWidth] = createSignal(window.innerWidth);
  /** Side panes waiting on the window to widen for them: they open as the page takes the width. */
  let waiting: (() => void)[] = [];
  onSettled(() => {
    let width = window.innerWidth;
    const onResize = () => {
      if (window.innerWidth < width) squeezesContent.set(false);
      width = window.innerWidth;
      setWindowWidth(width);
      const opens = waiting;
      waiting = [];
      for (const open of opens) open();
    };
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  });
  const panes = createMemo(() => {
    const sidebar = sidebarCollapsed.read() ? 0 : sidebarWidth.read() + 1;
    const inspector = showsInspector() ? inspectorWidth.read() + 1 : 0;
    let layout = fitPanes(windowWidth(), sidebar, inspector);
    // With the inspector aside, the sidebar has its own width back where there is room.
    if (!layout.fits && inspector && !squeezesContent.read()) layout = fitPanes(windowWidth(), sidebar, 0);
    return { sidebar: Math.max(0, layout.sidebar - 1), inspector: Math.max(0, layout.inspector - 1) };
  });
  paneLayout.showsInspector = () => panes().inspector > 0;
  // A side pane the user opens where there is no room widens the window by the pane, so the content
  // keeps its width, as AppKit's split view does. The pane opens as the page takes the new width,
  // in one layout, so nothing on screen moves twice. Where the window cannot widen enough
  // (maximized, full screen, the screen's edges), the content narrows past its minimum instead.
  paneLayout.open = (pane, show) => {
    const sidebar = pane === "sidebar" || panes().sidebar > 0 ? sidebarWidth.get() + 1 : 0;
    const inspector = pane === "inspector" || panes().inspector > 0 ? inspectorWidth.get() + 1 : 0;
    if (fitPanes(windowWidth(), sidebar, inspector).fits) {
      show();
      return;
    }
    const width = pane === "sidebar" ? sidebar : inspector;
    void host
      .windowRoom()
      .catch(() => 0)
      .then((room) => {
        const by = Math.min(width, room);
        const open = () => {
          if (by < width - 1) squeezesContent.set(true);
          show();
        };
        if (by < 1) {
          open();
          return;
        }
        waiting.push(open);
        // A window manager can turn the new size down; the pane opens anyway, the content narrowing.
        setTimeout(() => {
          if (!waiting.includes(open)) return;
          waiting = waiting.filter((each) => each !== open);
          squeezesContent.set(true);
          show();
        }, 500);
        void host.widenWindow(by);
      });
  };
  onCleanup(() => {
    paneLayout.showsInspector = undefined;
    paneLayout.open = undefined;
  });

  // The title: the chat's, the pane's, or the app's; the window's adds " - Lorca".
  const title = createMemo(() => {
    track.chats();
    const current = selection.read();
    if (current?.kind === "chat") {
      const chat = store.chat(current.id);
      if (chat) return { title: store.title(chat), subtitle: store.subtitle(chat) };
    }
    if (current?.kind === "settings") return { title: paneTitle(current.pane), subtitle: "" };
    return { title: hostInfo().name, subtitle: "" };
  });
  createEffect(
    () => title().title,
    (text) => {
      setWindowTitle(text);
    },
  );

  // The menu bar and the commands of this window; the menu bar keeps its items in step through a
  // memo and an effect, so it is set up here rather than once the window settles.
  onCleanup(setupWindow("main"));
  // Notifications, and the CLI's `ui.watching`, follow the chat on screen.
  const notifier = new Notifier();
  createEffect(
    () => selection.read(),
    () => notifier.watchingChanged(),
  );
  onSettled(() => {
    // A route that names a chat or a pane keeps it; the snapshot checks a chat once it lands.
    if (selectionAt(location.pathname) === null) restoreSelection();
    return startServices(notifier);
  });

  return (
    <div class={["main-window", `frame-${frameStyle()}`]}>
      <div class="split">
        <Show when={!sidebarCollapsed.read()}>
          <aside class="pane sidebar-pane" style={{ width: `${panes().sidebar}px` }}>
            <header class="pane-header sidebar-header">
              <LeadingButtons />
            </header>
            <div class="pane-body">
              <Show when={isSettings()} fallback={<ChatsSidebar />}>
                <SettingsSidebar />
              </Show>
            </div>
          </aside>
          <Divider edge="sidebar" />
        </Show>
        <main class="pane content-pane">
          <ContentHeader chatID={chatID()} title={title().title} subtitle={title().subtitle} rightmost={panes().inspector === 0} />
          <div class="pane-body content-body">{props.children}</div>
        </main>
        <Show when={panes().inspector > 0}>
          <Divider edge="inspector" />
          <aside class="pane inspector-pane" style={{ width: `${panes().inspector}px` }}>
            <header class="pane-header inspector-header">
              <span class="toolbar-spacer" />
              <InspectorToggle />
            </header>
            <div class="pane-body">
              <Show when={chatID()}>{(id) => <Inspector chatID={id()} />}</Show>
            </div>
          </aside>
        </Show>
      </div>
      <PaletteHost />
    </div>
  );
}

// MARK: - Routes

/** While the CLI is not answering: loading on the first connection, then the recovery page. */
function ConnectionState() {
  const starting = () => {
    track.connection();
    return store.isStarting;
  };
  return (
    <Show when={starting()} fallback={<Offline />}>
      <Loading />
    </Show>
  );
}

const connected = () => {
  track.connection();
  return store.isConnected;
};

/** `/`: nothing selected. */
export function HomeRoute() {
  return (
    <Show when={connected()} fallback={<ConnectionState />}>
      <Placeholder />
    </Show>
  );
}

/** `/chat/:id`: the chat, which the window keeps as the id changes. */
export function ChatRoute(props: RouteSectionProps) {
  const id = () => props.params.id ?? "";
  const exists = () => {
    track.chats();
    return store.chat(id()) !== undefined;
  };
  return (
    <Show when={connected()} fallback={<ConnectionState />}>
      <Show when={exists()} fallback={<Placeholder />}>
        <ChatView chatID={id()} onRedirect={(next) => select({ kind: "chat", id: next })} />
      </Show>
    </Show>
  );
}

/** `/settings/:pane`: the panes show while the CLI is not answering, since the relay URL and the CLI
 * port live there. */
export function SettingsRoute(props: RouteSectionProps) {
  return <SettingsPage pane={(props.params.pane ?? "general") as SettingsPane} />;
}

// MARK: - Services

/** What the main window runs while it is open: its commands, notifications, the unread badge, and
 * the answers to the Go side's events. */
function startServices(notifier: Notifier): () => void {
  const stops: (() => void)[] = [];
  commands.register({
    newBot,
    newGroupChat,
    marketplace: () => presentMarketplace(),
    pairDevice: presentPairing,
    settings: () => showSettings(),
    find: () => (selection.get()?.kind === "settings" ? sidebarActions.focusSettingsSearch?.() : palette.show()),
    palette: () => palette.toggle(),
    toggleSidebar,
    toggleInspector,
    scrollToLatest: () => chatActions.scrollToLatest?.(),
    addBot: () => addBotToChat(),
    renameChat: () => void renameChat(),
    pinChat: togglePinChat,
    stopResponding: () => {
      const id = selectedChatID();
      if (id) store.stopResponding(id);
    },
    // Every command in the chat that a bot's call still waits on.
    runInBackground: () => {
      const id = selectedChatID();
      if (!id) return;
      for (const message of store.foregroundCommands(id)) void store.sendCommandToBackground(id, message.id).catch(() => {});
    },
    deleteChat: () => void deleteChat(),
  });
  // Ctrl+1 to Ctrl+9: the chat at that place in the sidebar, ready for a reply, its row in view.
  setGoToChat((number) => {
    const id = chatForShortcut(number);
    if (!id) return;
    open(id);
    document.querySelector(`.chat-row[data-chat="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "nearest" });
  });
  stops.push(followStore());

  // Notifications start once the first connection settles.
  const startNotifier = () => {
    if (!store.isStarting) notifier.start();
  };
  startNotifier();
  stops.push(() => notifier.stop());

  // The unread count on the taskbar or launcher, and the tray menu's words.
  const updateBadge = () => {
    const count = store.hasIdentity === false ? 0 : store.chats.reduce((sum, chat) => sum + chat.unreadCount, 0);
    void host.setBadge(count);
  };
  updateBadge();

  const relayStateChanged = () => {
    if (!store.relayUpdateRequired || saidUpdateRequired) return;
    saidUpdateRequired = true;
    const updates = hostInfo().updatesEnabled;
    void alert({
      message: L("Update Lorca to keep syncing"),
      informative: L("The relay no longer works with this version. Chats on this computer stay as they are, and nothing syncs with your other Devices until you update."),
      buttons: updates ? [{ title: L("Check for Updates…") }, { title: L("Later") }] : [{ title: L("OK") }],
    }).then((answer) => {
      if (updates && answer === 0) commands.run("checkForUpdates");
    });
  };
  relayStateChanged();

  stops.push(
    onStoreEvent((event) => {
      switch (event.kind) {
        case "identityChanged":
        case "snapshotReplaced":
        case "chatsChanged":
          updateBadge();
          break;
        case "rosterChanged":
          relayStateChanged();
          break;
      }
      startNotifier();
    }),
    onOpenChat((id) => open(id)),
    // Coming to the front marks the chat on screen read.
    watchWindowState((state) => {
      const id = selectedChatID();
      if (state.focused && id) store.markRead(id);
    }),
  );

  // Escape leaves Settings when nothing in front of the window takes it.
  const onKey = (event: KeyboardEvent) => {
    if (event.key !== "Escape" || event.defaultPrevented || event.isComposing) return;
    if (selection.get()?.kind !== "settings" || hasSheet() || isPopoverOpen() || palette.isOpen()) return;
    event.preventDefault();
    closeSettings();
  };
  window.addEventListener("keydown", onKey);
  stops.push(() => window.removeEventListener("keydown", onKey));

  return () => {
    attachNavigator(null);
    for (const stop of stops) stop();
  };
}

// MARK: - Layout

/** A drag handle between two panes. The sidebar keeps 232 to 340 pixels, the inspector 268 to 320;
 * dragging the sidebar's well past its narrowest collapses it, as a split view's collapsible pane. */
function Divider(props: { edge: "sidebar" | "inspector" }) {
  const start = (event: MouseEvent) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const box = props.edge === "sidebar" ? sidebarWidth : inspectorWidth;
    const [low, high] = props.edge === "sidebar" ? [232, 340] : [268, 320];
    const origin = event.clientX;
    const from = box.get();
    document.body.classList.add("resizing");
    const move = (next: MouseEvent) => {
      const delta = next.clientX - origin;
      const wanted = props.edge === "sidebar" ? from + delta : from - delta;
      if (props.edge === "sidebar" && wanted < low / 2) {
        end();
        toggleSidebar();
        return;
      }
      box.set(clamp(wanted, low, high));
    };
    const end = () => {
      window.removeEventListener("mousemove", move);
      window.removeEventListener("mouseup", end);
      document.body.classList.remove("resizing");
      void setPreferences(props.edge === "sidebar" ? { sidebarWidth: box.get() } : { inspectorWidth: box.get() });
    };
    window.addEventListener("mousemove", move);
    window.addEventListener("mouseup", end);
  };
  // Not the edge's own class: `.inspector` scrolls, which would clip the handle to its pixel.
  return <div class={["divider", `${props.edge}-divider`]} role="separator" aria-orientation="vertical" onMouseDown={start} />;
}

/** The sidebar and Create buttons, after the traffic lights on macOS, or the window controls on
 * Linux when the desktop puts them at the left. They sit in the sidebar's header, or in the
 * content's while the sidebar is collapsed. */
function LeadingButtons() {
  const isSettings = () => selection.read()?.kind === "settings";
  return (
    <div class="leading-buttons">
      <Show when={frameStyle() === "mac"}>
        <span class="traffic-light-space" />
      </Show>
      <HoverButton symbol="sidebar.leading" tooltip={L("Toggle Sidebar (%@)", shortcutText("CmdOrCtrl+B"))} onClick={toggleSidebar} />
      {/* Creating bots and chats belongs to the chats; Settings hides it. A press opens its menu;
          the keyboard's press is the New Bot action, as the Mac button's own action is. */}
      <HoverButton
        symbol="plus"
        tooltip={L("Create")}
        hidden={isSettings()}
        onMouseDown={(event) => {
          if (event.button !== 0) return;
          event.preventDefault();
          void popupMenu(
            [{ id: "newBot", label: L("Create New Bot…") }, { id: "newGroupChat", label: L("Create Group Chat…") }, separator, { id: "pairDevice", label: L("Pair a Device…") }],
            event.currentTarget as HTMLElement,
          ).then((picked) => {
            if (picked) commands.run(picked);
          });
        }}
        onClick={(event) => {
          if (event.detail === 0) commands.run("newBot");
        }}
      />
    </div>
  );
}

function InspectorToggle() {
  return <HoverButton symbol="sidebar.trailing" tooltip={L("Toggle Inspector (%@)", shortcutText("CmdOrCtrl+Shift+B"))} onClick={toggleInspector} />;
}

/** The content's header: back and forward in Settings, the title, the Device picker on the Device
 * panes, and the chat's running tasks. The inspector's toggle sits here while the inspector is
 * closed, and in the inspector's header while it is open; the rightmost header keeps clear of the
 * window controls on Windows and Linux. */
function ContentHeader(props: { chatID: string | null; title: string; subtitle: string; rightmost: boolean }) {
  const pane = () => {
    const current = selection.read();
    return current?.kind === "settings" ? current.pane : null;
  };
  const isSettings = () => pane() !== null;
  return (
    <header class={["pane-header", "content-header", { rightmost: props.rightmost }]}>
      <Show when={sidebarCollapsed.read()}>
        <LeadingButtons />
      </Show>
      <Show when={isSettings()}>
        <div class="toolbar-navigation">
          <HoverButton symbol="chevron.left" tooltip={L("Back")} disabled={!canGoBack()} onClick={goBack} />
          <HoverButton symbol="chevron.right" tooltip={L("Forward")} disabled={!canGoForward()} onClick={goForward} />
        </div>
      </Show>
      <div class="toolbar-title">
        <span class="toolbar-title-text truncate">{props.title}</span>
        <Show when={props.subtitle !== ""}>
          <span class="toolbar-subtitle truncate">{props.subtitle}</span>
        </Show>
      </div>
      <span class="toolbar-spacer" />
      <Show when={pane() !== null && isDeviceScoped(pane()!)}>
        <DevicePicker />
      </Show>
      <Show when={!isSettings()}>
        <RunningTasksButton chatID={props.chatID} />
        <Show when={props.rightmost}>
          <InspectorToggle />
        </Show>
      </Show>
    </header>
  );
}

/** The Device the Plugins, Bots, and Devices panes show; the pick holds across them. Pairing is a
 * command, so the pop-up goes on showing the picked Device. */
function DevicePicker() {
  const devices = () => {
    track.roster();
    return store.devices;
  };
  return (
    <PopUpButton
      style="plain"
      class="device-picker"
      label={L("Device")}
      tooltip={L("The Device this page shows")}
      options={devices().map((device) => ({ value: device.id, label: device.isThisDevice ? L("%@ (This computer)", device.name) : device.name, symbol: deviceSymbol(device) }))}
      value={settingsDeviceID.read() ?? ""}
      onChange={(id) => showSettingsDevice(id)}
      extras={[{ id: "pairDevice", label: L("Pair a Device…") }]}
      onExtra={(id) => commands.run(id)}
    />
  );
}

/** The chat's running commands: shown while it has any, or while their popover is open, with how
 * many as a badge once there is more than one. */
function RunningTasksButton(props: { chatID: string | null }) {
  const count = createMemo(() => {
    const id = props.chatID;
    if (!id) return 0;
    track.chat(id);
    return store.runningCommands(id).length;
  });
  const [open, setOpen] = createSignal<string | null>(null);
  let button: HTMLButtonElement | undefined;
  /** When the popover last closed: a click on the button closes it before the button acts. */
  let closedAt = 0;
  const toggle = () => {
    if (open() !== null) {
      closePopover();
      return;
    }
    const id = props.chatID;
    if (Date.now() - closedAt < 300 || !id || !button || count() === 0) return;
    setOpen(id);
    showPopover(button, (close) => <RunningTasks chatID={id} onEmpty={close} onClose={close} />, {
      edge: "below",
      class: "running-tasks-popover",
      onClose: () => {
        closedAt = Date.now();
        setOpen(null);
      },
    });
  };
  // The popover belongs to the chat on screen.
  createEffect(
    () => props.chatID,
    (id) => {
      if (open() !== null && open() !== id) closePopover();
    },
  );
  onSettled(() => () => {
    if (open() !== null) closePopover();
  });
  return (
    <span class="toolbar-tasks" hidden={count() === 0 && open() === null}>
      <HoverButton ref={(element) => (button = element)} symbol="terminal" tooltip={L("Running tasks")} label={L("Running tasks (%d)", count())} active={open() !== null} onClick={toggle} />
      <Show when={count() > 1}>
        <span class="toolbar-badge">{count()}</span>
      </Show>
    </span>
  );
}
