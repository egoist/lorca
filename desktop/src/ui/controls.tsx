// The app's controls, after the macOS app's Controls.swift: borderless symbol buttons that fill
// under the pointer, push buttons, switches, pop-up buttons whose menu is the system's, fields,
// the search field, and a button that says it copied.

import { createSignal, onSettled, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { L } from "../l10n";
import { host } from "../host";
import { Icon } from "./icons";
import { popupMenu, separator, type MenuEntry } from "./menu";

/** Borderless symbol button that fills a rounded rect under the pointer. With `title`, the symbol
 * and the word, as wide as they need; `trailingSymbol` follows a word that leads somewhere. */
export function HoverButton(props: {
  symbol?: string;
  size?: number;
  title?: string;
  trailingSymbol?: string;
  tooltip?: string;
  label?: string;
  disabled?: boolean;
  hidden?: boolean;
  active?: boolean;
  class?: string;
  style?: JSX.CSSProperties;
  onClick?: (event: MouseEvent) => void;
  onMouseDown?: (event: MouseEvent) => void;
  ref?: (element: HTMLButtonElement) => void;
}) {
  return (
    <button
      ref={props.ref}
      class={["hover-button", props.class, { labelled: !!props.title, active: !!props.active }]}
      style={props.style}
      title={props.tooltip}
      aria-label={props.label ?? props.tooltip ?? props.title}
      disabled={props.disabled}
      hidden={props.hidden}
      onClick={(event) => props.onClick?.(event)}
      onMouseDown={(event) => props.onMouseDown?.(event)}
    >
      <Show when={props.symbol}>{(symbol) => <Icon name={symbol()} size={props.size ?? 16} strokeWidth={1.75} />}</Show>
      <Show when={props.title}>
        <span>{props.title}</span>
      </Show>
      <Show when={props.trailingSymbol}>{(symbol) => <Icon name={symbol()} size={10} strokeWidth={2.5} />}</Show>
    </button>
  );
}

/** A push button: the rounded bezel, the accent-filled default, or a destructive one. */
export function Button(props: {
  children: JSX.Element;
  kind?: "default" | "primary" | "destructive";
  small?: boolean;
  large?: boolean;
  disabled?: boolean;
  hidden?: boolean;
  tooltip?: string;
  class?: string;
  type?: "button" | "submit";
  onClick?: (event: MouseEvent) => void;
  ref?: (element: HTMLButtonElement) => void;
}) {
  return (
    <button
      ref={props.ref}
      type={props.type ?? "button"}
      class={["button", props.kind ?? "default", props.class, { small: !!props.small, large: !!props.large }]}
      disabled={props.disabled}
      hidden={props.hidden}
      title={props.tooltip}
      onClick={(event) => props.onClick?.(event)}
    >
      {props.children}
    </button>
  );
}

/** A text button in the accent color, as the rows' Edit… and Change do. */
export function LinkButton(props: { children: JSX.Element; disabled?: boolean; tooltip?: string; onClick?: (event: MouseEvent) => void; class?: string }) {
  return (
    <button class={["link-button", props.class]} disabled={props.disabled} title={props.tooltip} onClick={(event) => props.onClick?.(event)}>
      {props.children}
    </button>
  );
}

export function Switch(props: { checked: boolean; disabled?: boolean; small?: boolean; tooltip?: string; label?: string; onChange: (checked: boolean) => void }) {
  return (
    <button
      role="switch"
      aria-checked={props.checked ? "true" : "false"}
      aria-label={props.label}
      title={props.tooltip}
      class={["switch", { on: props.checked, small: !!props.small }]}
      disabled={props.disabled}
      onClick={(event) => {
        event.stopPropagation();
        props.onChange(!props.checked);
      }}
    >
      <span class="switch-knob" />
    </button>
  );
}

export interface PopUpOption<T> {
  value: T;
  label: string;
  /** A separator before this option. */
  separated?: boolean;
  /** A symbol the button shows before the picked option's label, as a Device's. */
  symbol?: string;
}

/** A pop-up button: the choice as text; the menu is the system's, with a check on the choice.
 * `settings` is System Settings' look (text, then chevrons on a small platter); `bordered` is a
 * form control's; `plain` is text with chevrons, for a toolbar. */
export function PopUpButton<T>(props: {
  options: PopUpOption<T>[];
  value: T;
  onChange: (value: T) => void;
  style?: "settings" | "bordered" | "plain";
  disabled?: boolean;
  tooltip?: string;
  label?: string;
  /** Items after the choices, such as Pair a Device…; picking one calls `onExtra` with its id. */
  extras?: MenuEntry[];
  onExtra?: (id: string) => void;
  class?: string;
}) {
  const current = () => props.options.find((option) => option.value === props.value);
  /** The menu is up: the platter stays pressed, as a highlighted pop-up's does. */
  const [opened, setOpened] = createSignal(false);
  const open = async (event: MouseEvent) => {
    if (props.disabled) return;
    const entries: MenuEntry[] = [];
    props.options.forEach((option, index) => {
      if (option.separated && entries.length > 0) entries.push(separator);
      entries.push({ id: String(index), label: option.label, checked: option.value === props.value });
    });
    if (props.extras?.length) entries.push(separator, ...props.extras);
    setOpened(true);
    const picked = await popupMenu(entries, event.currentTarget as HTMLElement).finally(() => setOpened(false));
    if (picked === null) return;
    const index = Number(picked);
    if (Number.isInteger(index) && props.options[index]) {
      if (props.options[index]!.value !== props.value) props.onChange(props.options[index]!.value);
    } else {
      props.onExtra?.(picked);
    }
  };
  return (
    <button
      class={["popup-button", props.style ?? "bordered", props.class, { open: opened() }]}
      disabled={props.disabled}
      title={props.tooltip}
      aria-label={props.label}
      aria-haspopup="menu"
      onMouseDown={(event) => {
        if (event.button !== 0) return;
        event.preventDefault();
        void open(event);
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " " || event.key === "ArrowDown") {
          event.preventDefault();
          void open(event as unknown as MouseEvent);
        }
      }}
    >
      <Show when={current()?.symbol}>{(symbol) => <Icon name={symbol()} size={13} strokeWidth={1.9} class="popup-symbol" />}</Show>
      <span class="popup-title truncate">{current()?.label ?? ""}</span>
      <span class="popup-chevrons">
        <Icon name="chevron.up.chevron.down" size={props.style === "settings" ? 11 : 12} strokeWidth={2.4} />
      </span>
    </button>
  );
}

export function TextField(props: {
  value: string;
  placeholder?: string;
  secure?: boolean;
  monospaced?: boolean;
  disabled?: boolean;
  readOnly?: boolean;
  autofocus?: boolean;
  plain?: boolean;
  class?: string;
  label?: string;
  spellcheck?: boolean;
  onInput?: (value: string) => void;
  onCommit?: (value: string) => void;
  onEnter?: () => void;
  onKeyDown?: (event: KeyboardEvent) => void;
  ref?: (element: HTMLInputElement) => void;
}) {
  let input: HTMLInputElement | undefined;
  onSettled(() => {
    // Taking the keyboard selects what is there, as a field becoming first responder does, so
    // typing replaces a name being renamed.
    if (!props.autofocus || !input) return;
    input.focus();
    input.select();
  });
  return (
    <input
      ref={(element) => {
        input = element;
        props.ref?.(element);
      }}
      class={["text-field", props.class, { mono: !!props.monospaced, plain: !!props.plain }]}
      type={props.secure ? "password" : "text"}
      value={props.value}
      placeholder={props.placeholder}
      disabled={props.disabled}
      readonly={props.readOnly}
      aria-label={props.label}
      spellcheck={props.spellcheck ? "true" : "false"}
      autocomplete="off"
      autocapitalize="off"
      onInput={(event) => props.onInput?.(event.currentTarget.value)}
      onChange={(event) => props.onCommit?.(event.currentTarget.value)}
      onKeyDown={(event) => {
        props.onKeyDown?.(event);
        if (event.defaultPrevented) return;
        if (event.key === "Enter" && !event.isComposing) {
          props.onEnter?.();
          if (props.onCommit) event.currentTarget.blur();
        }
      }}
    />
  );
}

export function TextArea(props: {
  value: string;
  placeholder?: string;
  rows?: number;
  monospaced?: boolean;
  disabled?: boolean;
  readOnly?: boolean;
  autofocus?: boolean;
  class?: string;
  label?: string;
  onInput?: (value: string) => void;
  onKeyDown?: (event: KeyboardEvent) => void;
  ref?: (element: HTMLTextAreaElement) => void;
  /** Grows with its text from `rows` lines, as a wrapping text field does, and the sheet with it. */
  grows?: boolean;
}) {
  let area: HTMLTextAreaElement | undefined;
  const fit = () => {
    if (!props.grows || !area) return;
    area.style.height = "auto";
    area.style.height = `${area.scrollHeight}px`;
  };
  onSettled(() => {
    if (props.autofocus) area?.focus();
    fit();
  });
  return (
    <textarea
      ref={(element) => {
        area = element;
        props.ref?.(element);
      }}
      class={["text-area", props.class, { mono: !!props.monospaced }]}
      value={props.value}
      placeholder={props.placeholder}
      rows={props.rows ?? 4}
      disabled={props.disabled}
      readonly={props.readOnly}
      aria-label={props.label}
      spellcheck="false"
      onInput={(event) => {
        props.onInput?.(event.currentTarget.value);
        fit();
      }}
      onKeyDown={(event) => props.onKeyDown?.(event)}
    />
  );
}

/** A capsule search field with a magnifier and a clear button. `onActivate` makes it the way into a
 * search elsewhere: a click calls it and the field never takes the keyboard. */
export function SearchField(props: {
  value?: string;
  placeholder?: string;
  onInput?: (value: string) => void;
  onActivate?: () => void;
  onKeyDown?: (event: KeyboardEvent) => void;
  ref?: (element: HTMLInputElement) => void;
  class?: string;
  /** Escape clears the text first, as NSSearchField's does; off where Escape closes what holds
   * the field wherever the keyboard is, as the marketplace. */
  clearsOnEscape?: boolean;
}) {
  return (
    <div class={["search-field", props.class]} onMouseDown={(event) => {
      if (!props.onActivate) return;
      event.preventDefault();
      props.onActivate();
    }}>
      <Icon name="magnifyingglass" size={13} strokeWidth={2} class="search-glyph" />
      <input
        ref={props.ref}
        type="text"
        value={props.value ?? ""}
        placeholder={props.placeholder ?? L("Search")}
        spellcheck="false"
        autocomplete="off"
        readonly={!!props.onActivate}
        tabindex={props.onActivate ? -1 : 0}
        onInput={(event) => props.onInput?.(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === "Escape" && (props.value ?? "") !== "" && props.clearsOnEscape !== false) {
            event.preventDefault();
            event.stopPropagation();
            props.onInput?.("");
            return;
          }
          props.onKeyDown?.(event);
        }}
      />
      <Show when={(props.value ?? "") !== ""}>
        <button class="search-clear" tabindex={-1} aria-label={L("Clear")} onMouseDown={(event) => event.preventDefault()} onClick={() => props.onInput?.("")}>
          <Icon name="xmark" size={9} strokeWidth={3} />
        </button>
      </Show>
    </div>
  );
}

export function Spinner(props: { size?: number; class?: string }) {
  return <span class={["spinner", props.class]} style={{ width: `${props.size ?? 16}px`, height: `${props.size ?? 16}px` }} role="progressbar" />;
}

/** A button that copies `text` and says so for a moment: its symbol turns into a green check, its
 * title into Copied. */
export function CopyButton(props: {
  text: () => string;
  title?: string;
  symbol?: string;
  tooltip?: string;
  class?: string;
  bordered?: boolean;
  onCopied?: () => void;
}) {
  const [copied, setCopied] = createSignal(false);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const copy = () => {
    void host.copyText(props.text());
    setCopied(true);
    clearTimeout(timer);
    timer = setTimeout(() => setCopied(false), 1500);
    props.onCopied?.();
  };
  return (
    <button
      class={[props.bordered ? "button default" : "copy-button", props.class, { copied: copied() }]}
      title={copied() ? L("Copied") : props.tooltip}
      aria-label={props.tooltip ?? props.title ?? L("Copy")}
      onClick={copy}
    >
      <Show when={copied() || props.symbol}>
        <Icon name={copied() ? "checkmark" : (props.symbol ?? "doc.on.doc")} size={13} strokeWidth={2} />
      </Show>
      <Show when={props.title}>
        <span>{copied() ? L("Copied") : props.title}</span>
      </Show>
    </button>
  );
}

/** A segmented control: one choice of a few, each a segment. */
export function Segmented<T>(props: { options: { value: T; label: string }[]; value: T; onChange: (value: T) => void; label?: string; class?: string }) {
  return (
    <div class={["segmented", props.class]} role="radiogroup" aria-label={props.label}>
      {props.options.map((option) => (
        <button
          role="radio"
          aria-checked={option.value === props.value ? "true" : "false"}
          class={["segment", { selected: option.value === props.value }]}
          onClick={() => {
            if (option.value !== props.value) props.onChange(option.value);
          }}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
