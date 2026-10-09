// A plugin's state in a word or two, as the desktop apps' rows word it. What it needs in full is in
// the plugin's sheet on a computer, or a named account's own screen here.

import type { PluginStatus } from "../core/model";
import { t } from "../i18n";

export function pluginStateWord(plugin: PluginStatus): string {
  switch (plugin.state) {
    case "ready":
      return plugin.account_name ? t("Connected") : t("Ready");
    case "needs_setup":
      return t("Needs setup");
    case "needs_auth":
      return t("Needs sign-in");
    case "insufficient_access":
      return t("Needs more access");
    case "connecting":
      return t("Connecting…");
    case "error":
      return plugin.account_name ? t("Can’t connect") : t("Error");
  }
}
