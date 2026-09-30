// The command palette, after the macOS app's CommandPalette and PaletteIndex: a search field over the
// menu bar's commands, the chats, and the settings, floating over the main window. The field keeps
// the keyboard; the arrows move the list's selection, Return runs it, and Escape or a click elsewhere
// closes it. A query also searches the chats' messages through the CLI.

import { createMemo, createSignal, For, onSettled, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { host, hostInfo } from "../host";
import { L } from "../l10n";
import { paneSymbol, paneTitle, settingsPanes } from "../model/models";
import { store } from "../model/store";
import { AvatarCluster, botAvatar } from "./avatar";
import { box } from "./box";
import { commands, shortcutText } from "./commands";
import { Icon } from "./icons";
import { hasSheet } from "./overlay";
import { open, reveal, settingsDeviceID, showSettings } from "./root";
import { entriesIn } from "./settings/search";
import type { Bot } from "../model/models";
import type { WireSearchResults } from "../model/wire";

interface PaletteItem {
  icon: { kind: "symbol"; name: string } | { kind: "chat"; bots: Bot[] };
  title: string;
  subtitle: string;
  /** The menu item's shortcut, as the menu spells it. */
  shortcut: string;
  /** Words that find the item besides its title. */
  keywords: string[];
  /** Left out of the list until a query finds it. */
  isSearchOnly: boolean;
  chatID?: string;
  run: () => void;
}

interface PaletteSection {
  title: string;
  items: PaletteItem[];
}

type Row = { kind: "header"; title: string } | { kind: "item"; item: PaletteItem };

/** The menu bar's commands worth offering, in the macOS app's order. Each takes its title, shortcut,
 * and enabled state from the command table, so it reads and behaves as it does in the menu. */
const paletteCommands: { id: string; symbol: string; keywords: string }[] = [
  { id: "newBot", symbol: "plus.message", keywords: "create add" },
  { id: "newGroupChat", symbol: "person.2", keywords: "create room" },
  { id: "pairDevice", symbol: "qrcode", keywords: "phone link runner" },
  { id: "addBot", symbol: "person.badge.plus", keywords: "invite member group" },
  { id: "renameChat", symbol: "pencil", keywords: "title name" },
  { id: "pinChat", symbol: "pin", keywords: "unpin favorite" },
  { id: "stopResponding", symbol: "stop.circle", keywords: "cancel interrupt" },
  { id: "scrollToLatest", symbol: "arrow.down.to.line", keywords: "bottom newest jump" },
  { id: "toggleSidebar", symbol: "sidebar.leading", keywords: "show hide" },
  { id: "toggleInspector", symbol: "sidebar.trailing", keywords: "show hide details" },
  { id: "fullScreen", symbol: "arrow.up.left.and.arrow.down.right", keywords: "fullscreen" },
  { id: "settings", symbol: "gearshape", keywords: "preferences" },
  { id: "checkForUpdates", symbol: "arrow.triangle.2.circlepath", keywords: "upgrade version" },
  { id: "deleteChat", symbol: "trash", keywords: "remove" },
  { id: "help", symbol: "questionmark.circle", keywords: "about" },
];

function item(fields: Partial<PaletteItem> & Pick<PaletteItem, "icon" | "title" | "run">): PaletteItem {
  return { subtitle: "", shortcut: "", keywords: [], isSearchOnly: false, ...fields };
}

function actions(): PaletteItem[] {
  return paletteCommands.flatMap((entry) => {
    const command = commands.get(entry.id);
    if (!command) return [];
    if (entry.id === "checkForUpdates" && !hostInfo().updatesEnabled) return [];
    if (command.enabled && !command.enabled()) return [];
    return [
      item({
        icon: { kind: "symbol", name: entry.symbol },
        title: command.title(),
        shortcut: command.accelerator ? shortcutText(command.accelerator) : "",
        keywords: [entry.keywords],
        // Full screen is the system's own menu item; the palette asks the window for it.
        run: () => (entry.id === "fullScreen" ? void host.toggleFullScreen() : commands.run(entry.id)),
      }),
    ];
  });
}

function chats(): PaletteItem[] {
  if (!store.isConnected) return [];
  return store.chats.map((chat) => {
    const bots = store.botsIn(chat);
    return item({
      icon: { kind: "chat", bots },
      title: store.title(chat),
      subtitle: store.subtitle(chat),
      keywords: [...bots.map((bot) => bot.name), store.preview(chat)],
      chatID: chat.id,
      run: () => open(chat.id),
    });
  });
}

/** The panes, then the settings on them, which only a query brings up. */
function settings(): PaletteItem[] {
  const deviceID = settingsDeviceID.get();
  const device = deviceID ? store.device(deviceID) : undefined;
  const panes = settingsPanes.map((pane) =>
    item({ icon: { kind: "symbol", name: paneSymbol(pane) }, title: paneTitle(pane), keywords: [L("Settings")], run: () => showSettings(pane) }),
  );
  const entries = settingsPanes.flatMap((pane) =>
    entriesIn(pane, device, store).map((entry) =>
      item({
        icon: { kind: "symbol", name: paneSymbol(pane) },
        title: entry.title,
        subtitle: paneTitle(pane),
        keywords: entry.keywords,
        isSearchOnly: true,
        run: () => reveal(entry),
      }),
    ),
  );
  return [...panes, ...entries];
}

function searchSections(results: WireSearchResults): PaletteSection[] {
  const hits = (list: { chat_id: string; snippet: string }[]) =>
    list.flatMap((hit) => {
      const chat = store.chat(hit.chat_id);
      if (!chat) return [];
      return [item({ icon: { kind: "chat", bots: store.botsIn(chat) }, title: store.title(chat), subtitle: hit.snippet, isSearchOnly: true, chatID: chat.id, run: () => open(chat.id) })];
    });
  return [
    { title: L("Chats"), items: hits(results.chats) },
    { title: L("Messages"), items: hits(results.messages) },
  ].filter((section) => section.items.length > 0);
}

// MARK: - Search

/** Case, accents, and full-width forms folded away, as the Mac palette compares. */
function fold(text: string): string {
  return text.normalize("NFKC").normalize("NFD").replace(/\p{Diacritic}/gu, "").toLocaleLowerCase();
}

function isSubsequence(needle: string, haystack: string): boolean {
  let at = 0;
  for (const character of needle) {
    const found = haystack.indexOf(character, at);
    if (found < 0) return false;
    at = found + character.length;
  }
  return true;
}

/** A title that starts with the query beats a word that does, then a substring, then the query's
 * letters in order ("ngc" finds New Group Chat). Keywords and subtitles rank last. */
function score(entry: PaletteItem, query: string): number | null {
  const title = fold(entry.title);
  if (title.startsWith(query)) return 100;
  const words = title.split(/[^\p{L}\p{N}]+/u).filter((word) => word !== "");
  if (words.some((word) => word.startsWith(query))) return 80;
  if (title.includes(query)) return 60;
  if (isSubsequence(query, words.map((word) => [...word][0]).join(""))) return 50;
  if ([entry.subtitle, ...entry.keywords].some((text) => fold(text).includes(query))) return 40;
  if ([...query].length > 1 && isSubsequence(query, title)) return 20;
  return null;
}

/** The sections narrowed to a query, best match first. With no query, everything but the items only
 * a search brings up. */
function filter(sections: PaletteSection[], raw: string): PaletteSection[] {
  const query = fold(raw.trim());
  return sections.flatMap((section) => {
    const items =
      query === ""
        ? section.items.filter((entry) => !entry.isSearchOnly)
        : section.items
            .flatMap((entry) => {
              const value = score(entry, query);
              return value === null ? [] : [{ entry, value }];
            })
            .sort((a, b) => b.value - a.value)
            .map(({ entry }) => entry);
    return items.length === 0 ? [] : [{ title: section.title, items }];
  });
}

function merge(incoming: PaletteSection[], sections: PaletteSection[]): PaletteSection[] {
  const merged = sections.map((section) => ({ ...section, items: [...section.items] }));
  for (const section of incoming) {
    const existing = section.title === L("Chats") ? merged.find((each) => each.title === section.title) : undefined;
    if (existing) {
      const known = new Set(existing.items.flatMap((entry) => (entry.chatID ? [entry.chatID] : [])));
      existing.items.push(...section.items.filter((entry) => !entry.chatID || !known.has(entry.chatID)));
    } else {
      merged.push(section);
    }
  }
  return merged;
}

/** The query's words in `text`, semibold on an accent tint, found ignoring case and accents. */
function highlighted(text: string, query: string): JSX.Element {
  const terms = fold(query)
    .split(/[^\p{L}\p{N}]+/u)
    .filter((term) => term !== "");
  if (terms.length === 0 || text === "") return text;
  const characters = [...text];
  // Folded one character at a time, so a match maps back onto the characters it came from.
  const folded = characters.map((character) => {
    const one = fold(character);
    return [...one].length === 1 ? one : character.toLocaleLowerCase();
  });
  const marked = new Array<boolean>(characters.length).fill(false);
  for (const term of terms) {
    const needle = [...term];
    for (let start = 0; start + needle.length <= folded.length; start++) {
      if (needle.every((character, offset) => folded[start + offset] === character)) {
        for (let offset = 0; offset < needle.length; offset++) marked[start + offset] = true;
        start += needle.length - 1;
      }
    }
  }
  const parts: JSX.Element[] = [];
  let run = "";
  let inMark = false;
  const flush = () => {
    if (run === "") return;
    parts.push(inMark ? <mark>{run}</mark> : run);
    run = "";
  };
  characters.forEach((character, index) => {
    if (marked[index] !== inMark) {
      flush();
      inMark = marked[index]!;
    }
    run += character;
  });
  flush();
  return parts;
}

// MARK: - The panel

const isOpen = box(false);

export const palette = {
  toggle(): void {
    if (isOpen.get()) palette.close();
    else palette.show();
  },
  show(): void {
    if (hasSheet()) {
      void host.beep();
      return;
    }
    isOpen.set(true);
  },
  close(): void {
    isOpen.set(false);
  },
  isOpen: () => isOpen.get(),
};

export function PaletteHost() {
  return (
    <Show when={isOpen.read()}>
      <Palette />
    </Show>
  );
}

const metric = { width: 620, fieldHeight: 52, rowHeight: 36, headerHeight: 26, maxListHeight: 360, emptyHeight: 64 };

function Palette() {
  // Read when the palette opens, as the Mac palette reads the menu before it takes the keyboard.
  const sections: PaletteSection[] = [
    { title: L("Actions"), items: actions() },
    { title: L("Chats"), items: chats() },
    { title: L("Settings"), items: settings() },
  ];
  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<WireSearchResults>({ chats: [], messages: [] });
  const [searching, setSearching] = createSignal(false);
  /** The row the arrows or the pointer picked; none follows the list's first item. */
  const [picked, setPicked] = createSignal<number | null>(null);
  let field: HTMLInputElement | undefined;
  let list: HTMLDivElement | undefined;
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  let searchGeneration = 0;
  /** Where the pointer was when the list last changed under it: rows that appear or scroll beneath a
   * resting pointer leave the selection alone; only a real move selects. */
  let restingPointer: { x: number; y: number } | null = null;
  let lastPointer = { x: -1, y: -1 };

  const rows = createMemo((): Row[] => {
    let matched = filter(sections, query());
    if (query().trim() !== "") matched = merge(searchSections(results()), matched);
    return matched.flatMap((section): Row[] => [{ kind: "header", title: section.title }, ...section.items.map((entry): Row => ({ kind: "item", item: entry }))]);
  });
  const firstItem = () => rows().findIndex((row) => row.kind === "item");
  const selected = () => picked() ?? firstItem();
  const setSelected = (index: number) => setPicked(index);

  const close = () => {
    clearTimeout(searchTimer);
    searchGeneration++;
    palette.close();
  };
  const run = (entry: PaletteItem) => {
    close();
    // The window's content has the keyboard again by the time the command looks for it.
    setTimeout(() => entry.run());
  };

  const searchMessages = (text: string) => {
    clearTimeout(searchTimer);
    const generation = ++searchGeneration;
    const trimmed = text.trim();
    if (trimmed === "") {
      setSearching(false);
      return;
    }
    searchTimer = setTimeout(() => {
      store.searchChats(trimmed).then(
        (found) => {
          if (generation !== searchGeneration) return;
          setResults(found);
          setSearching(false);
          listChanged();
        },
        () => {
          if (generation !== searchGeneration) return;
          setResults({ chats: [], messages: [] });
          setSearching(false);
        },
      );
    }, 140);
  };
  /** The list changed: the first item takes the selection, and the list starts at its top. */
  const listChanged = () => {
    restingPointer = { ...lastPointer };
    setPicked(null);
    queueMicrotask(() => list?.scrollTo({ top: 0 }));
  };

  const scrollToVisible = (index: number) => {
    const element = list?.querySelector<HTMLElement>(`[data-row="${index}"]`);
    if (!element) return;
    // A section's first item brings its header along, so the list's top never hides one.
    const previous = rows()[index - 1];
    const header = previous?.kind === "header" ? list?.querySelector<HTMLElement>(`[data-row="${index - 1}"]`) : null;
    (header ?? element).scrollIntoView({ block: "nearest" });
    element.scrollIntoView({ block: "nearest" });
  };

  /** The next item up or down, past the headers, wrapping at the ends. */
  const move = (delta: number) => {
    const items = rows().flatMap((row, index) => (row.kind === "item" ? [index] : []));
    if (items.length === 0) return;
    const current = items.indexOf(selected());
    const next = current >= 0 ? items[(current + delta + items.length) % items.length]! : delta > 0 ? items[0]! : items[items.length - 1]!;
    setSelected(next);
    scrollToVisible(next);
  };

  onSettled(() => {
    field?.focus();
    const onBlur = () => close();
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("blur", onBlur);
      clearTimeout(searchTimer);
    };
  });

  const top = Math.max(90, Math.round(window.innerHeight * 0.2));
  const width = Math.min(metric.width, window.innerWidth - 40);
  return (
    <div class="palette-scrim" onMouseDown={(event) => event.target === event.currentTarget && close()}>
      <div class="palette" role="dialog" aria-label={L("Command Palette")} style={{ top: `${top}px`, width: `${width}px` }}>
        <div class="palette-field" style={{ height: `${metric.fieldHeight}px` }}>
          <Icon name="magnifyingglass" size={20} strokeWidth={1.8} class="palette-glass" />
          <input
            ref={(element) => (field = element)}
            value={query()}
            placeholder={L("Search actions, chats, messages, and settings")}
            spellcheck={false}
            autocomplete="off"
            onInput={(event) => {
              const text = event.currentTarget.value;
              setQuery(text);
              setResults({ chats: [], messages: [] });
              setSearching(text.trim() !== "");
              listChanged();
              searchMessages(text);
            }}
            onKeyDown={(event) => {
              if (event.isComposing) return;
              if (event.key === "ArrowDown") {
                event.preventDefault();
                move(1);
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                move(-1);
              } else if (event.key === "Enter") {
                event.preventDefault();
                const row = rows()[selected()];
                if (row?.kind === "item") run(row.item);
              } else if (event.key === "Escape") {
                event.preventDefault();
                event.stopPropagation();
                close();
              }
            }}
          />
        </div>
        <div class="palette-separator" />
        <Show
          when={rows().length > 0}
          fallback={
            <div class="palette-empty" style={{ height: `${metric.emptyHeight}px` }}>
              {searching() ? L("Searching…") : L("No Results")}
            </div>
          }
        >
          <div
            ref={(element) => (list = element)}
            class="palette-list"
            role="listbox"
            style={{ "max-height": `${metric.maxListHeight}px` }}
            onMouseMove={(event) => {
              lastPointer = { x: event.clientX, y: event.clientY };
              if (restingPointer && Math.abs(event.clientX - restingPointer.x) <= 1 && Math.abs(event.clientY - restingPointer.y) <= 1) return;
              restingPointer = null;
              const target = (event.target as HTMLElement).closest<HTMLElement>("[data-row]");
              const index = target ? Number(target.dataset.row) : -1;
              if (index >= 0 && rows()[index]?.kind === "item") setSelected(index);
            }}
          >
            <For each={rows()}>
              {(row, index) =>
                row.kind === "header" ? (
                  <div class="palette-header" data-row={index()} style={{ height: `${metric.headerHeight}px` }}>
                    {row.title}
                  </div>
                ) : (
                  <div
                    class={["palette-row", { selected: selected() === index() }]}
                    role="option"
                    aria-selected={selected() === index() ? "true" : "false"}
                    data-row={index()}
                    style={{ height: `${metric.rowHeight}px` }}
                    onMouseDown={(event) => event.preventDefault()}
                    onClick={() => run(row.item)}
                  >
                    <span class="palette-icon">
                      {row.item.icon.kind === "symbol" ? (
                        <Icon name={row.item.icon.name} size={16} strokeWidth={1.8} />
                      ) : (
                        <AvatarCluster contents={row.item.icon.bots.map(botAvatar)} slot={22} />
                      )}
                    </span>
                    <span class="palette-title">{highlighted(row.item.title, query())}</span>
                    <Show when={row.item.subtitle !== ""}>
                      <span class="palette-subtitle">{highlighted(row.item.subtitle, query())}</span>
                    </Show>
                    <span class="palette-spacer" />
                    <Show when={row.item.shortcut !== ""}>
                      <span class="palette-shortcut">{row.item.shortcut}</span>
                    </Show>
                  </div>
                )
              }
            </For>
          </div>
        </Show>
      </div>
    </div>
  );
}
