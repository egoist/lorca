// Onboarding, after the macOS app's OnboardingWindowController: a new identity with its backup
// phrase, a restore from that phrase, or pairing with a Device that has the account; then the first
// bot and a provider for it. Its own window, which the main window replaces when it finishes.

import { createMemo, createSignal, For, onSettled, Show, type Accessor } from "solid-js";
import type { JSX } from "@solidjs/web";
import iconURL from "./images/icon.png";
import devIconURL from "./images/icon-dev.png";
import { host, hostInfo } from "../host";
import { L, Lc } from "../l10n";
import { keyPlaceholder, providerKinds, providerName, signInRequirement, usesAPIKey, type ProviderKind } from "../model/models";
import { onStoreEvent, track } from "../model/reactive";
import { errorText, store } from "../model/store";
import { Avatar, botAvatar } from "./avatar";
import { Button, CopyButton, PopUpButton } from "./controls";
import { Icon } from "./icons";
import { alert } from "./overlay";

type Step = "welcome" | "create" | "restore" | "pair" | "bot" | "provider" | "done";

/** The first bot's description as the CLI writes it, which the page shows in the app's language. */
const defaultDescription = "Chief of staff. Plans the work and delegates each task to the right teammate, proposing a new one when none fits. Does hands-on work when necessary.";

export function Onboarding() {
  const [step, setStep] = createSignal<Step>("welcome");
  const [phrase, setPhrase] = createSignal<string[]>([]);
  const [savedPhrase, setSavedPhrase] = createSignal(false);
  const [status, setStatus] = createSignal<{ text: string; color: string } | null>(null);
  const [providerKind, setProviderKind] = createSignal<ProviderKind>("deepseek");
  const [apiKey, setAPIKey] = createSignal("");
  let busy = false;

  /** The bot the CLI created with the identity, or the first bot in the roster. */
  const firstBot = createMemo(() => {
    track.roster();
    return store.bots.find((bot) => bot.name === "Chef") ?? store.bots[0];
  });

  const go = (next: Step) => {
    setStatus(null);
    setAPIKey("");
    setStep(next);
  };

  onSettled(() =>
    onStoreEvent((event) => {
      // Identity arrived from elsewhere (the CLI, or a paired Device) while the page waited.
      if (event.kind !== "identityChanged" || store.hasIdentity !== true || busy) return;
      if (step() === "welcome" || step() === "restore" || step() === "pair") go("done");
    }),
  );

  const presentError = (message: string) => void alert({ message: hostInfo().name, informative: message, style: "warning", buttons: [{ title: L("OK") }] });

  const createIdentity = async () => {
    if (busy) return;
    if (store.isMock) {
      const { backupPhrase } = await import("../model/mock");
      setPhrase(backupPhrase);
      go("create");
      return;
    }
    if (!store.isConnected) {
      presentError(L("The Lorca CLI is not running. Start it with `lorca serve` and try again.").replace("lorca serve", hostInfo().cliCommand));
      return;
    }
    busy = true;
    try {
      setPhrase(await store.createIdentity());
      go("create");
    } catch (error) {
      presentError(errorText(error));
    } finally {
      busy = false;
    }
  };

  /** A computer that joined an account gets its credentials with the first sync, so the provider
   * step shows only when none arrive. */
  const continueAfterJoining = async () => {
    const connected = () => store.providers.some((credential) => credential.isConnected);
    for (let attempt = 0; attempt < 15 && !connected(); attempt++) await new Promise((resolve) => setTimeout(resolve, 200));
    go(connected() ? "done" : "provider");
  };

  const join = async (text: string, waiting: string, run: (text: string) => Promise<void>) => {
    const trimmed = text.trim();
    if (busy || trimmed === "") return;
    if (store.isMock) {
      go("provider");
      return;
    }
    busy = true;
    setStatus({ text: waiting, color: "var(--label-2)" });
    try {
      await run(trimmed);
      await continueAfterJoining();
    } catch (error) {
      setStatus({ text: errorText(error), color: "var(--red)" });
    } finally {
      busy = false;
    }
  };

  const connectProvider = async () => {
    if (busy) return;
    if (store.isMock) {
      go("done");
      return;
    }
    const kind = providerKind();
    const key = apiKey().trim();
    if (usesAPIKey(kind) && key === "") {
      setStatus({ text: kind === "anthropic" ? L("Paste an %@ API key to continue.", providerName(kind)) : L("Paste a %@ API key to continue.", providerName(kind)), color: "var(--red)" });
      return;
    }
    busy = true;
    setStatus({ text: usesAPIKey(kind) ? L("Checking the key with %@…", providerName(kind)) : L("Waiting for the browser…"), color: "var(--label-2)" });
    try {
      if (usesAPIKey(kind)) await store.connectAPIKey(kind, key);
      else await store.connectSignIn(kind);
      go("done");
    } catch (error) {
      setStatus({ text: errorText(error), color: "var(--red)" });
    } finally {
      busy = false;
    }
  };

  const saveFirstBot = (name: string, description: string) => {
    const bot = firstBot();
    if (busy || !bot) {
      go("provider");
      return;
    }
    if (name === "") {
      setStatus({ text: L("Give the bot a name."), color: "var(--red)" });
      return;
    }
    // The bot runs with the provider chosen here, not the CLI's default.
    if (name !== bot.name || description !== bot.description || providerKind() !== bot.provider) store.updateBotProfile(bot.id, name, description, providerKind());
    void connectProvider();
  };

  const providerRows = () => (
    <ProviderRows kind={providerKind} setKind={setProviderKind} apiKey={apiKey} setAPIKey={setAPIKey} status={status} />
  );
  /** The Continue button of a step with a credential: signing in says so, a key must be typed. */
  const credentialButton = (run: () => void) => ({
    title: usesAPIKey(providerKind()) ? L("Continue") : L("Sign in with %@", providerName(providerKind())),
    enabled: !usesAPIKey(providerKind()) || apiKey().trim() !== "",
    run,
  });

  return (
    <div class="onboarding">
      <Show when={step()} keyed>
        {(current) => {
          switch (current) {
            case "welcome":
              return <Welcome onCreate={() => void createIdentity()} onRestore={() => go("restore")} onPair={() => go("pair")} />;
            case "create":
              return (
                <StepLayout
                  title={L("Your backup phrase")}
                  subtitle={L("This phrase is your master secret. It re-derives every key and unwraps everything on the relay. Write it down — nobody can reset it for you.")}
                  next={{ title: L("Continue"), enabled: savedPhrase(), run: () => go(firstBot() ? "bot" : "provider") }}
                >
                  <div class="onboarding-create">
                    <div class="phrase-grid">
                      <For each={phrase()}>
                        {(word, index) => (
                          <div class="phrase-cell">
                            <span class="phrase-number">{index() + 1}</span>
                            <span class="phrase-word selectable">{word}</span>
                          </div>
                        )}
                      </For>
                    </div>
                    <div>
                      <CopyButton bordered text={() => phrase().join(" ")} title={L("Copy Phrase")} />
                    </div>
                    <label class="checkbox">
                      <input type="checkbox" checked={savedPhrase()} onChange={(event) => setSavedPhrase(event.currentTarget.checked)} />
                      <span>{L("I wrote this phrase down somewhere safe")}</span>
                    </label>
                  </div>
                </StepLayout>
              );
            case "restore":
              return (
                <JoinStep
                  title={L("Restore your identity")}
                  subtitle={L("Paste the twelve groups from your backup phrase. Everything else is re-derived and unwrapped from the relay.")}
                  placeholder="k4mq 7rth 2bnz …"
                  note={
                    store.relayURL === null
                      ? L("Restoring unwraps the account key from the relay. Set a relay URL in Settings › Advanced first.")
                      : L("Restoring never contacts a login server. The relay only answers a signature challenge.")
                  }
                  status={status()}
                  action={L("Restore")}
                  onBack={() => go("welcome")}
                  onSubmit={(text) => void join(text, L("Re-deriving keys and unwrapping the account key…"), (phraseText) => store.restoreIdentity(phraseText))}
                />
              );
            case "pair":
              return (
                <JoinStep
                  title={L("Pair this computer")}
                  subtitle={L(
                    "On a computer that already has your identity, choose File › Pair a Device in the desktop app or run lorca pair in a terminal, then paste the code here.",
                  )}
                  placeholder="lorca://pair?relay=…"
                  note={L("The two Devices run a handshake; the relay only carries the ciphertext. This computer joins as a Runner.")}
                  status={status()}
                  action={L("Pair")}
                  onBack={() => go("welcome")}
                  onSubmit={(text) => void join(text, L("Waiting for the other Device to wrap the account key…"), (code) => store.acceptPairing(code))}
                />
              );
            case "bot":
              return <FirstBot bot={firstBot()} providerRows={providerRows} button={(run) => credentialButton(run)} onSave={saveFirstBot} />;
            case "provider":
              return (
                <StepLayout
                  title={L("Connect a provider")}
                  subtitle={L(
                    "Credentials belong to your account: your bots use them on every Runner you pair. They sync encrypted with your account key; the relay cannot read them.",
                  )}
                  back={{ title: L("Skip for Now"), run: () => go("done") }}
                  next={credentialButton(() => void connectProvider())}
                >
                  <div class="onboarding-card">
                    <div class="form-grid">{providerRows()}</div>
                  </div>
                </StepLayout>
              );
            case "done":
              return <Done botName={firstBot()?.name} />;
          }
        }}
      </Show>
    </div>
  );
}

function Welcome(props: { onCreate: () => void; onRestore: () => void; onPair: () => void }) {
  return (
    <div class="onboarding-center">
      <div class="onboarding-column">
        <img class="onboarding-icon" src={hostInfo().isDevelopment ? devIconURL : iconURL} width={96} height={96} alt="" draggable={false} />
        <div class="onboarding-app-name">{hostInfo().name}</div>
        <div class="onboarding-lede">
          {L("Bots that run on computers you own. Your identity is a key pair on this computer — no account, no server that can read your chats.")}
        </div>
        <div class="onboarding-choices">
          <Button kind="primary" large onClick={props.onCreate}>
            {L("Create a New Identity")}
          </Button>
          <Button large onClick={props.onRestore}>
            {L("Restore from Backup Phrase")}
          </Button>
          <Button large onClick={props.onPair}>
            {L("Pair with Another Device")}
          </Button>
        </div>
      </div>
    </div>
  );
}

interface StepButton {
  title: string;
  enabled?: boolean;
  run: () => void;
}

/** A step: its title and what it is for at the top, its body, and Back and Continue at the bottom.
 * Return presses Continue, as a default button does. */
function StepLayout(props: { title: string; subtitle: string; back?: StepButton; next: StepButton; children: JSX.Element }) {
  return (
    <div
      class="onboarding-step"
      onKeyDown={(event) => {
        if (event.key !== "Enter" || event.isComposing || event.defaultPrevented) return;
        const target = event.target as HTMLElement;
        if (target.tagName === "TEXTAREA" || target.tagName === "BUTTON") return;
        event.preventDefault();
        if (props.next.enabled !== false) props.next.run();
      }}
    >
      <div class="onboarding-header">
        <div class="onboarding-title">{props.title}</div>
        <div class="onboarding-subtitle">{props.subtitle}</div>
      </div>
      <div class="onboarding-body">{props.children}</div>
      <div class="onboarding-buttons">
        <Show when={props.back}>
          {(back) => (
            <Button large onClick={() => back().run()}>
              {back().title}
            </Button>
          )}
        </Show>
        <Button kind="primary" large class="onboarding-next" disabled={props.next.enabled === false} onClick={() => props.next.run()}>
          {props.next.title}
        </Button>
      </div>
    </div>
  );
}

function JoinStep(props: {
  title: string;
  subtitle: string;
  placeholder: string;
  note: string;
  status: { text: string; color: string } | null;
  action: string;
  onBack: () => void;
  onSubmit: (text: string) => void;
}) {
  const [text, setText] = createSignal("");
  let field: HTMLInputElement | undefined;
  onSettled(() => field?.focus());
  return (
    <StepLayout title={props.title} subtitle={props.subtitle} back={{ title: L("Back"), run: props.onBack }} next={{ title: props.action, run: () => props.onSubmit(text()) }}>
      <div class="onboarding-join">
        <input
          ref={(element) => (field = element)}
          class="text-field mono onboarding-wide"
          placeholder={props.placeholder}
          spellcheck={false}
          autocomplete="off"
          value={text()}
          onInput={(event) => setText(event.currentTarget.value)}
        />
        <div class="onboarding-note onboarding-wide" style={props.status ? { color: props.status.color } : undefined}>
          {props.status?.text ?? props.note}
        </div>
      </div>
    </StepLayout>
  );
}

/** Label column on the left, controls on the right, every control the same width. */
function FormRow(props: { label: string; top?: boolean; children: JSX.Element }) {
  return (
    <>
      <span class={["form-label", { top: !!props.top }]}>{props.label}</span>
      <div class="form-control">{props.children}</div>
    </>
  );
}

/** The provider picker and the credential for the chosen provider: a key field, or what signing in
 * does. Switching providers swaps only the credential row. */
function ProviderRows(props: {
  kind: Accessor<ProviderKind>;
  setKind: (kind: ProviderKind) => void;
  apiKey: Accessor<string>;
  setAPIKey: (key: string) => void;
  status: Accessor<{ text: string; color: string } | null>;
}) {
  const kind = props.kind;
  const note = () =>
    props.status() ?? {
      text: usesAPIKey(kind())
        ? L("The key is checked against %@ and shared with your paired Devices, encrypted.", providerName(kind()))
        : L("Tokens from the sign-in are shared with your paired Devices, encrypted."),
      color: "var(--label-3)",
    };
  return (
    <>
      <FormRow label={L("Provider")}>
        <PopUpButton
          class="fill"
          options={providerKinds.map((each) => ({ value: each, label: providerName(each) }))}
          value={kind()}
          onChange={(each) => {
            props.setAPIKey("");
            props.setKind(each);
          }}
        />
      </FormRow>
      <FormRow label={usesAPIKey(kind()) ? L("API key") : L("Account")} top={!usesAPIKey(kind())}>
        <Show
          when={usesAPIKey(kind())}
          fallback={<div class="form-text">{L("Your browser opens a %@ sign-in when you continue. %@", providerName(kind()), signInRequirement(kind()))}</div>}
        >
          <input
            class="text-field mono"
            type="password"
            placeholder={keyPlaceholder(kind())}
            aria-label={L("API key")}
            spellcheck={false}
            autocomplete="off"
            value={props.apiKey()}
            onInput={(event) => props.setAPIKey(event.currentTarget.value)}
          />
        </Show>
      </FormRow>
      <FormRow label="">
        <div class="form-note" style={{ color: note().color }}>
          {note().text}
        </div>
      </FormRow>
    </>
  );
}

function FirstBot(props: {
  bot: import("../model/models").Bot | undefined;
  providerRows: () => JSX.Element;
  button: (run: () => void) => StepButton;
  onSave: (name: string, description: string) => void;
}) {
  // The CLI's default profile reads in the app's language; a name or description the user saved
  // stays as it is.
  const [name, setName] = createSignal(props.bot && props.bot.name !== "Chef" ? props.bot.name : Lc("Chef", "first bot name"));
  const [description, setDescription] = createSignal(props.bot && props.bot.description !== defaultDescription
      ? props.bot.description
      : L("Chief of staff. Plans the work and delegates each task to the right teammate, proposing a new one when none fits. Does hands-on work when necessary."),
  );
  const save = () => props.onSave(name().trim(), description().trim());
  return (
    <StepLayout
      title={L("Your first bot")}
      subtitle={L("It runs on this computer, plans your work, and builds the rest of the team when you ask. Give it a name and the credentials it runs with.")}
      next={props.button(save)}
    >
      <div class="onboarding-card">
        <div class="form-grid">
          <FormRow label="">
            <Avatar content={props.bot ? botAvatar(props.bot) : { kind: "bot", symbolName: "sparkles", accent: "indigo" }} size={40} />
          </FormRow>
          <FormRow label={L("Name")}>
            <input class="text-field" placeholder={L("Name")} value={name()} spellcheck={false} onInput={(event) => setName(event.currentTarget.value)} />
          </FormRow>
          <FormRow label={L("Description")} top>
            <textarea
              class="text-area wrapping-field"
              rows={3}
              placeholder={L("What it does and how it should work")}
              value={description()}
              onInput={(event) => setDescription(event.currentTarget.value)}
              onKeyDown={(event) => {
                // Return continues, as in a wrapping text field; Option or Shift with it starts a line.
                if (event.key === "Enter" && !event.isComposing && !event.altKey && !event.shiftKey) {
                  event.preventDefault();
                  const button = props.button(save);
                  if (button.enabled !== false) button.run();
                }
              }}
            />
          </FormRow>
          {props.providerRows()}
        </div>
      </div>
    </StepLayout>
  );
}

function Done(props: { botName: string | undefined }) {
  let button: HTMLButtonElement | undefined;
  onSettled(() => button?.focus());
  return (
    <div class="onboarding-center">
      <div class="onboarding-column">
        <span class="onboarding-done-icon">
          <Icon name="checkmark.circle.fill" size={56} strokeWidth={1.6} />
        </span>
        <div class="onboarding-title center">{L("This computer is your first Device")}</div>
        <div class="onboarding-lede">
          {props.botName
            ? L("%@ is ready to talk to. Pair another Device any time from the File menu.", props.botName)
            : L("Bots you create here run on this computer with your account's provider credentials. Pair another Device any time from the File menu.")}
        </div>
        <Button kind="primary" large class="onboarding-open" ref={(element) => (button = element)} onClick={() => void host.finishOnboarding()}>
          {L("Open Lorca")}
        </Button>
      </div>
    </div>
  );
}
