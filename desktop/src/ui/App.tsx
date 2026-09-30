// The app's pages. Each window opens on its route: the main window on `/` (a chat at `/chat/:id`, a
// settings pane at `/settings/:pane`), onboarding on `/onboarding`, and while onboarding is up, the
// small Settings window of General and Advanced on `/settings-window`. A new language builds the
// page again on the route it is on.

import { createRouter, useNavigate } from "@solidjs/router";
import { For, onCleanup } from "solid-js";
import { languageKey } from "../l10n";
import { settingsPanes } from "../model/models";
import { ChatRoute, HomeRoute, MainWindow, SettingsRoute } from "./MainWindow";
import { PageMenuHost } from "./menu";
import { Onboarding } from "./onboarding";
import { PopoverHost, SheetHost } from "./overlay";
import { SettingsWindow } from "./settings/panes";
import { setupWindow } from "./window";

// The menu bar keeps its items in step through a memo and an effect, so it is set up with the
// window's page, not after it settles.
function OnboardingRoute() {
  onCleanup(setupWindow("other"));
  return <Onboarding />;
}

function SettingsWindowRoute() {
  onCleanup(setupWindow("other"));
  return <SettingsWindow />;
}

/** Any other address is the main window's. */
function Elsewhere() {
  useNavigate()(`/${window.location.search}`, { replace: true });
  return null;
}

export const Router = createRouter({
  routes: [
    { path: "/onboarding", component: OnboardingRoute },
    { path: "/settings-window", component: SettingsWindowRoute },
    {
      path: "/",
      component: MainWindow,
      children: [
        { path: "/", component: HomeRoute },
        { path: "/chat/:id", component: ChatRoute },
        { path: "/settings/:pane", component: SettingsRoute, matchFilters: { pane: [...settingsPanes] } },
      ],
    },
    { path: "*", component: Elsewhere },
  ],
  // The pages have no router links: an anchor is a message's link, for the browser.
  explicitLinks: true,
  preloadLinks: false,
  scrollRestoration: false,
});

export function App() {
  return (
    <For each={[languageKey()]}>
      {() => (
        <Router>
          {(props) => (
            <>
              {props.children}
              <SheetHost />
              <PopoverHost />
              <PageMenuHost />
            </>
          )}
        </Router>
      )}
    </For>
  );
}
