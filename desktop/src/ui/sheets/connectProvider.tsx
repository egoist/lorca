// Connects a provider for the account, after the macOS app's ConnectProviderViewController.
// API-key providers take a pasted key; ChatGPT and Grok sign in through the browser, driven by the
// CLI. The credential reaches every paired Device encrypted with the account key.

import { createSignal, Show } from "solid-js";
import { L } from "../../l10n";
import { defaultBaseURL, keyPlaceholder, providerName, signInRequirement, usesAPIKey, type ProviderKind } from "../../model/models";
import { errorText, store } from "../../model/store";
import { Button, HoverButton, Spinner } from "../controls";
import { alert, presentSheet, Sheet } from "../overlay";

let fetching = false;

/** Fetches the saved credential before presenting, so the first frame is filled in and masked,
 * with no loading row changing the sheet's size. */
export async function presentConnectProvider(kind: ProviderKind, options: { baseURL?: string; onDone?: () => void } = {}): Promise<void> {
  if (fetching) return;
  fetching = true;
  let credential: { api_key?: string | null; base_url?: string | null } | null = null;
  try {
    if (usesAPIKey(kind) && store.credential(kind)?.isConnected) credential = await store.providerAPIKey(kind);
  } catch (error) {
    fetching = false;
    void alert({ message: errorText(error) });
    return;
  }
  fetching = false;
  presentSheet((dismiss) => <ConnectProviderSheet kind={kind} credential={credential} baseURL={options.baseURL} onDone={options.onDone ?? (() => {})} dismiss={dismiss} />);
}

function ConnectProviderSheet(props: {
  kind: ProviderKind;
  credential: { api_key?: string | null; base_url?: string | null } | null;
  baseURL?: string;
  onDone: () => void;
  dismiss: () => void;
}) {
  const kind = props.kind;
  const name = providerName(kind);
  const isEditing = props.credential?.api_key != null;
  const [key, setKey] = createSignal(props.credential?.api_key ?? "");
  const [baseURL, setBaseURL] = createSignal((props.credential === null ? props.baseURL : props.credential.base_url) ?? "");
  const [revealed, setRevealed] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [spinning, setSpinning] = createSignal(false);
  const [status, setStatus] = createSignal<{ text: string; color: string } | null>(null);
  let keyField: HTMLInputElement | undefined;
  let closed = false;
  let signingIn = false;

  const canConfirm = () => !busy() && (!usesAPIKey(kind) || key().trim() !== "");

  const begin = (message: string) => {
    setBusy(true);
    setSpinning(true);
    setStatus({ text: message, color: "var(--label-2)" });
  };
  const fail = (error: unknown) => {
    setSpinning(false);
    setBusy(false);
    setStatus({ text: errorText(error), color: "var(--red)" });
  };
  const finish = () => {
    props.dismiss();
    props.onDone();
  };

  const confirm = async () => {
    if (!canConfirm()) return;
    begin(usesAPIKey(kind) ? L("Checking the key with %@…", name) : L("Waiting for the browser…"));
    try {
      if (usesAPIKey(kind)) {
        await store.connectAPIKey(kind, key().trim(), baseURL());
      } else {
        signingIn = true;
        await store.connectSignIn(kind);
        signingIn = false;
      }
      if (closed) return;
      setSpinning(false);
      setStatus({ text: L("%@ connected.", name), color: "var(--green)" });
      await new Promise((resolve) => setTimeout(resolve, 600));
      if (!closed) finish();
    } catch (error) {
      signingIn = false;
      if (!closed) fail(error);
    }
  };

  const disconnect = async () => {
    begin(L("Disconnecting…"));
    try {
      await store.disconnectProvider(kind);
      if (!closed) finish();
    } catch (error) {
      if (!closed) fail(error);
    }
  };

  const cancel = () => {
    closed = true;
    // A sign-in waiting on the browser stops with the sheet.
    if (signingIn) store.cancelSignIn();
    props.dismiss();
  };

  const toggleReveal = () => {
    const selection: [number | null, number | null] | null = keyField && document.activeElement === keyField ? [keyField.selectionStart, keyField.selectionEnd] : null;
    setRevealed(!revealed());
    if (selection && keyField) {
      const field = keyField;
      queueMicrotask(() => {
        field.focus();
        field.setSelectionRange(selection[0], selection[1]);
      });
    }
  };

  const flow = kind === "chatgpt" ? L("Sign-in uses the same OAuth flow as the Codex CLI.") : L("Sign-in uses the same OAuth flow as xAI's Grok CLI.");
  return (
    <Sheet
      title={isEditing ? name : L("Connect %@", name)}
      subtitle={
        usesAPIKey(kind)
          ? L("Encrypted and shared with your paired Devices.")
          : L("Your browser opens a %@ sign-in. The tokens are shared with your paired Devices, encrypted with your account key; the relay cannot read them.", name)
      }
      width={420}
      confirm={usesAPIKey(kind) ? (isEditing ? L("Save") : L("Connect")) : L("Sign in with %@…", name)}
      confirmDisabled={!canConfirm()}
      onConfirm={() => void confirm()}
      onCancel={cancel}
      leading={
        isEditing && usesAPIKey(kind) ? (
          <Button kind="destructive" disabled={busy()} onClick={() => void disconnect()}>
            {L("Disconnect")}
          </Button>
        ) : undefined
      }
    >
      <Show
        when={usesAPIKey(kind)}
        fallback={<div class="sheet-note">{`${flow} ${signInRequirement(kind)}`}</div>}
      >
        <label class="field-stack">
          <span class="field-label">{L("API key")}</span>
          <span class="key-field">
            <input
              ref={(element) => (keyField = element)}
              class="text-field mono"
              type={revealed() ? "text" : "password"}
              value={key()}
              placeholder={keyPlaceholder(kind)}
              aria-label={L("API key")}
              disabled={busy()}
              spellcheck={false}
              autocomplete="off"
              onInput={(event) => setKey(event.currentTarget.value)}
            />
            <HoverButton
              class="key-reveal"
              symbol={revealed() ? "eye.slash" : "eye"}
              size={14}
              tooltip={revealed() ? L("Hide API key") : L("Show API key")}
              disabled={busy()}
              onMouseDown={(event) => event.preventDefault()}
              onClick={toggleReveal}
            />
          </span>
        </label>
        <div class="field-group">
          <label class="field-stack">
            <span class="field-label">{L("API base URL")}</span>
            <input
              class="text-field mono"
              value={baseURL()}
              placeholder={defaultBaseURL(kind)}
              disabled={busy()}
              spellcheck={false}
              autocomplete="off"
              onInput={(event) => setBaseURL(event.currentTarget.value)}
            />
          </label>
          <div class="field-note">{L("Leave empty to use %@’s API.", name)}</div>
        </div>
      </Show>
      <Show when={status()}>
        {(current) => (
          <div class="status-line">
            <Show when={spinning()}>
              <Spinner size={14} />
            </Show>
            <span style={{ color: current().color }}>{current().text}</span>
          </div>
        )}
      </Show>
    </Sheet>
  );
}
