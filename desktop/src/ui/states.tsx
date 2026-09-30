// The content area's states besides a chat and Settings, after the macOS app's
// StateViewControllers: loading while the CLI starts, the recovery page while it is not answering,
// and the placeholder when nothing is selected.

import { hostInfo, preferences } from "../host";
import { L } from "../l10n";
import { track } from "../model/reactive";
import { store } from "../model/store";
import { newBot } from "./actions";
import { Button, CopyButton, Spinner } from "./controls";
import { Icon } from "./icons";

/** The first window is ready while the CLI starts and loads the account. */
export function Loading() {
  return (
    <div class="state-page">
      <div class="state-column">
        <Spinner size={22} />
        <div class="state-caption">{L("Loading…")}</div>
      </div>
    </div>
  );
}

/** Shown when the local CLI is not answering on 127.0.0.1. */
export function Offline() {
  const command = hostInfo().cliCommand;
  const hint = () => {
    track.connection();
    return `${store.offlineStatus} · 127.0.0.1:${preferences().cliPort}`;
  };
  return (
    <div
      class="state-page"
      onKeyDown={(event) => {
        if (event.key === "Enter" && (event.target as HTMLElement).tagName !== "BUTTON") store.reconnect();
      }}
    >
      <div class="state-column offline">
        <span class="state-icon">
          <Icon name="bolt.horizontal.circle" size={44} strokeWidth={1.4} />
        </span>
        <div class="state-title large">{L("The Lorca CLI isn't answering")}</div>
        <div class="state-body">
          {L(
            "Your bots, keys and transcripts live in the CLI on this computer. The app starts it on its own; you can also run it from a terminal, and this window reconnects either way.",
          )}
        </div>
        <div class="state-command">
          <span class="selectable mono">{`$ ${command}`}</span>
          <CopyButton text={() => command} symbol="doc.on.doc" tooltip={L("Copy command")} />
        </div>
        <Button kind="primary" large onClick={() => store.reconnect()}>
          {L("Retry Connection")}
        </Button>
        <div class="state-hint selectable">{hint()}</div>
      </div>
    </div>
  );
}

/** Shown when nothing is selected in the sidebar. */
export function Placeholder() {
  return (
    <div class="state-page">
      <div class="state-column">
        <span class="state-icon">
          <Icon name="bubble.left.and.bubble.right" size={38} strokeWidth={1.4} />
        </span>
        <div class="state-title">{L("No chat selected")}</div>
        <div class="state-body">{L("Pick a conversation in the sidebar, or start a new one.")}</div>
        <Button large onClick={newBot}>
          {L("New Bot…")}
        </Button>
      </div>
    </div>
  );
}
