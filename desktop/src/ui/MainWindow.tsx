// The main window, after the macOS app's MainWindowController and RootSplitViewController: a toolbar
// (the sidebar and create buttons, back and forward in Settings, the title, the Device picker on the
// Device panes, the chat's running tasks, and the inspector's toggle) over a split of the sidebar,
// the content, and the inspector. It is the layout of the `/` routes; the content is theirs: a chat
// (`/chat/:id`), a settings pane (`/settings/:pane`), or with nothing selected, a state (`/`).

import { useLocation, useNavigate, type RouteSectionProps } from "@solidjs/router";
import { createEffect, createMemo, createSignal, onCleanup, onSettled, Show } from "solid-js";
import { frameStyle, host, hostInfo, onOpenChat, onUpdateAvailable, setPreferences, watchWindowState, windowControls, type WindowState } from "../host";
import { L, Lc } from "../l10n";
import { isDeviceScoped, paneTitle, type SettingsPane } from "../model/models";
import { onStoreEvent, track } from "../model/reactive";
import { store } from "../model/store";
import { addBotToChat, newBot, newGroupChat, presentMarketplace, presentPairing } from "./actions";
import { ChatView } from "./chat/ChatView";
import { RunningTasks } from "./chat/runningTasks";
import { commands, popupAppMenu, setGoToChat, shortcutText } from "./commands";
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
  togglePinChat,
  toggleInspector,
  toggleSidebar,
  userWantsInspector,
} from "./root";
import { SettingsPage } from "./settings/panes";
import { chatForShortcut, ChatsSidebar, SettingsSidebar } from "./sidebar";
import { Loading, Offline, Placeholder } from "./states";
import { offerUpdate, setupWindow } from "./window";

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
  const chatID = createMemo(() => {
    track.connection();
    track.chats();
    const current = selection.read();
    return current?.kind === "chat" && store.isConnected && store.chat(current.id) ? current.id : null;
  });
  const showsInspector = () => chatID() !== null && userWantsInspector.read();

  // The window's title: the chat's, the pane's, or the app's.
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
      document.title = text;
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

  // The window's buttons and, on Linux, whether its edges resize it.
  const [frame, setFrame] = createSignal<WindowState>({ focused: true, visible: true, minimized: false, maximized: false });
  onSettled(() => watchWindowState(setFrame));

  return (
    <div class={["main-window", `frame-${frameStyle()}`, `platform-${hostInfo().platform}`]}>
      <div class="split">
        <Show when={!sidebarCollapsed.read()}>
          <aside class="pane sidebar-pane" style={{ width: `${sidebarWidth.read()}px` }}>
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
          <ContentHeader chatID={chatID()} title={title().title} subtitle={title().subtitle} rightmost={!showsInspector()} />
          <div class="pane-body content-body">{props.children}</div>
        </main>
        <Show when={showsInspector() && chatID()}>
          {(id) => (
            <>
              <Divider edge="inspector" />
              <aside class="pane inspector-pane" style={{ width: `${inspectorWidth.read()}px` }}>
                <header class="pane-header inspector-header rightmost">
                  <span class="toolbar-spacer" />
                  <InspectorToggle />
                </header>
                <div class="pane-body">
                  <Inspector chatID={id()} />
                </div>
              </aside>
            </>
          )}
        </Show>
      </div>
      <Show when={frameStyle() === "custom"}>
        <WindowButtons state={frame()} />
      </Show>
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
    deleteChat: () => void deleteChat(),
    // The system's items, for a window whose page draws its title bar and answers their keys.
    closeWindow: () => void host.closeWindow(),
    quit: () => void host.quit(),
    fullScreen: () => void host.toggleFullScreen(),
  });
  // Ctrl+1 to Ctrl+9: the chat at that place in the sidebar, ready for a reply.
  setGoToChat((number) => {
    const id = chatForShortcut(number);
    if (id) open(id);
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
  void host.setTrayMenu(L("Open %@", hostInfo().name), L("Quit %@", hostInfo().name));

  // The relay turned this build away. Said once per launch, since every sync attempt gets the same
  // answer until Lorca is updated.
  let saidUpdateRequired = false;
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
    onUpdateAvailable((update) => void offerUpdate(update)),
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

/** A drag handle between two panes. The sidebar keeps 232 to 340 pixels, the inspector 268 to 320. */
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
      box.set(clamp(props.edge === "sidebar" ? from + delta : from - delta, low, high));
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
  return <div class={["divider", props.edge]} role="separator" aria-orientation="vertical" onMouseDown={start} />;
}

/** The buttons beside the traffic lights on macOS, or where a title bar's would be elsewhere: the
 * menu (on Windows and Linux, where the window has no menu bar), the sidebar, and Create. They sit
 * in the sidebar's header, or in the content's while the sidebar is collapsed. */
function LeadingButtons() {
  const isSettings = () => selection.read()?.kind === "settings";
  return (
    <div class="leading-buttons">
      <Show when={frameStyle() === "mac"}>
        <span class="traffic-light-space" />
      </Show>
      <Show when={frameStyle() === "custom"}>
        <HoverButton symbol="line.3.horizontal" tooltip={L("Menu")} onMouseDown={(event) => {
          if (event.button !== 0) return;
          event.preventDefault();
          void popupAppMenu(event.currentTarget as HTMLElement);
        }} />
      </Show>
      <HoverButton symbol="sidebar.leading" tooltip={L("Toggle Sidebar (%@)", shortcutText("CmdOrCtrl+B"))} onClick={toggleSidebar} />
      {/* Creating bots and chats belongs to the chats; Settings hides it. A press opens its menu. */}
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
      />
    </div>
  );
}

function InspectorToggle() {
  return <HoverButton symbol="sidebar.trailing" tooltip={L("Toggle Inspector (%@)", shortcutText("CmdOrCtrl+Shift+B"))} onClick={toggleInspector} />;
}

/** The content's header: back and forward in Settings, the title, the Device picker on the Device
 * panes, and the chat's running tasks. The inspector's toggle sits here while the inspector is
 * closed, and in the inspector's header while it is open. */
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

/** Minimize, maximize or restore, and close, drawn by the page at the window's top-right corner:
 * Windows 11's caption buttons, or round ones on Linux. They dim while the window is behind. */
function WindowButtons(props: { state: WindowState }) {
  return (
    <div class={["window-buttons", { inactive: !props.state.focused }]}>
      <button class="window-button" title={L("Minimize")} aria-label={L("Minimize")} onClick={() => void windowControls.minimize()}>
        <svg viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0 5.5h10" />
        </svg>
      </button>
      <button
        class="window-button"
        title={props.state.maximized ? Lc("Restore", "window") : L("Maximize")}
        aria-label={props.state.maximized ? Lc("Restore", "window") : L("Maximize")}
        onClick={() => void windowControls.toggleMaximize()}
      >
        <svg viewBox="0 0 10 10" aria-hidden="true">
          <Show when={props.state.maximized} fallback={<rect x="0.5" y="0.5" width="9" height="9" />}>
            <path d="M2.5 2.5V0.5h7v7h-2" />
            <rect x="0.5" y="2.5" width="7" height="7" />
          </Show>
        </svg>
      </button>
      <button class="window-button close" title={L("Close")} aria-label={L("Close")} onClick={() => void windowControls.close()}>
        <svg viewBox="0 0 10 10" aria-hidden="true">
          <path d="M0.5 0.5l9 9M9.5 0.5l-9 9" />
        </svg>
      </button>
    </div>
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
      options={devices().map((device) => ({ value: device.id, label: device.isThisDevice ? L("%@ (This computer)", device.name) : device.name }))}
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
