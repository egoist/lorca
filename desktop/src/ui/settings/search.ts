// One searchable setting, after the macOS app's SettingsSearch. The panes label their rows from
// these, so the search index and the pages share one spelling.

import { hostInfo } from "../../host";
import { L } from "../../l10n";
import { paneTitle, providerName, providerSubtitle, settingsPanes, type Bot, type Device, type InstalledPlugin, type ProviderKind, type SettingsPane } from "../../model/models";
import type { AppStore } from "../../model/store";

export interface SettingsEntry {
  pane: SettingsPane;
  /** What the results list shows. */
  title: string;
  /** The row's label on the pane, or a section's title. The pane finds the row by it. */
  row: string;
  /** Words that find the setting besides its title. */
  keywords: string[];
}

function entry(pane: SettingsPane, title: string, options: { row?: string; keywords?: string[] } = {}): SettingsEntry {
  return { pane, title, row: options.row ?? title, keywords: options.keywords ?? [] };
}

export const Entries = {
  sendOnReturn: () => entry("general", L("Return sends the message"), { keywords: [L("enter send newline keyboard chats")] }),
  timestamps: () => entry("general", L("Show timestamps in transcripts"), { keywords: [L("time date messages chats")] }),
  appearance: () => entry("general", L("Appearance"), { keywords: [L("theme dark mode light mode system")] }),
  appLanguage: () => entry("general", L("App Language"), { keywords: [L("language locale english chinese translation")] }),
  version: () => entry("general", L("Check for Updates"), { row: L("Version"), keywords: [L("update upgrade release")] }),
  automaticChecks: () => entry("general", L("Check for updates automatically"), { keywords: [L("update upgrade background")] }),
  automaticDownloads: () => entry("general", L("Download and install updates automatically"), { keywords: [L("update upgrade background")] }),
  autoReviewSwitch: () => entry("auto-review", L("Check actions before they run"), { keywords: [L("auto-review approve approval permission plugin ask")] }),
  autoReviewRules: () => entry("auto-review", L("Auto-review Rules"), { keywords: [L("rule allow automatically ask first always allow")] }),
  relayURL: () => entry("advanced", L("Relay URL"), { keywords: [L("server self-host sync pairing connection")] }),
  cliPort: () => entry("advanced", L("CLI port"), { keywords: [L("localhost 127.0.0.1 serve connection")] }),
  onboarding: () => entry("advanced", L("Onboarding"), { keywords: [L("show onboarding again setup welcome restore")] }),
  deleteAccount: () => entry("advanced", L("Delete Account"), { row: L("Account"), keywords: [L("erase remove wipe relay data identity")] }),
  machineKey: () => entry("device", L("Machine key"), { keywords: [L("device os role runner last seen relay")] }),
  pairing: () => entry("device", L("Pairing"), { keywords: [L("unpair remove device paired")] }),
  bot: (bot: Bot) => entry("bots", bot.name, { keywords: [bot.description, providerName(bot.provider), L("bot runner")] }),
  plugin: (plugin: InstalledPlugin) => entry("plugins", plugin.name, { keywords: [plugin.description, L("plugin mcp marketplace")] }),
  provider: (kind: ProviderKind) => entry("providers", providerName(kind), { keywords: [providerSubtitle(kind), L("credential connect disconnect sign in model")] }),
};

/** Whether `text` holds `query` the way a search field matches: ignoring case and accents. */
export function standardContains(text: string, query: string): boolean {
  const fold = (value: string) => value.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLocaleLowerCase();
  return fold(text).includes(fold(query));
}

export function matches(setting: SettingsEntry, query: string): boolean {
  return [setting.title, ...setting.keywords].some((text) => standardContains(text, query));
}

export function entriesIn(pane: SettingsPane, device: Device | undefined, store: AppStore): SettingsEntry[] {
  switch (pane) {
    case "general":
      return [Entries.sendOnReturn(), Entries.timestamps(), Entries.appearance(), Entries.appLanguage()].concat(
        hostInfo().updatesEnabled ? [Entries.version(), Entries.automaticChecks(), Entries.automaticDownloads()] : [],
      );
    case "auto-review":
      return [Entries.autoReviewSwitch(), Entries.autoReviewRules()];
    case "advanced":
      return [Entries.relayURL(), Entries.cliPort(), Entries.onboarding()].concat(store.hasIdentity === true ? [Entries.deleteAccount()] : []);
    case "bots":
      return (device ? store.botsOn(device.id) : []).map(Entries.bot);
    case "providers":
      return store.providers.map((credential) => Entries.provider(credential.kind));
    case "plugins":
      return (device?.plugins ?? []).map(Entries.plugin);
    case "device":
      return device && !device.isThisDevice ? [Entries.machineKey(), Entries.pairing()] : [Entries.machineKey()];
  }
}

/** Each pane that matches by name or holds a matching setting, and those settings under it. The
 * Device panes answer for the picked Device. */
export function panesMatching(query: string, device: Device | undefined, store: AppStore): { pane: SettingsPane; entries: SettingsEntry[] }[] {
  return settingsPanes.flatMap((pane) => {
    const entries = entriesIn(pane, device, store).filter((setting) => matches(setting, query));
    if (entries.length === 0 && !standardContains(paneTitle(pane), query)) return [];
    return [{ pane, entries }];
  });
}
