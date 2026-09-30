// Pairing another Device, after the macOS app's PairingSheetViewController: a pairing string for the
// other Device to scan or paste. The CLI runs the handshake and wraps the account key to the joining
// machine; this sheet only polls for the outcome. Done keeps the code good for ten minutes; Cancel
// retires it.

import qrcode from "qrcode-generator";
import { createSignal, onSettled, Show } from "solid-js";
import { L } from "../../l10n";
import { errorText, store } from "../../model/store";
import { CopyButton, Spinner } from "../controls";
import { Icon } from "../icons";
import { presentSheet, Sheet } from "../overlay";

/** A QR code as crisp squares, one path, dark on a white field. */
export function QRCode(props: { text: string; size: number }) {
  const path = () => {
    const code = qrcode(0, "M");
    code.addData(props.text);
    code.make();
    const count = code.getModuleCount();
    let d = "";
    for (let row = 0; row < count; row++) {
      for (let column = 0; column < count; column++) {
        if (code.isDark(row, column)) d += `M${column} ${row}h1v1h-1z`;
      }
    }
    return { d, count };
  };
  return (
    <svg class="qr-code" width={props.size} height={props.size} viewBox={`0 0 ${path().count} ${path().count}`} shape-rendering="crispEdges" role="img">
      <path d={path().d} fill="#000" />
    </svg>
  );
}

export function presentPairing(): void {
  presentSheet((dismiss) => <PairingSheet dismiss={dismiss} />);
}

function PairingSheet(props: { dismiss: () => void }) {
  const [pairingString, setPairingString] = createSignal("");
  const [paired, setPaired] = createSignal<string | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  let nonce: string | null = null;
  let stopped = false;

  const begin = async () => {
    try {
      const started = await store.startPairing();
      if (stopped) return;
      nonce = started.nonce;
      setPairingString(started.pairing_string);
      if (store.isMock) return;
      while (!stopped) {
        await new Promise((resolve) => setTimeout(resolve, 1500));
        if (stopped) return;
        const status = await store.pairingStatus(started.nonce);
        if (stopped) return;
        if (status.state === "completed") {
          // The code is spent: cover it, retire the copy button, and say who joined.
          setPaired(status.device?.name ?? L("the other Device"));
          return;
        }
        if (status.state === "failed") throw new Error(status.error ?? L("Pairing failed"));
      }
    } catch (failure) {
      if (!stopped) setError(errorText(failure));
    }
  };
  onSettled(() => {
    void begin();
    return () => {
      stopped = true;
    };
  });

  /** Done keeps the code good: the CLI goes on waiting for ten minutes, so copying the code and
   * closing this sheet before pasting it on the phone is fine. */
  const done = () => {
    stopped = true;
    props.dismiss();
  };
  /** Cancel retires the code: the CLI stops waiting and the relay drops the mailbox, so a Device that
   * pastes it afterwards is told at once. */
  const cancel = () => {
    stopped = true;
    if (nonce !== null && paired() === null) store.cancelPairing(nonce);
    props.dismiss();
  };

  return (
    <Sheet
      title={L("Pair a Device")}
      subtitle={L(
        "On the other Device, choose Pair in onboarding (or run `lorca pair <code>`) and paste this code. The Devices run a handshake; the relay only carries ciphertext.",
      )}
      width={400}
      confirm={L("Done")}
      onConfirm={done}
      onCancel={cancel}
      class="pairing-sheet"
    >
      <div class="qr-frame">
        <Show when={pairingString() !== ""} fallback={<div class="qr-placeholder" />}>
          <QRCode text={pairingString()} size={180} />
        </Show>
        <Show when={paired() !== null}>
          <div class="qr-paired">
            <Icon name="checkmark.circle.fill" size={64} strokeWidth={1.5} label={L("Paired")} />
          </div>
        </Show>
      </div>
      <div class="pairing-code">
        <Show
          when={paired()}
          fallback={
            <>
              <span class="pairing-code-text selectable">{pairingString() || L("Asking the CLI for a pairing code…")}</span>
              <Show when={pairingString() !== ""}>
                <CopyButton text={pairingString} symbol="doc.on.doc" tooltip={L("Copy pairing string")} />
              </Show>
            </>
          }
        >
          {(device) => <span class="pairing-code-done">{L("Paired with %@. This code is used up; pair another Device with a fresh one.", device())}</span>}
        </Show>
      </div>
      <div class="status-line">
        <Show when={paired() === null && error() === null}>
          <Spinner size={14} />
        </Show>
        <span class={["pairing-status", { success: paired() !== null, failure: error() !== null }]}>
          {error() ??
            (paired() !== null
              ? L("Paired. The account key is wrapped to that machine.")
              : L("Waiting for the other Device… Done keeps this code good for ten minutes; Cancel retires it."))}
        </span>
      </div>
    </Sheet>
  );
}
