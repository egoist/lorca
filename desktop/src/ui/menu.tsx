// Menus that pop up: the system's own in a Lorca window, and a stand-in drawn in the page in a
// browser tab.

import { Portal } from "@solidjs/web";
import { For, onSettled, Show } from "solid-js";
import { inApp, menus, type MenuItemSpec } from "../host";
import { box } from "./box";

export interface MenuEntry {
  id?: string;
  label?: string;
  separator?: boolean;
  checked?: boolean;
  disabled?: boolean;
}

export const separator: MenuEntry = { separator: true };

function spec(entries: MenuEntry[]): MenuItemSpec[] {
  return entries.map((entry) =>
    entry.separator
      ? { type: "separator" }
      : { id: entry.id, label: entry.label, disabled: entry.disabled, checked: entry.checked, type: entry.checked !== undefined ? "checkbox" : undefined },
  );
}

interface PageMenu {
  entries: MenuEntry[];
  x: number;
  y: number;
  resolve: (id: string | null) => void;
}

const pageMenu = box<PageMenu | null>(null);

/** Shows a menu with its top-left corner at `x`, `y` (viewport pixels), or below `anchor`, and
 * answers with the id of the item picked, or null. */
export async function popupMenu(entries: MenuEntry[], at: { x: number; y: number } | HTMLElement): Promise<string | null> {
  let point: { x: number; y: number };
  if (at instanceof HTMLElement) {
    const rect = at.getBoundingClientRect();
    point = { x: Math.round(rect.left), y: Math.round(rect.bottom + 2) };
  } else {
    point = { x: Math.round(at.x), y: Math.round(at.y) };
  }
  if (inApp) {
    const id = await menus.popup(spec(entries), point.x, point.y);
    return id || null;
  }
  return new Promise((resolve) => {
    pageMenu.get()?.resolve(null);
    pageMenu.set({ entries, x: point.x, y: point.y, resolve });
  });
}

/** Where a browser tab draws the menus the app gets from the system. */
export function PageMenuHost() {
  const close = (id: string | null) => {
    const menu = pageMenu.get();
    pageMenu.set(null);
    menu?.resolve(id);
  };
  return (
    <Show when={pageMenu.read()}>
      {(menu) => {
        let element: HTMLDivElement | undefined;
        onSettled(() => {
          const onKey = (event: KeyboardEvent) => {
            if (event.key === "Escape") close(null);
          };
          window.addEventListener("keydown", onKey);
          if (element) {
            const rect = element.getBoundingClientRect();
            if (rect.bottom > window.innerHeight) element.style.top = `${Math.max(4, window.innerHeight - rect.height - 4)}px`;
            if (rect.right > window.innerWidth) element.style.left = `${Math.max(4, window.innerWidth - rect.width - 4)}px`;
          }
          return () => window.removeEventListener("keydown", onKey);
        });
        return (
          <Portal>
            <div class="page-menu-scrim" onMouseDown={() => close(null)} onContextMenu={(event) => event.preventDefault()} />
            <div class="page-menu" ref={(el) => (element = el)} style={{ left: `${menu().x}px`, top: `${menu().y}px` }}>
              <For each={menu().entries}>
                {(entry) =>
                  entry.separator ? (
                    <div class="page-menu-separator" />
                  ) : (
                    <button class="page-menu-item" disabled={entry.disabled} onClick={() => close(entry.id ?? null)}>
                      <span class="page-menu-check">{entry.checked ? "✓" : ""}</span>
                      {entry.label}
                    </button>
                  )
                }
              </For>
            </div>
          </Portal>
        );
      }}
    </Show>
  );
}
