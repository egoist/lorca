// What every window of the app sets up, after the macOS app's AppDelegate actions: its menu bar and
// the commands any window answers (Help, Architecture Notes, About, Check for Updates, and the Debug
// menu's), and the update offer.

import { host, hostInfo, preferences, type UpdateInfo } from "../host";
import { L } from "../l10n";
import { errorText, store } from "../model/store";
import { commands, installMenuBar } from "./commands";
import { alert } from "./overlay";

function presentNote(title: string, body: string): void {
  void alert({ message: title, informative: body, style: "informational", buttons: [{ title: L("OK") }], width: 420 });
}

export function showHelp(): void {
  presentNote(
    L("Lorca runs on Devices you own"),
    L(
      "Every bot is assigned to a Runner: a Device running macOS, Linux, or Windows. That machine's CLI runs the turn with your account's provider credentials, so a bot on an offline Runner waits until it reconnects. Phones and tablets pair as Devices but never run bots.\n\nThe app talks only to the local CLI on 127.0.0.1:%@. Start it with `lorca serve`; the CLI holds your keys and provider credentials, which reach your other Devices encrypted.",
      String(preferences().cliPort),
    ).replace("lorca serve", hostInfo().cliCommand),
  );
}

export function showArchitecture(): void {
  presentNote(
    L("Three processes"),
    L(
      "The app talks only to the local CLI over a localhost websocket. The CLI holds the keys, runs the agent loop, and syncs ciphertext with the relay. The relay stores public keys and opaque blobs.\n\nFull notes live in ARCHITECTURE.md.",
    ),
  );
}

export function showAbout(): void {
  void alert({ message: hostInfo().name, informative: L("Version %@", hostInfo().version), buttons: [{ title: L("OK") }] });
}

/** Offers a newer version: install it now, which relaunches the app, or later. */
export async function offerUpdate(update: UpdateInfo): Promise<void> {
  const name = hostInfo().name;
  const answer = await alert({
    message: L("A new version of %@ is available!", name),
    informative: [L("%@ %@ is now available—you have %@. Would you like to install it now?", name, update.version, hostInfo().version), update.notes.trim()]
      .filter((part) => part !== "")
      .join("\n\n"),
    buttons: [{ title: L("Install Update") }, { title: L("Remind Me Later") }],
    width: 440,
  });
  if (answer !== 0) return;
  try {
    await host.installUpdate();
  } catch (error) {
    void alert({ message: L("Update Error!"), informative: errorText(error) });
  }
}

export async function checkForUpdates(): Promise<void> {
  try {
    const found = await host.checkForUpdates();
    if (found) await offerUpdate(found);
    else void alert({ message: L("You’re up to date!"), informative: L("%@ %@ is currently the newest version available.", hostInfo().name, hostInfo().version) });
  } catch (error) {
    void alert({ message: L("Update Error!"), informative: errorText(error) });
  }
}

/** The menu bar and the commands any window answers. The main window adds its own. */
export function setupWindow(kind: "main" | "other"): () => void {
  commands.register({
    help: showHelp,
    architecture: showArchitecture,
    about: showAbout,
    checkForUpdates: () => void checkForUpdates(),
    simulateOffline: () => (store.isMock ? store.setConnected(!store.isConnected) : store.reconnect()),
    replayMock: () => store.resetMockData(),
    showOnboarding: () => void host.showOnboarding(),
  });
  return installMenuBar(kind);
}
