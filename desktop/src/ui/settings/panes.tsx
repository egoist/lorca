// Settings panes in the main window's content area, after the macOS app's settings view
// controllers: section cards and footnotes in one scrolling column, the window's title bar carrying
// the page title. General and Advanced also fill the small Settings window onboarding opens.

import { createEffect, createMemo, createSignal, For, onSettled, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { host, hostInfo, onUpdaterChanged, preferences, setPreferences, type UpdaterState } from "../../host";
import { chosenLanguage, L, supportedLanguages } from "../../l10n";
import * as Format from "../../model/format";
import {
  behaviorTitle,
  deviceSymbol,
  isRunner,
  osDisplayName,
  paneSymbol,
  paneTitle,
  providerName,
  providerSubtitle,
  providerSymbol,
  usesAPIKey,
  type AutoReviewRule,
  type Device,
  type SettingsPane,
} from "../../model/models";
import { track } from "../../model/reactive";
import { errorText, store } from "../../model/store";
import { newBot, presentMarketplace } from "../actions";
import { HoverButton, PopUpButton, Switch, TextArea } from "../controls";
import { Icon } from "../icons";
import { alert, presentSheet, Sheet } from "../overlay";
import { open, revealed, settingsDeviceID } from "../root";
import { AccessoryRow, ActionRow, BotRow, EditableRow, KeyValueRow, NoteRow, PluginRow, Section, StatusRow } from "../sections";
import { presentConnectProvider } from "../sheets/connectProvider";
import { presentPlugin } from "../sheets/plugin";
import { Entries } from "./search";

export function SettingsPage(props: { pane: SettingsPane }) {
  return <>{paneFor(props.pane)}</>;
}

function paneFor(pane: SettingsPane): JSX.Element {
  switch (pane) {
    case "general":
      return <GeneralPane />;
    case "providers":
      return <ProvidersPane />;
    case "auto-review":
      return <AutoReviewPane />;
    case "plugins":
      return <PluginsPane />;
    case "bots":
      return <BotsPane />;
    case "device":
      return <DevicePane />;
    case "advanced":
      return <AdvancedPane />;
  }
}

/** A page: its cards and footnotes in a column that scrolls. A setting a search picked scrolls into
 * view and flashes. */
function PaneFrame(props: { pane: SettingsPane; children: JSX.Element }) {
  let scroller: HTMLDivElement | undefined;
  const flash = (row: string) => {
    if (!scroller) return;
    const targets = [...scroller.querySelectorAll<HTMLElement>("[data-label]")].filter((element) => element.dataset.label === row);
    // A row before a section of the same name: the row is the setting.
    const target = targets.find((element) => element.classList.contains("row")) ?? targets[0];
    if (!target) return;
    target.scrollIntoView({ block: "nearest", behavior: "smooth" });
    target.classList.remove("flash");
    void target.offsetWidth;
    target.classList.add("flash");
    setTimeout(() => target.classList.remove("flash"), 1300);
  };
  createEffect(
    () => revealed.read(),
    (picked) => {
      if (!picked || picked.entry.pane !== props.pane) return;
      // The page may have come on screen with this same click, so its rows settle first.
      requestAnimationFrame(() => flash(picked.entry.row));
    },
  );
  onSettled(() => {
    const picked = revealed.get();
    if (picked && picked.entry.pane === props.pane && Date.now() - picked.at < 1000) requestAnimationFrame(() => flash(picked.entry.row));
  });
  return (
    <div ref={(element) => (scroller = element)} class="settings-page">
      <div class="settings-column">{props.children}</div>
    </div>
  );
}

function Footnote(props: { text: string }) {
  return <div class="settings-footnote">{props.text}</div>;
}

// MARK: - General

export function GeneralPane() {
  const [prefs, setPrefs] = createSignal(preferences());
  const [updater, setUpdater] = createSignal<UpdaterState | null>(null);
  const update = (patch: Parameters<typeof setPreferences>[0]) => {
    setPrefs({ ...prefs(), ...patch });
    void setPreferences(patch);
  };
  onSettled(() => {
    if (!hostInfo().updatesEnabled) return;
    void host.updaterState().then(setUpdater);
    return onUpdaterChanged(setUpdater);
  });
  const lastCheck = () => {
    const state = updater();
    if (!state || state.lastCheck === 0) return L("Never checked");
    return L("Last checked %@", Format.dateTime(state.lastCheck * 1000));
  };
  return (
    <PaneFrame pane="general">
      <Section title={L("Chats")} style="heading">
        <AccessoryRow label={Entries.sendOnReturn().row}>
          <Switch small checked={prefs().sendOnReturn} label={Entries.sendOnReturn().row} onChange={(on) => update({ sendOnReturn: on })} />
        </AccessoryRow>
        <AccessoryRow label={Entries.timestamps().row}>
          <Switch small checked={prefs().showTimestamps} label={Entries.timestamps().row} onChange={(on) => update({ showTimestamps: on })} />
        </AccessoryRow>
      </Section>
      <Section title={L("Appearance")} style="heading">
        <AccessoryRow label={Entries.appearance().row}>
          <PopUpButton
            style="settings"
            label={Entries.appearance().row}
            options={[
              { value: "", label: L("System") },
              { value: "light", label: L("Light") },
              { value: "dark", label: L("Dark") },
            ]}
            value={prefs().appearance === "light" || prefs().appearance === "dark" ? prefs().appearance : ""}
            onChange={(appearance) => update({ appearance })}
          />
        </AccessoryRow>
        {/* The app's own language. Each one is named in itself, so it reads whatever is showing. */}
        <AccessoryRow label={Entries.appLanguage().row}>
          <PopUpButton
            style="settings"
            label={Entries.appLanguage().row}
            options={[{ value: "", label: L("System") }, ...supportedLanguages.map((language, index) => ({ value: language.code as string, label: language.name, separated: index === 0 }))]}
            value={chosenLanguage() ?? ""}
            onChange={(appLanguage) => update({ appLanguage })}
          />
        </AccessoryRow>
      </Section>
      <Show when={hostInfo().updatesEnabled}>
        <Section title={L("Updates")} style="heading">
          <ActionRow
            label={Entries.version().row}
            value={`${updater()?.version ?? hostInfo().version} · ${lastCheck()}`}
            tint="var(--label-2)"
            actionTitle={L("Check for Updates…")}
            onAction={() => void host.checkForUpdates()}
          />
          <AccessoryRow label={Entries.automaticChecks().row}>
            <Switch
              small
              checked={updater()?.automaticChecks ?? false}
              label={Entries.automaticChecks().row}
              onChange={(on) => {
                const state = updater();
                if (!state) return;
                setUpdater({ ...state, automaticChecks: on });
                void host.setAutomaticUpdates(on, state.automaticDownloads);
              }}
            />
          </AccessoryRow>
          <AccessoryRow label={Entries.automaticDownloads().row}>
            <Switch
              small
              checked={updater()?.automaticDownloads ?? false}
              disabled={!updater()?.automaticChecks}
              label={Entries.automaticDownloads().row}
              onChange={(on) => {
                const state = updater();
                if (!state) return;
                setUpdater({ ...state, automaticDownloads: on });
                void host.setAutomaticUpdates(state.automaticChecks, on);
              }}
            />
          </AccessoryRow>
        </Section>
      </Show>
      <Footnote text={L("Lorca talks only to the CLI on this computer. Nothing here is synced; each Device keeps its own settings.")} />
    </PaneFrame>
  );
}

// MARK: - Advanced

export function AdvancedPane() {
  const [prefs, setPrefs] = createSignal(preferences());
  const identity = createMemo(() => {
    track.connection();
    return store.hasIdentity === true;
  });
  const relayValue = () => {
    track.roster();
    return store.relayURL ?? prefs().relayURL;
  };
  const commitRelay = (value: string) => {
    if (value === (store.relayURL ?? "")) return;
    setPrefs({ ...prefs(), relayURL: value });
    void setPreferences({ relayURL: value });
    store.setRelayURL(value);
  };
  const commitPort = (value: string) => {
    const number = Number(value);
    if (!Number.isInteger(number) || number <= 0 || number >= 65536) {
      // The row shows the port in force again.
      setPrefs({ ...preferences() });
      return;
    }
    if (number === prefs().cliPort) return;
    setPrefs({ ...prefs(), cliPort: number });
    // The app reconnects to the CLI on the new port.
    void setPreferences({ cliPort: number });
  };
  const confirmDeleteAccount = async () => {
    const answer = await alert({
      message: L("Delete this account?"),
      informative: L("The relay deletes everything it holds for the account, and every paired Device, this one included, forgets its keys and chats. This can’t be undone."),
      style: "critical",
      buttons: [{ title: L("Delete Account"), destructive: true }, { title: L("Cancel") }],
    });
    if (answer !== 0) return;
    try {
      await store.deleteAccount();
    } catch (error) {
      void alert({ message: L("Couldn’t delete the account"), informative: errorText(error) });
    }
  };
  return (
    <PaneFrame pane="advanced">
      <Section title={L("Connection")} style="heading">
        <EditableRow label={Entries.relayURL().row} value={relayValue()} placeholder={hostInfo().productionRelayURL} monospaced alignRight onCommit={commitRelay} />
        <EditableRow label={Entries.cliPort().row} value={String(prefs().cliPort)} placeholder={String(hostInfo().defaultCLIPort)} monospaced alignRight onCommit={commitPort} />
      </Section>
      <Footnote
        text={L(
          "Self-hosting the relay is a URL change: clients sign their requests and upload ciphertext, so the relay has nothing to trust. Leave it empty to use Lorca’s relay.",
        )}
      />
      <Section title={L("Setup")} style="heading">
        {/* Onboarding closes the window this row is in, so the click returns first. */}
        <ActionRow label={Entries.onboarding().row} tint="var(--label-2)" actionTitle={L("Show Onboarding Again")} onAction={() => setTimeout(() => void host.showOnboarding())} />
      </Section>
      <Show when={identity()}>
        <Section title={L("Account")} style="heading">
          <ActionRow label={Entries.deleteAccount().row} tint="var(--label-2)" actionTitle={L("Delete Account…")} onAction={() => void confirmDeleteAccount()} />
        </Section>
        <Footnote text={L("Deletes the account from the relay and from every paired Device: chats, attachments, bots, and provider credentials.")} />
      </Show>
    </PaneFrame>
  );
}

// MARK: - Providers

/** The account's provider credentials: connected on any Device, used by every Runner. */
function ProvidersPane() {
  const providers = () => {
    track.roster();
    return store.providers;
  };
  return (
    <PaneFrame pane="providers">
      <Section title={L("Credentials")} style="heading">
        <Show when={providers().length > 0} fallback={<KeyValueRow label={L("Waiting for the CLI")} value="" />}>
          <For each={providers()} keyed={(credential) => credential.kind}>
            {(credential) => {
              const signIn = () => credential().isConnected && !usesAPIKey(credential().kind);
              return (
                <StatusRow
                  symbol={providerSymbol(credential().kind)}
                  title={providerName(credential().kind)}
                  subtitle={`${providerSubtitle(credential().kind)} · ${credential().detail}`}
                  state={credential().isConnected ? L("Connected") : undefined}
                  stateColor="var(--green)"
                  actionTitle={credential().isConnected ? (usesAPIKey(credential().kind) ? L("Edit…") : L("Disconnect")) : L("Connect…")}
                  destructive={signIn()}
                  onAction={() => {
                    if (signIn()) void store.disconnectProvider(credential().kind).catch(() => {});
                    else void presentConnectProvider(credential().kind, { baseURL: credential().baseURL });
                  }}
                />
              );
            }}
          </For>
        </Show>
      </Section>
      <Footnote
        text={L(
          "Credentials belong to your account. They reach your paired Devices encrypted with the account key, so a bot uses them on whichever Runner it is assigned to; the relay stores ciphertext.",
        )}
      />
    </PaneFrame>
  );
}

// MARK: - Auto-review

/** The switch and the rules, shared by every Device through the roster. Add and Edit use a sheet;
 * a card's Always allow adds a rule here. */
function AutoReviewPane() {
  const review = () => {
    track.roster();
    return store.autoReview;
  };
  const save = (rule: AutoReviewRule | null, text: string, behavior: AutoReviewRule["behavior"]) => {
    const current = store.autoReview;
    const index = rule ? current.rules.findIndex((each) => each.id === rule.id) : -1;
    const rules = [...current.rules];
    if (rule && index >= 0) {
      const updated = { ...rules[index]! };
      if (updated.tool === undefined) updated.text = text;
      updated.behavior = behavior;
      rules[index] = updated;
    } else {
      rules.push({ id: "", text, behavior });
    }
    store.setAutoReview({ ...current, rules });
  };
  const remove = (id: string) => {
    const current = store.autoReview;
    store.setAutoReview({ ...current, rules: current.rules.filter((rule) => rule.id !== id) });
  };
  return (
    <PaneFrame pane="auto-review">
      <Section title={L("Auto-review")} style="heading">
        <AccessoryRow label={Entries.autoReviewSwitch().row}>
          <Switch
            small
            checked={review().isEnabled}
            label={Entries.autoReviewSwitch().row}
            onChange={(on) => store.setAutoReview({ ...store.autoReview, isEnabled: on })}
          />
        </AccessoryRow>
        <NoteRow text={L("Lorca checks each action before it runs and asks you first when needed. Add rules to customize what bots can do automatically.")} />
      </Section>
      <Section
        title={Entries.autoReviewRules().row}
        style="heading"
        accessory={<HoverButton symbol="plus" size={13} tooltip={L("Add rule")} onClick={() => presentRuleEditor(null, save)} />}
      >
        <Show when={review().rules.length > 0} fallback={<NoteRow text={L("No rules yet. Always allow on a card adds one, or write one below.")} />}>
          <For each={review().rules} keyed={(rule) => rule.id || rule.text}>
            {(rule) => (
              <div class="row rule-row" data-label={rule().text}>
                <span class="rule-text selectable">{rule().text}</span>
                <div class="rule-meta">
                  <span class="rule-behavior" title={behaviorTitle(rule().behavior)}>
                    {behaviorTitle(rule().behavior)}
                  </span>
                  <HoverButton symbol="square.and.pencil" size={13} tooltip={L("Edit rule")} onClick={() => presentRuleEditor(rule(), save)} />
                  <HoverButton symbol="trash" size={13} tooltip={L("Delete rule")} onClick={() => remove(rule().id)} />
                </div>
              </div>
            )}
          </For>
        </Show>
      </Section>
      <Footnote
        text={L(
          'Read-only commands and commands inside Lorca\'s own folders run at once. Auto-review checks effectful plugin actions and every other shell command before they run: a small, fast model on the bot\'s provider applies your rules and latest request, so safe work normally runs automatically and risky work asks. Off, every such action asks. Write one short, natural-language rule for each action; "Ask first" takes priority if rules conflict. Built-in safety checks always apply.',
        )}
      />
    </PaneFrame>
  );
}

/** Add and Edit share one sheet. Rule text is editable; a rule for an exact plugin tool keeps its
 * tool, while its Allow/Ask behavior can still change. */
function presentRuleEditor(rule: AutoReviewRule | null, onSave: (rule: AutoReviewRule | null, text: string, behavior: AutoReviewRule["behavior"]) => void): void {
  const exact = rule?.tool !== undefined;
  presentSheet((dismiss) => {
    let text = rule?.text ?? "";
    const [behavior, setBehavior] = createSignal<AutoReviewRule["behavior"]>(rule?.behavior === "ask" ? "ask" : "allow");
    const save = () => {
      const value = (exact ? (rule?.text ?? "") : text).trim();
      if (value === "") {
        void host.beep();
        return;
      }
      onSave(rule, value, behavior());
      dismiss();
    };
    return (
      <Sheet
        title={rule === null ? L("Add rule") : L("Edit rule")}
        subtitle={exact ? L("Exact actions cannot be changed. Delete this rule and allow a different action instead.") : L("Describe when a bot should be allowed automatically or asked first.")}
        width={500}
        confirm={rule === null ? L("Add rule") : L("Save")}
        onConfirm={save}
        onCancel={dismiss}
      >
        <div class="field-stack">
          <span class="sheet-caption">{L("Rule").toUpperCase()}</span>
          <TextArea value={text} readOnly={exact} autofocus={!exact} class="rule-editor" label={L("Rule")} onInput={(value) => (text = value)} />
        </div>
        <Section title={L("Behavior")}>
          <AccessoryRow label={L("Auto-review")}>
            <PopUpButton
              style="settings"
              options={[
                { value: "allow" as const, label: behaviorTitle("allow") },
                { value: "ask" as const, label: behaviorTitle("ask") },
              ]}
              value={behavior()}
              onChange={setBehavior}
            />
          </AccessoryRow>
        </Section>
      </Sheet>
    );
  });
}

// MARK: - Device panes

/** The Device the Plugins, Bots, and Devices panes show, picked in the window's toolbar. */
function pickedDevice(): Device | undefined {
  track.roster();
  const id = settingsDeviceID.read();
  return id ? store.device(id) : undefined;
}

/** The one row a Runner's section shows for a Device that is not one, or before the CLI answers. */
function Placeholder(props: { device: Device | undefined; children: JSX.Element }) {
  return (
    <Show when={props.device} fallback={<KeyValueRow label={L("Waiting for the CLI")} value="" />}>
      {(device) => (
        <Show
          when={isRunner(device())}
          fallback={<NoteRow text={L("%@ Devices hold your keys and chats but never run a bot. Pick a Runner: a Device running macOS, Linux, or Windows.", osDisplayName(device().os))} />}
        >
          {props.children}
        </Show>
      )}
    </Show>
  );
}

/** What the picked Runner has installed, with each plugin's state, and the marketplace. */
function PluginsPane() {
  const device = pickedDevice;
  return (
    <PaneFrame pane="plugins">
      <Section title={device() ? L("Plugins on %@", device()!.name) : L("Plugins")} style="heading">
        <Placeholder device={device()}>
          <For each={device()?.plugins ?? []} keyed={(plugin) => plugin.id}>
            {(plugin) => (
              <PluginRow
                plugin={plugin()}
                onClick={() => {
                  const runner = device();
                  if (runner) presentPlugin(plugin().id, runner);
                }}
              />
            )}
          </For>
          <Show when={(device()?.plugins.length ?? 0) === 0}>
            <KeyValueRow label={L("No plugins installed")} value="" tint="var(--label-2)" />
          </Show>
          <ActionRow label={L("Marketplace")} tint="var(--label-2)" actionTitle={L("Add from Plugins…")} onAction={() => presentMarketplace(device()?.id ?? null)} />
        </Placeholder>
      </Section>
      <Footnote text={L("Plugins are installed on a Runner, and the bots assigned to it use them. An action that changes something goes through Auto-review first.")} />
    </PaneFrame>
  );
}

function BotsPane() {
  const device = pickedDevice;
  const bots = () => {
    const current = device();
    return current ? store.botsOn(current.id) : [];
  };
  const openChat = (botID: string) => open(store.dm(botID));
  return (
    <PaneFrame pane="bots">
      <Section title={device() ? L("Bots on %@", device()!.name) : L("Bots")} style="heading">
        <Placeholder device={device()}>
          <For each={bots()} keyed={(bot) => bot.id}>
            {(bot) => (
              <BotRow
                bot={bot()}
                detail={providerName(bot().provider)}
                accessorySymbol="bubble.left"
                accessoryTooltip={L("Open chat")}
                onAccessory={() => openChat(bot().id)}
                onClick={() => openChat(bot().id)}
              />
            )}
          </For>
          <Show when={bots().length === 0}>
            <KeyValueRow label={L("No bots assigned")} value="" tint="var(--label-2)" />
          </Show>
          <ActionRow label={L("New")} tint="var(--label-2)" actionTitle={L("New Bot…")} onAction={newBot} />
        </Placeholder>
      </Section>
      <Footnote text={L("A bot runs on the Runner it is assigned to, with your account's credentials and that Runner's plugins.")} />
    </PaneFrame>
  );
}

/** The one confirmation every device list shares. */
export async function confirmUnpair(device: Device): Promise<void> {
  if (device.isThisDevice) {
    void host.beep();
    return;
  }
  const answer = await alert({
    message: L('Unpair "%@"?', device.name),
    informative: isRunner(device)
      ? L("It loses its keys and synced chats the next time it connects, and bots assigned to it stop running until you assign them to another Runner. You can pair it again any time.")
      : L("It loses its keys and synced chats the next time it connects. You can pair it again any time."),
    style: "warning",
    buttons: [{ title: L("Unpair"), destructive: true }, { title: L("Cancel") }],
  });
  if (answer !== 0) return;
  try {
    await store.unpairDevice(device.id);
  } catch (error) {
    void alert({ message: L("Couldn’t unpair %@", device.name), informative: errorText(error) });
  }
}

/** Presence as a small dot: green online, orange pairing, gray offline. */
function StatusDot(props: { status: Device["status"] }) {
  return <span class={["status-dot", props.status]} />;
}

/** The picked Device itself: what it is, whether it is online, its machine key, and Unpair. */
function DevicePane() {
  const device = pickedDevice;
  const relay = () => {
    track.roster();
    track.connection();
    const url = store.relayURL;
    if (!url) return L("Not configured");
    if (store.relayUpdateRequired) return L("%@ · update Lorca to sync", url);
    if (store.relayConnected) return url;
    // Why the last try to connect failed, in the CLI's words.
    return store.relayError ? `${url} · ${store.relayError}` : L("%@ · offline", url);
  };
  const status = (current: Device): { text: string; color: string } => {
    switch (current.status) {
      case "online":
        return {
          text: current.isThisDevice ? L("This computer · CLI running") : isRunner(current) ? L("Online · paired") : L("Online · paired · not a Runner"),
          color: "var(--green)",
        };
      case "pairing":
        return { text: L("Pairing…"), color: "var(--orange)" };
      default:
        return { text: Format.lastSeen(current.lastSeen) + (isRunner(current) ? L(" · jobs wait on the relay") : ""), color: "var(--label-2)" };
    }
  };
  return (
    <PaneFrame pane="device">
      <Show when={device()}>
        {(current) => (
          <>
            <div class="device-header">
              <span class="device-header-icon">
                <Icon name={deviceSymbol(current())} size={40} strokeWidth={1.4} />
              </span>
              <div class="device-header-text">
                <span class="device-header-name">{current().name}</span>
                <span class="device-header-model">{`${current().model} · ${current().osVersion}`}</span>
                <span class="device-header-status" style={{ color: status(current()).color }}>
                  <StatusDot status={current().status} />
                  {status(current()).text}
                </span>
              </div>
            </div>
            <Section title={L("Machine")} style="heading">
              <KeyValueRow label={Entries.machineKey().row} value={current().machineKey} monospaced />
              <KeyValueRow label={L("OS")} value={`${osDisplayName(current().os)} · ${current().osVersion}`} />
              <KeyValueRow label={L("Role")} value={isRunner(current()) ? L("Runner · runs bots with its own credentials") : L("Device · never runs bots")} />
              <KeyValueRow label={L("Last seen")} value={current().status === "online" ? L("Active now") : Format.lastSeen(current().lastSeen)} />
              <KeyValueRow label={L("Relay")} value={relay()} monospaced />
              <Show when={!current().isThisDevice}>
                <ActionRow
                  label={Entries.pairing().row}
                  value={L("Paired to this account")}
                  tint="var(--label-2)"
                  actionTitle={L("Unpair…")}
                  onAction={() => void confirmUnpair(current())}
                />
              </Show>
            </Section>
          </>
        )}
      </Show>
    </PaneFrame>
  );
}

// MARK: - The small window

/** Settings while onboarding is up: General and Advanced, the panes that work without an account,
 * as tabs in a window of their own. */
export function SettingsWindow() {
  const [tab, setTab] = createSignal<"general" | "advanced">("general");
  return (
    <div class="settings-window">
      <div class="settings-tabs" role="tablist">
        <For each={["general", "advanced"] as const}>
          {(pane) => (
            <button role="tab" aria-selected={tab() === pane ? "true" : "false"} class={["settings-tab", { selected: tab() === pane }]} onClick={() => setTab(pane)}>
              <Icon name={paneSymbol(pane)} size={20} strokeWidth={1.6} />
              <span>{paneTitle(pane)}</span>
            </button>
          )}
        </For>
      </div>
      <div class="settings-window-pane">
        <Show when={tab() === "general"} fallback={<AdvancedPane />}>
          <GeneralPane />
        </Show>
      </div>
    </div>
  );
}

