// Sheets, alerts, and popovers, after AppKit's: a sheet is modal to the window and stacks over
// the one before it; an alert is a sheet with a message and buttons; a popover hangs off the
// control that opened it and closes when the user clicks elsewhere.

import { Portal } from "@solidjs/web";
import { createEffect, For, onSettled, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { hostInfo } from "../host";
import { L } from "../l10n";
import { box } from "./box";
import { Button } from "./controls";
import { Icon } from "./icons";
import devIconURL from "./images/icon-dev.png";
import iconURL from "./images/icon.png";
import { Markdown } from "./markdown";

// MARK: - Sheets

interface SheetEntry {
  id: number;
  render: (dismiss: () => void) => JSX.Element;
  onDismiss?: () => void;
  /** What had the keyboard when the sheet came up. */
  focus: Element | null;
}

const sheets = box<SheetEntry[]>([]);
let nextSheet = 1;

export interface SheetHandle {
  dismiss(): void;
}

/** The sheet in front: the element a sheet's content renders in its frame. */
function frontSheet(): HTMLElement | null {
  const frames = document.querySelectorAll(".sheet-frame");
  const sheet = frames[frames.length - 1]?.firstElementChild;
  return sheet instanceof HTMLElement ? sheet : null;
}

/** Takes a sheet away and hands the keyboard back to what had it before, as a sheet's window
 * closing does on the Mac, once the window behind takes keys again; unless something took the
 * keyboard on the way, as a chat the sheet opened. */
function removeSheet(id: number): void {
  const entry = sheets.get().find((sheet) => sheet.id === id);
  if (!entry) return;
  sheets.set(sheets.get().filter((sheet) => sheet.id !== id));
  entry.onDismiss?.();
  setTimeout(() => {
    if (document.activeElement && document.activeElement !== document.body) return;
    const previous = entry.focus;
    if (previous instanceof HTMLElement && previous.isConnected && !previous.closest("[inert]")) previous.focus({ preventScroll: true });
    else frontSheet()?.focus();
  });
}

/** Presents a sheet over the window. `render` gets the function that closes it. */
export function presentSheet(render: (dismiss: () => void) => JSX.Element, onDismiss?: () => void): SheetHandle {
  const id = nextSheet++;
  sheets.set([...sheets.get(), { id, render, onDismiss, focus: document.activeElement }]);
  return { dismiss: () => removeSheet(id) };
}

/** A click on a sheet's button leaves the keyboard where it was, so Return still presses the
 * default button and a field keeps its caret, as AppKit's buttons do. */
const keepFocus = (event: MouseEvent) => {
  if ((event.target as Element).closest("button")) event.preventDefault();
};

/** Whether a sheet is up, so the window's commands leave the keyboard to it. */
export function hasSheet(): boolean {
  return sheets.get().length > 0;
}

export function readHasSheet(): boolean {
  return sheets.read().length > 0;
}

export function SheetHost() {
  // The window behind a sheet takes no clicks, keys, or Tab, as a window with a sheet on the Mac;
  // nor does a sheet behind another.
  createEffect(
    () => sheets.read().length > 0,
    (up) => {
      const root = document.getElementById("root");
      if (root) root.inert = up;
    },
  );
  onSettled(() => {
    // A sheet is the key window. A key typed while nothing has the keyboard, once the control that
    // had it went away with a page or turned into a spinner, goes to the sheet in front, which takes
    // the keyboard back for the keys after it.
    const onKey = (event: KeyboardEvent) => {
      const sheet = frontSheet();
      if (!sheet || event.target !== document.body) return;
      event.stopImmediatePropagation();
      sheet.focus();
      if (!sheet.dispatchEvent(new KeyboardEvent(event.type, event))) event.preventDefault();
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  });
  return (
    <For each={sheets.read()}>
      {(entry, index) => {
        const covered = () => index() < sheets.read().length - 1;
        return (
          <Portal>
            <div class={["sheet-scrim", { covered: covered() }]} inert={covered()}>
              <div class="sheet-frame" role="dialog" aria-modal="true" onMouseDown={keepFocus}>
                {entry.render(() => removeSheet(entry.id))}
              </div>
            </div>
          </Portal>
        );
      }}
    </For>
  );
}

/** Shared sheet chrome: a title, an optional subtitle, the content, and a trailing row of buttons.
 * Return confirms and Escape cancels, as a sheet's default and cancel buttons do. */
export function Sheet(props: {
  title: string;
  subtitle?: string;
  width?: number;
  children?: JSX.Element;
  confirm?: string;
  /** null leaves only the confirm button, which then answers Escape as well. */
  cancel?: string | null;
  confirmKind?: "primary" | "destructive" | "default";
  confirmDisabled?: boolean;
  leading?: JSX.Element;
  onConfirm?: () => void;
  onCancel: () => void;
  /** Return is the content's own, as in a multi-line editor. */
  returnInContent?: boolean;
  class?: string;
}) {
  let element: HTMLDivElement | undefined;
  onSettled(() => {
    // The first field takes the keyboard, as a sheet's initial first responder does; with none, the
    // sheet itself does, so keys stop going to the window behind it.
    const field = element?.querySelector<HTMLElement>("[data-autofocus], input:not([readonly]), textarea:not([readonly])");
    if (element && !element.contains(document.activeElement)) (field ?? element).focus();
  });
  const onKeyDown = (event: KeyboardEvent) => {
    if (event.defaultPrevented || event.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      if (props.cancel === null) props.onConfirm?.();
      else props.onCancel();
      return;
    }
    const target = event.target as HTMLElement;
    const multiline = target.tagName === "TEXTAREA" || target.isContentEditable;
    if (event.key === "Enter" && !multiline && !props.returnInContent && target.tagName !== "BUTTON") {
      if (props.confirm && !props.confirmDisabled) {
        event.preventDefault();
        props.onConfirm?.();
      }
    }
  };
  return (
    <div ref={(el) => (element = el)} class={["sheet", props.class]} style={{ width: `${props.width ?? 420}px` }} tabindex={-1} onKeyDown={onKeyDown}>
      <div class="sheet-header">
        <div class="sheet-title">{props.title}</div>
        <Show when={props.subtitle}>
          <div class="sheet-subtitle">{props.subtitle}</div>
        </Show>
      </div>
      <Show when={props.children}>
        <div class="sheet-content">{props.children}</div>
      </Show>
      <Show when={props.confirm || props.leading}>
        <div class="sheet-buttons">
          <Show when={props.leading}>
            <div class="sheet-leading">{props.leading}</div>
          </Show>
          <div class="sheet-spacer" />
          <Show when={props.cancel !== null && props.confirm}>
            <Button onClick={() => props.onCancel()}>{props.cancel ?? L("Cancel")}</Button>
          </Show>
          <Show when={props.confirm}>
            <Button kind={props.confirmKind ?? "primary"} disabled={props.confirmDisabled} onClick={() => props.onConfirm?.()}>
              {props.confirm}
            </Button>
          </Show>
        </div>
      </Show>
    </div>
  );
}

// MARK: - Alerts

export interface AlertOptions {
  message: string;
  informative?: string;
  /** The first is the default (Return); one titled Cancel answers Escape, else the last. */
  buttons?: { title: string; destructive?: boolean }[];
  /** The button Escape presses, when it is neither Cancel nor the last. */
  escape?: number;
  style?: "informational" | "warning" | "critical";
  /** A control under the text, as an alert's accessory view. */
  accessory?: () => JSX.Element;
  width?: number;
}

/** An alert as a sheet, answering with the index of the button chosen. */
export function alert(options: AlertOptions): Promise<number> {
  return new Promise((resolve) => {
    const buttons = options.buttons ?? [{ title: L("OK") }];
    let answered = false;
    const answer = (index: number, dismiss: () => void) => {
      if (answered) return;
      answered = true;
      dismiss();
      resolve(index);
    };
    const cancelIndex = options.escape ?? buttons.findIndex((button) => button.title === L("Cancel"));
    presentSheet(
      (dismiss) => (
        <div
          class="sheet alert"
          style={{ width: `${options.width ?? 380}px` }}
          tabindex={-1}
          onKeyDown={(event) => {
            if (event.isComposing) return;
            if (event.key === "Escape") {
              event.preventDefault();
              answer(cancelIndex >= 0 ? cancelIndex : buttons.length - 1, dismiss);
            } else if (event.key === "Enter" && (event.target as HTMLElement).tagName !== "BUTTON" && (event.target as HTMLElement).tagName !== "TEXTAREA") {
              event.preventDefault();
              answer(0, dismiss);
            }
          }}
        >
          <span class="alert-icon" aria-hidden="true">
            <img src={hostInfo().isDevelopment ? devIconURL : iconURL} width={48} height={48} alt="" draggable="false" />
            <Show when={options.style === "critical"}>
              <span class="alert-caution">
                <Icon name="exclamationmark.triangle.fill" size={20} strokeWidth={2.2} />
              </span>
            </Show>
          </span>
          <div class="sheet-header">
            <div class="sheet-title">{options.message}</div>
            <Show when={options.informative}>
              <div class="sheet-subtitle alert-informative">{options.informative}</div>
            </Show>
          </div>
          <Show when={options.accessory}>{(accessory) => <div class="alert-accessory">{accessory()()}</div>}</Show>
          <div class="sheet-buttons">
            <div class="sheet-spacer" />
            <For each={[...buttons.keys()].reverse()}>
              {(index) => (
                <Button
                  kind={buttons[index]!.destructive ? "destructive" : index === 0 ? "primary" : "default"}
                  ref={(element) => {
                    if (index === 0 && !options.accessory) queueMicrotask(() => element.focus());
                  }}
                  onClick={() => answer(index, dismiss)}
                >
                  {buttons[index]!.title}
                </Button>
              )}
            </For>
          </div>
        </div>
      ),
      () => {
        if (!answered) {
          answered = true;
          resolve(cancelIndex >= 0 ? cancelIndex : buttons.length - 1);
        }
      },
    );
  });
}

// MARK: - Popovers

interface PopoverEntry {
  anchor: DOMRect;
  render: (close: () => void) => JSX.Element;
  edge: "below" | "above";
  onClose?: () => void;
  class?: string;
}

const popover = box<PopoverEntry | null>(null);

export function closePopover(): void {
  const current = popover.get();
  if (!current) return;
  popover.set(null);
  current.onClose?.();
}

export function isPopoverOpen(): boolean {
  return popover.get() !== null;
}

/** A transient popover hanging off `anchor`, below it unless there is no room. */
export function showPopover(anchor: Element, render: (close: () => void) => JSX.Element, options: { edge?: "below" | "above"; onClose?: () => void; class?: string } = {}): void {
  closePopover();
  popover.set({ anchor: anchor.getBoundingClientRect(), render, edge: options.edge ?? "below", onClose: options.onClose, class: options.class });
}

export function PopoverHost() {
  return (
    <Show when={popover.read()} keyed>
      {(entry) => {
        let element: HTMLDivElement | undefined;
        const place = () => {
          if (!element) return;
          const width = element.offsetWidth;
          const height = element.offsetHeight;
          const x = Math.min(Math.max(8, entry.anchor.left + entry.anchor.width / 2 - width / 2), window.innerWidth - width - 8);
          const below = entry.anchor.bottom + 8;
          const above = entry.anchor.top - 8 - height;
          const y = entry.edge === "above" ? (above >= 8 ? above : below) : below + height <= window.innerHeight - 8 ? below : Math.max(8, above);
          element.style.left = `${Math.round(x)}px`;
          element.style.top = `${Math.round(y)}px`;
          const arrow = Math.min(Math.max(14, entry.anchor.left + entry.anchor.width / 2 - x), width - 14);
          element.style.setProperty("--arrow-x", `${arrow}px`);
          element.dataset.side = y >= entry.anchor.bottom ? "below" : "above";
        };
        onSettled(() => {
          place();
          const observer = new ResizeObserver(place);
          if (element) observer.observe(element);
          const onPointer = (event: MouseEvent) => {
            if (element && !element.contains(event.target as Node)) closePopover();
          };
          const onKey = (event: KeyboardEvent) => {
            if (event.key === "Escape") {
              event.stopPropagation();
              closePopover();
            }
          };
          const onBlur = () => closePopover();
          window.addEventListener("mousedown", onPointer, true);
          window.addEventListener("keydown", onKey, true);
          window.addEventListener("blur", onBlur);
          return () => {
            observer.disconnect();
            window.removeEventListener("mousedown", onPointer, true);
            window.removeEventListener("keydown", onKey, true);
            window.removeEventListener("blur", onBlur);
          };
        });
        return (
          <Portal>
            <div ref={(el) => (element = el)} class={["popover", entry.class]} style={{ left: "-9999px", top: "0px" }}>
              {entry.render(closePopover)}
            </div>
          </Portal>
        );
      }}
    </Show>
  );
}

/** The full text behind a one-line marker, rendered like a message body, scrolling past a
 * screenful, sized to its content up to a bubble's width. */
export function showTextPopover(anchor: Element, text: string): void {
  if (!text.trim()) return;
  showPopover(anchor, () => (
    <div class="text-popover selectable">
      <Markdown text={text} />
    </div>
  ));
}

/** The first non-empty line of `text`, with fence markers skipped: what a one-line preview shows. */
export function firstLineOf(text: string): string {
  return (
    text
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find((line) => line !== "" && !line.startsWith("```")) ?? ""
  );
}
