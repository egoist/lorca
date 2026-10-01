// A provider's API key, after the macOS app's APIKeyField: masked by default, with an eye button
// inside the field that shows or hides the key and keeps the selection while it switches.

import { createSignal, onSettled } from "solid-js";
import { L } from "../../l10n";
import { HoverButton } from "../controls";

/** `autofocus` makes it the field a sheet starts in. */
export function APIKeyField(props: { value: string; placeholder: string; disabled?: boolean; autofocus?: boolean; onInput: (value: string) => void }) {
  const [revealed, setRevealed] = createSignal(false);
  let field: HTMLInputElement | undefined;
  onSettled(() => {
    if (props.autofocus) field?.focus();
  });

  const toggleReveal = () => {
    const selection: [number | null, number | null] | null = field && document.activeElement === field ? [field.selectionStart, field.selectionEnd] : null;
    setRevealed(!revealed());
    if (selection && field) {
      const input = field;
      queueMicrotask(() => {
        input.focus();
        input.setSelectionRange(selection[0], selection[1]);
      });
    }
  };

  return (
    <span class="key-field">
      <input
        ref={(element) => (field = element)}
        class="text-field mono"
        type={revealed() ? "text" : "password"}
        value={props.value}
        placeholder={props.placeholder}
        aria-label={L("API key")}
        disabled={props.disabled}
        spellcheck="false"
        autocomplete="off"
        onInput={(event) => props.onInput(event.currentTarget.value)}
      />
      <HoverButton
        class="key-reveal"
        symbol={revealed() ? "eye.slash" : "eye"}
        size={14}
        tooltip={revealed() ? L("Hide API key") : L("Show API key")}
        disabled={props.disabled}
        onMouseDown={(event) => event.preventDefault()}
        onClick={toggleReveal}
      />
    </span>
  );
}
