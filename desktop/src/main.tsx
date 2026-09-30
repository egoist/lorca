// The page of every window: the build and preferences from the Go side, the language and appearance,
// the store's connection to the CLI, then the window's route.

import "./ui/theme.css";
import "./ui/components.css";
import "./ui/layout.css";
import { render } from "@solidjs/web";
import { hostInfo, loadHost, onPreferencesChanged, preferences } from "./host";
import { setLanguage } from "./l10n";
import { store } from "./model/store";
import { App } from "./ui/App";
import { loadWindowState } from "./ui/root";

const darkScheme = window.matchMedia("(prefers-color-scheme: dark)");

/** Light or dark as Settings picks it, else as the system is. */
function applyAppearance(): void {
  const appearance = preferences().appearance;
  const dark = appearance === "dark" || (appearance !== "light" && darkScheme.matches);
  document.documentElement.toggleAttribute("data-dark", dark);
}

async function start(): Promise<void> {
  await loadHost();
  document.documentElement.dataset.platform = hostInfo().platform;
  setLanguage(preferences().appLanguage, hostInfo().locale);
  applyAppearance();
  darkScheme.addEventListener("change", applyAppearance);
  onPreferencesChanged((prefs) => {
    setLanguage(prefs.appLanguage, hostInfo().locale);
    applyAppearance();
  });
  store.start();
  loadWindowState();
  render(() => <App />, document.getElementById("root")!);
}

void start();
