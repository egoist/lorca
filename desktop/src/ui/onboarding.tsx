// Onboarding, after the macOS app's OnboardingWindowController: a new identity with its backup
// phrase, a restore from that phrase, or pairing with a Device that has the account; then the first
// bot and a provider for it. Its own window, which the main window replaces when it finishes.

import { createMemo, createSignal, For, onSettled, Show, type Accessor } from "solid-js";
import type { JSX } from "@solidjs/web";
import iconURL from "./images/icon.png";
import devIconURL from "./images/icon-dev.png";
import { host, hostInfo } from "../host";
import { L, Lc } from "../l10n";
import { customPresets, keyPlaceholder, providerKinds, providerName, signInRequirement, usesAPIKey, type BuiltInProviderKind, type CustomPreset } from "../model/models";
import { onStoreEvent, track } from "../model/reactive";
import { errorText, store } from "../model/store";
import { Avatar, botAvatar } from "./avatar";
import { Button, CopyButton, PopUpButton, Spinner } from "./controls";
import { Icon } from "./icons";
import { alert, hasSheet } from "./overlay";
import { presentCustomProvider } from "./sheets/customProvider";

type Step = "welcome" | "create" | "restore" | "pair" | "bot" | "provider" | "done";
/** How this computer came by its identity, which the last step words. */
type Origin = "created" | "restored" | "paired";
/** A restore or a pairing in flight. Back cancels `pairing`, which waits on the other Device; a
 * restore, and the sync after either, end on their own. */
type Joining = "restoring" | "pairing" | "syncing";

/** The first bot's description as the CLI writes it, which the page shows in the app's language. */
const defaultDescription = "Chief of staff. Plans the work and delegates each task to the right teammate, proposing a new one when none fits. Does hands-on work when necessary.";

// Where onboarding stands lives outside the page: a language change draws the page again, and the
// user stays on the step they were on, with the phrase they were shown and what they typed.
const [step, setStep] = createSignal<Step>("welcome");
const [phrase, setPhrase] = createSignal<string[]>([]);
const [savedPhrase, setSavedPhrase] = createSignal(false);
const [status, setStatus] = createSignal<{ text: string; color: string } | null>(null);
const [providerKind, setProviderKind] = createSignal<BuiltInProviderKind>("deepseek");
/** A custom provider picked instead of a built-in one: a preset, or none inside for Other Server.
 * It is set up in its own sheet, and the first bot moves to it once it is saved. */
const [customChoice, setCustomChoice] = createSignal<{ preset?: CustomPreset } | null>(null);
const [apiKey, setAPIKey] = createSignal("");
/** `null` when the identity arrived from elsewhere while the page waited. */
const [origin, setOrigin] = createSignal<Origin | null>(null);
const [joining, setJoining] = createSignal<Joining | null>(null);
let busy = false;
/** A subscription sign-in is waiting on the browser. */
let signingIn = false;

export function Onboarding() {

  /** The bot the CLI created with the identity, or the first bot in the roster. */
  const firstBot = createMemo(() => {
    track.roster();
    return store.bots.find((bot) => bot.name === "Chef") ?? store.bots[0];
  });

  /** One that arrived from elsewhere was paired, unless this computer holds the identity. */
  const doneOrigin = createMemo((): Origin => {
    track.connection();
    return origin() ?? (store.isIdentityDevice ? "created" : "paired");
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
      if (step() !== "welcome" && step() !== "restore" && step() !== "pair") return;
      setOrigin(null);
      go("done");
    }),
  );
  // Return presses the page's default button while nothing on it has the keyboard, as a window's
  // default button answers Return wherever the keyboard is.
  onSettled(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Enter" || event.isComposing || event.defaultPrevented || event.target !== document.body || hasSheet()) return;
      const button = document.querySelector<HTMLButtonElement>(".onboarding .button.primary:not(:disabled)");
      if (!button) return;
      event.preventDefault();
      button.click();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const presentError = (message: string) => void alert({ message: hostInfo().name, informative: message, style: "warning", buttons: [{ title: L("OK") }] });

  const createIdentity = async () => {
    if (busy) return;
    if (store.isMock) {
      const { backupPhrase } = await import("../model/mock");
      setPhrase(backupPhrase);
      setOrigin("created");
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
      setOrigin("created");
      go("create");
    } catch (error) {
      presentError(errorText(error));
    } finally {
      busy = false;
    }
  };

  /** A computer that joined an account gets its credentials with its first pull from the relay,
   * so the provider step shows only when the account has none. */
  const continueAfterJoining = async () => {
    setJoining("syncing");
    setStatus({ text: L("Syncing the account from the relay…"), color: "var(--label-2)" });
    go((await store.accountHasProvider()) ? "done" : "provider");
  };

  const join = async (text: string, joined: Origin, waiting: string, run: (text: string) => Promise<void>) => {
    const trimmed = text.trim();
    if (busy || trimmed === "") return;
    if (store.isMock) {
      setOrigin(joined);
      go("provider");
      return;
    }
    busy = true;
    setJoining(joined === "paired" ? "pairing" : "restoring");
    setStatus({ text: waiting, color: "var(--label-2)" });
    try {
      await run(trimmed);
      setOrigin(joined);
      await continueAfterJoining();
    } catch (error) {
      // Back cancelled the pairing and left the step.
      if (step() === "restore" || step() === "pair") setStatus({ text: errorText(error), color: "var(--red)" });
    } finally {
      busy = false;
      setJoining(null);
    }
  };

  /** Back from a pairing that waits on the other Device stops the wait in the CLI too. */
  const leaveJoin = () => {
    if (joining() === "pairing") store.abortPairing();
    go("welcome");
  };

  const connectProvider = async () => {
    if (busy) return;
    const choice = customChoice();
    if (choice) {
      setUpCustomProvider(choice.preset);
      return;
    }
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
      else {
        signingIn = true;
        await store.connectSignIn(kind);
      }
      go("done");
    } catch (error) {
      // Skipped while the browser waited, the page has moved on; the bot step and the provider
      // step both show the rows the error belongs under.
      if (step() !== "done") setStatus({ text: errorText(error), color: "var(--red)" });
    } finally {
      busy = false;
      signingIn = false;
    }
  };

  /** Opens the custom provider sheet over onboarding; once the provider is saved, the first bot
   * runs with it and onboarding is done. Cancel leaves onboarding where it was. */
  const setUpCustomProvider = (preset: CustomPreset | undefined) => {
    void presentCustomProvider(undefined, {
      preset,
      onSave: (kind) => {
        if (step() === "done") return;
        const bot = firstBot();
        if (bot && bot.provider !== kind) store.updateBotProfile(bot.id, bot.name, undefined, kind);
        go("done");
      },
    });
  };

  /** Skip for Now leaves the provider for later, a sign-in still waiting on the browser included,
   * so finishing it there afterwards connects nothing. */
  const skipProvider = () => {
    if (signingIn) store.cancelSignIn();
    go("done");
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
    // The bot runs with the provider chosen here, not the CLI's default; a custom one is set once
    // its sheet saves it.
    const provider = customChoice() ? bot.provider : providerKind();
    if (name !== bot.name || description !== bot.description || provider !== bot.provider) store.updateBotProfile(bot.id, name, description, provider);
    void connectProvider();
  };

  const providerRows = () => (
    <ProviderRows
      kind={providerKind}
      setKind={(kind) => {
        // The note under the key is the new provider's, not the last one's error.
        setStatus(null);
        setCustomChoice(null);
        setProviderKind(kind);
      }}
      custom={customChoice}
      setCustom={(choice) => {
        setStatus(null);
        setCustomChoice(choice);
      }}
      apiKey={apiKey}
      setAPIKey={setAPIKey}
      status={status}
    />
  );
  /** The Continue button of a step with a credential: signing in says so, a key must be typed, and
   * a custom provider is set up next. */
  const credentialButton = (run: () => void) => {
    const choice = customChoice();
    if (choice) return { title: choice.preset ? L("Set Up %@…", choice.preset.name) : L("Set Up…"), enabled: true, run };
    return {
      title: usesAPIKey(providerKind()) ? L("Continue") : L("Sign in with %@", providerName(providerKind())),
      enabled: !usesAPIKey(providerKind()) || apiKey().trim() !== "",
      run,
    };
  };

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
                  busy={joining() !== null}
                  canGoBack={joining() === null}
                  onBack={leaveJoin}
                  onSubmit={(text) => void join(text, "restored", L("Re-deriving keys and unwrapping the account key…"), (phraseText) => store.restoreIdentity(phraseText))}
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
                  busy={joining() !== null}
                  canGoBack={joining() === null || joining() === "pairing"}
                  onBack={leaveJoin}
                  onSubmit={(text) => void join(text, "paired", L("Waiting for the other Device to wrap the account key…"), (code) => store.acceptPairing(code))}
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
                  back={{ title: L("Skip for Now"), run: skipProvider }}
                  next={credentialButton(() => void connectProvider())}
                >
                  <div class="onboarding-card">
                    <div class="form-grid">{providerRows()}</div>
                  </div>
                </StepLayout>
              );
            case "done":
              return <Done origin={doneOrigin()} botName={firstBot()?.name} />;
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
        <img class="onboarding-icon" src={hostInfo().isDevelopment ? devIconURL : iconURL} width={96} height={96} alt="" draggable="false" />
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
 * Return presses Continue, as a default button does. While the step is `busy`, a spinner turns
 * beside the buttons. */
function StepLayout(props: { title: string; subtitle: string; back?: StepButton; next: StepButton; busy?: boolean; children: JSX.Element }) {
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
        <Show when={props.busy}>
          <Spinner size={16} />
        </Show>
        <Show when={props.back}>
          {(back) => (
            <Button large disabled={back().enabled === false} onClick={() => back().run()}>
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
  /** The restore or the pairing is under way: the field and the action wait. */
  busy: boolean;
  canGoBack: boolean;
  onBack: () => void;
  onSubmit: (text: string) => void;
}) {
  const [text, setText] = createSignal("");
  let field: HTMLInputElement | undefined;
  onSettled(() => field?.focus());
  return (
    <StepLayout
      title={props.title}
      subtitle={props.subtitle}
      back={{ title: L("Back"), enabled: props.canGoBack, run: props.onBack }}
      next={{ title: props.action, enabled: !props.busy, run: () => props.onSubmit(text()) }}
      busy={props.busy}
    >
      <div class="onboarding-join">
        <input
          ref={(element) => (field = element)}
          class="text-field mono onboarding-wide"
          placeholder={props.placeholder}
          spellcheck="false"
          autocomplete="off"
          disabled={props.busy}
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

/** The provider picker and the credential for the chosen provider: a key field, what signing in
 * does, or that a custom provider's server is set up next. Switching providers swaps only the
 * credential row. */
function ProviderRows(props: {
  kind: Accessor<BuiltInProviderKind>;
  setKind: (kind: BuiltInProviderKind) => void;
  custom: Accessor<{ preset?: CustomPreset } | null>;
  setCustom: (choice: { preset?: CustomPreset }) => void;
  apiKey: Accessor<string>;
  setAPIKey: (key: string) => void;
  status: Accessor<{ text: string; color: string } | null>;
}) {
  const kind = props.kind;
  const note = () =>
    props.status() ?? {
      text: props.custom()
        ? L("Lorca checks the server, then shares it with your paired Devices, encrypted.")
        : usesAPIKey(kind())
          ? L("The key is checked against %@ and shared with your paired Devices, encrypted.", providerName(kind()))
          : L("Tokens from the sign-in are shared with your paired Devices, encrypted."),
      color: "var(--label-3)",
    };
  // The built-in providers, then any other server: for an account whose models run on a gateway or
  // its own computers.
  const options = () => [
    ...providerKinds.map((each) => ({ value: each as string, label: providerName(each) })),
    ...customPresets.map((preset, index) => ({ value: `preset:${preset.name}`, label: preset.name, separated: index === 0 })),
    { value: "other", label: L("Other Server…") },
  ];
  const picked = () => {
    const choice = props.custom();
    if (!choice) return kind() as string;
    return choice.preset ? `preset:${choice.preset.name}` : "other";
  };
  const pick = (value: string) => {
    props.setAPIKey("");
    if (value === "other") props.setCustom({});
    else if (value.startsWith("preset:")) props.setCustom({ preset: customPresets.find((preset) => `preset:${preset.name}` === value) });
    else props.setKind(value as BuiltInProviderKind);
  };
  return (
    <>
      <FormRow label={L("Provider")}>
        <PopUpButton class="fill" options={options()} value={picked()} onChange={pick} />
      </FormRow>
      <FormRow label={props.custom() ? L("Server") : usesAPIKey(kind()) ? L("API key") : L("Account")} top={props.custom() !== null || !usesAPIKey(kind())}>
        <Show
          when={!props.custom()}
          fallback={
            <div class="form-text">
              {L("Any server that speaks OpenAI’s or Anthropic’s API, such as a gateway or a model server on your network. Set up its address, key, and models next.")}
            </div>
          }
        >
          <Show
            when={usesAPIKey(kind())}
            fallback={<div class="form-text">{L("Your browser opens a %@ sign-in when you continue. %@", providerName(kind()), signInRequirement(kind()))}</div>}
          >
            <input
              class="text-field mono"
              type="password"
              placeholder={keyPlaceholder(kind())}
              aria-label={L("API key")}
              spellcheck="false"
              autocomplete="off"
              value={props.apiKey()}
              onInput={(event) => props.setAPIKey(event.currentTarget.value)}
            />
          </Show>
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
            <input class="text-field" placeholder={L("Name")} value={name()} spellcheck="false" onInput={(event) => setName(event.currentTarget.value)} />
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

function Done(props: { origin: Origin; botName: string | undefined }) {
  let button: HTMLButtonElement | undefined;
  onSettled(() => button?.focus());
  const title = () => {
    switch (props.origin) {
      case "created":
        return L("This computer is your first Device");
      case "restored":
        return L("Your identity is restored");
      case "paired":
        return L("This computer is paired");
    }
  };
  const lede = () => {
    switch (props.origin) {
      case "created":
        return props.botName
          ? L("%@ is ready to talk to. Pair another Device any time from the File menu.", props.botName)
          : L("Bots you create here run on this computer with your account's provider credentials. Pair another Device any time from the File menu.");
      case "restored":
      case "paired":
        return L("Your bots and chats sync to this computer, and it can run bots too. Pair another Device any time from the File menu.");
    }
  };
  return (
    <div class="onboarding-center">
      <div class="onboarding-column">
        <span class="onboarding-done-icon">
          <Icon name="checkmark.circle.fill" size={56} strokeWidth={1.6} />
        </span>
        <div class="onboarding-title center">{title()}</div>
        <div class="onboarding-lede">{lede()}</div>
        <Button kind="primary" large class="onboarding-open" ref={(element) => (button = element)} onClick={() => void host.finishOnboarding()}>
          {L("Open Lorca")}
        </Button>
      </div>
    </div>
  );
}
