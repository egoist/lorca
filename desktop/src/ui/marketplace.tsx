// The marketplace, after the macOS app's MarketplaceViewController and its pages: featured plugins
// and bots, everything else by category, one search over both, and a page for each plugin and bot.
// A sheet with a way back: the home page leads to a plugin, a bot, a full list, or the plugins the
// Runner has. Plugins install on the Runner picked in the top bar, for every bot there; a bot is
// added to that Runner, and the sheet closes on the new bot's chat, where the bot sets itself up.

import { createMemo, createSignal, For, onSettled, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { host } from "../host";
import { L, Lc } from "../l10n";
import * as Format from "../model/format";
import {
  deviceSymbol,
  pluginStateColor,
  pluginSymbol,
  type BotTemplate,
  type Device,
  type InstalledPlugin,
  type Marketplace,
  type MarketplacePlugin,
} from "../model/models";
import { onStoreEvent, track } from "../model/reactive";
import { errorText, store } from "../model/store";
import { Avatar } from "./avatar";
import { box } from "./box";
import { Button, HoverButton, PopUpButton, SearchField, Segmented, Spinner } from "./controls";
import { Icon } from "./icons";
import { presentSheet } from "./overlay";
import { pluginMarks } from "./pluginMarks";
import { open } from "./root";
import { KeyValueRow, NoteRow, PluginRow, Section, StatusRow } from "./sections";
import { presentPlugin } from "./sheets/plugin";

/** How many rows a home section shows before View all. */
const previewCount = 4;

type Item = { kind: "plugin"; plugin: MarketplacePlugin } | { kind: "bot"; bot: BotTemplate };
type Page =
  | { kind: "home" }
  | { kind: "plugin"; id: string }
  | { kind: "bot"; id: string }
  | { kind: "list"; title: string; items: (catalog: Marketplace) => Item[] }
  | { kind: "installed" };

const itemCategory = (item: Item) => (item.kind === "plugin" ? item.plugin.category : item.bot.category);
const itemID = (item: Item) => `${item.kind}:${item.kind === "plugin" ? item.plugin.id : item.bot.id}`;
const allItems = (catalog: Marketplace): Item[] => [
  ...catalog.plugins.map((plugin): Item => ({ kind: "plugin", plugin })),
  ...catalog.bots.map((bot): Item => ({ kind: "bot", bot })),
];

/** Every word of the query appears in its name, description, category, or maker. */
function matches(item: Item, words: string[]): boolean {
  const fields =
    item.kind === "plugin"
      ? [item.plugin.name, item.plugin.description, item.plugin.author, categoryTitle(item.plugin.category), ...item.plugin.tags]
      : [item.bot.name, item.bot.summary, item.bot.description, item.bot.author, categoryTitle(item.bot.category)];
  const text = fields.join(" ").toLocaleLowerCase();
  return words.every((word) => text.includes(word.toLocaleLowerCase()));
}

/** The sections the home page lists after the featured ones. */
const categoryOrder = ["credentials", "productivity", "communication", "design", "code", "data", "sales", "finance", "research", "support"];

export function categoryTitle(key: string): string {
  switch (key) {
    case "credentials":
      return L("Login and Credential Management");
    case "productivity":
      return L("Productivity");
    case "communication":
      return L("Communication");
    case "design":
      return L("Design");
    case "code":
      return L("Code");
    case "data":
      return L("Data");
    case "sales":
      return L("Sales");
    case "finance":
      return L("Finance");
    case "research":
      return L("Research");
    case "support":
      return L("Support");
    case "":
      return L("More");
    default:
      return key
        .replaceAll("-", " ")
        .split(" ")
        .map((word) => word.charAt(0).toUpperCase() + word.slice(1).toLowerCase())
        .join(" ");
  }
}

/** The categories in use, known ones in their order, then the rest, then the uncategorized. */
function categoryKeys(items: Item[]): string[] {
  const used = new Set(items.map(itemCategory));
  const known = categoryOrder.filter((key) => used.has(key));
  const other = [...used].filter((key) => key !== "" && !categoryOrder.includes(key)).sort();
  return [...known, ...other, ...(used.has("") ? [""] : [])];
}

let isOpen = false;

/** The marketplace as a sheet sized to the window. From a bot's inspector it opens on that bot's
 * Runner, from Settings on the picked Device; a bot added there lands in its chat. */
export function presentMarketplace(runnerID: string | null = null): void {
  if (isOpen) return;
  isOpen = true;
  const width = Math.min(800, Math.max(640, window.innerWidth - 60));
  const height = Math.min(700, Math.max(460, window.innerHeight - 60));
  presentSheet(
    (dismiss) => <MarketplaceSheet runnerID={runnerID} width={width} height={height} dismiss={dismiss} />,
    () => {
      isOpen = false;
    },
  );
}

interface Market {
  catalog: () => Marketplace;
  loading: () => "loading" | "loaded" | "failed";
  runner: () => Device | undefined;
  installing: () => Record<string, true>;
  plugin: (id: string) => MarketplacePlugin | undefined;
  installedPlugin: (id: string) => InstalledPlugin | undefined;
  installedPlugins: () => InstalledPlugin[];
  load: () => void;
  show: (page: Page) => void;
  install: (plugin: MarketplacePlugin) => void;
  manage: (pluginID: string) => void;
  add: (template: BotTemplate) => void;
}

function MarketplaceSheet(props: { runnerID: string | null; width: number; height: number; dismiss: () => void }) {
  const catalog = box<Marketplace>({ plugins: [], bots: [] });
  const loading = box<"loading" | "loaded" | "failed">("loading");
  const installing = box<Record<string, true>>({});
  /** What an install answered, by Runner and plugin, until the Runner's own list has it: a Runner
   * elsewhere reports its plugins through the relay a moment later. */
  const installed = box<Record<string, Record<string, InstalledPlugin>>>({});
  const pages = box<Page[]>([{ kind: "home" }]);
  const [notice, setNotice] = createSignal<{ text: string; isError: boolean; fading: boolean } | null>(null);
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;
  let root: HTMLDivElement | undefined;

  const runners = createMemo(() => {
    track.roster();
    return store.runners;
  });
  const initialRunner = (() => {
    const all = store.runners;
    return all.find((device) => device.id === props.runnerID)?.id ?? all.find((device) => device.isThisDevice)?.id ?? all[0]?.id ?? null;
  })();
  const pickedRunner = box<string | null>(initialRunner);
  /** Where plugins install and bots are added; the first Runner when the picked one leaves. */
  const runner = () => {
    const all = runners();
    return all.find((device) => device.id === pickedRunner.read()) ?? all[0];
  };

  const load = () => {
    loading.set("loading");
    store.marketplace().then(
      (loaded) => {
        catalog.set(loaded);
        loading.set("loaded");
      },
      () => loading.set("failed"),
    );
  };

  const showNotice = (text: string, isError = false) => {
    setNotice({ text, isError, fading: false });
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(
      () => {
        setNotice((current) => (current ? { ...current, fading: true } : null));
        noticeTimer = setTimeout(() => setNotice(null), 250);
      },
      isError ? 7000 : 4000,
    );
  };

  const nextStep = (plugin: InstalledPlugin, on: Device): string => {
    switch (plugin.state) {
      case "ready":
        return L("Added %@. Every bot on %@ can use it.", plugin.name, on.name);
      case "needs_auth":
        return L("Added %@. It needs a sign-in: click Connect.", plugin.name);
      case "needs_setup":
        return L("Added %@. It needs setup: click Set Up.", plugin.name);
      default:
        return `${plugin.name}: ${plugin.detail}`;
    }
  };

  const market: Market = {
    catalog: () => catalog.read(),
    loading: () => loading.read(),
    runner,
    installing: () => installing.read(),
    plugin: (id) => catalog.read().plugins.find((plugin) => plugin.id === id),
    /** The plugin as the picked Runner has it, undefined when it is not installed there. */
    installedPlugin: (id) => {
      const on = runner();
      if (!on) return undefined;
      return on.plugins.find((plugin) => plugin.id === id) ?? installed.read()[on.id]?.[id];
    },
    /** Everything the picked Runner has, the marketplace's and the rest, in its own order. */
    installedPlugins: () => {
      const on = runner();
      if (!on) return [];
      const extra = Object.values(installed.read()[on.id] ?? {}).filter((plugin) => !on.plugins.some((each) => each.id === plugin.id));
      return [...on.plugins, ...extra.sort((a, b) => a.name.localeCompare(b.name))];
    },
    load,
    show: (page) => pages.set([...pages.get(), page]),
    /** Installs a plugin on the picked Runner, for every bot there. */
    install: (plugin) => {
      const on = runner();
      if (!on || installing.get()[plugin.id]) return;
      installing.set({ ...installing.get(), [plugin.id]: true });
      store
        .installPlugin(plugin.id, on.id)
        .then(
          (status) => {
            installed.set({ ...installed.get(), [on.id]: { ...installed.get()[on.id], [plugin.id]: status } });
            showNotice(nextStep(status, on));
          },
          (error) => showNotice(L("Couldn't install %@: %@", plugin.name, errorText(error)), true),
        )
        .finally(() => {
          const next = { ...installing.get() };
          delete next[plugin.id];
          installing.set(next);
        });
    },
    /** The plugin's own sheet on the picked Runner: its sign-in, its setup, and Remove. */
    manage: (pluginID) => {
      const on = runner();
      if (on) presentPlugin(pluginID, on);
    },
    /** Adds the bot on the picked Runner and opens its chat, where it greets the user. */
    add: (template) => {
      const on = runner();
      if (!on) return;
      const chatID = store.addBotFromTemplate(template, on.id);
      props.dismiss();
      open(chatID);
    },
  };

  const goBack = () => {
    if (pages.get().length > 1) pages.set(pages.get().slice(0, -1));
  };

  onSettled(() => {
    load();
    if (root && !root.contains(document.activeElement)) root.focus();
    const off = onStoreEvent((event) => {
      if (event.kind === "rosterChanged" && !store.runners.some((device) => device.id === pickedRunner.get())) pickedRunner.set(store.runners[0]?.id ?? null);
    });
    return () => {
      off();
      clearTimeout(noticeTimer);
    };
  });

  return (
    <div
      ref={(element) => (root = element)}
      class="sheet market"
      style={{ width: `${props.width}px`, height: `${props.height}px` }}
      tabindex={-1}
      onKeyDown={(event) => {
        // Escape closes the sheet from any page, wherever the keyboard is, the search field too.
        if (event.key === "Escape" && !event.isComposing) {
          event.preventDefault();
          props.dismiss();
        }
      }}
    >
      <div class="market-bar">
        <HoverButton symbol="chevron.left" tooltip={L("Back")} hidden={pages.read().length < 2} onClick={goBack} />
        <span class="sheet-spacer" />
        {/* One Runner is no choice; the pages name it where it matters. */}
        <Show when={runners().length >= 2}>
          <PopUpButton
            style="settings"
            tooltip={L("Plugins install here, and bots are added here.")}
            options={runners().map((device) => ({ value: device.id, label: device.isThisDevice ? L("%@ (this computer)", device.name) : device.name }))}
            value={runner()?.id ?? ""}
            onChange={(id) => pickedRunner.set(id)}
          />
        </Show>
        <HoverButton symbol="xmark" size={14} tooltip={L("Close")} onClick={props.dismiss} />
      </div>
      <div class="market-pages">
        <For each={pages.read()}>
          {(page, index) => (
            <div class="market-page" hidden={index() !== pages.read().length - 1}>
              <div class="market-page-content">
                <PageView page={page} market={market} />
              </div>
            </div>
          )}
        </For>
      </div>
      <Show when={notice()}>
        {(current) => <div class={["market-notice", { error: current().isError, fading: current().fading }]}>{current().text}</div>}
      </Show>
    </div>
  );
}

function PageView(props: { page: Page; market: Market }) {
  switch (props.page.kind) {
    case "home":
      return <HomePage market={props.market} />;
    case "plugin":
      return <PluginPage market={props.market} pluginID={props.page.id} />;
    case "bot":
      return <BotPage market={props.market} templateID={props.page.id} />;
    case "list":
      return <ListPage market={props.market} heading={props.page.title} items={props.page.items} />;
    case "installed":
      return <InstalledPage market={props.market} />;
  }
}

// MARK: - Pieces

/** A plugin's icon: its real mark on a white tile, or its symbol on a light tile with a hairline. */
function PluginIcon(props: { pluginID: string; symbol: string; size: number }) {
  const mark = () => pluginMarks[props.pluginID];
  return (
    <Show
      when={mark()}
      fallback={
        <span class="plugin-icon-tile" style={{ width: `${props.size}px`, height: `${props.size}px`, "border-radius": `${props.size * 0.25}px` }}>
          <Icon name={props.symbol} size={Math.round(props.size * 0.46)} strokeWidth={1.8} />
        </span>
      }
    >
      {(svg) => (
        <span
          class="plugin-tile"
          style={{ width: `${props.size}px`, height: `${props.size}px`, "border-radius": `${props.size * 0.25}px`, padding: `${props.size * 0.2}px` }}
          innerHTML={svg()}
        />
      )}
    </Show>
  );
}

/** One entry in a grid: the icon or avatar, the name with its maker, a line about it, and what can
 * be done with it on the Runner. A click anywhere but the accessory's button opens it. */
function MarketRow(props: { media: JSX.Element; title: string; byline?: string; subtitle: string; accessory?: JSX.Element; onOpen: () => void; onCard?: boolean }) {
  return (
    <div
      class={["market-row", { "on-card": !!props.onCard }]}
      role="button"
      tabindex={0}
      title={props.subtitle}
      aria-label={[props.title, props.byline ?? "", props.subtitle].filter((part) => part !== "").join(", ")}
      onClick={(event) => {
        if (!(event.target as Element).closest("button")) props.onOpen();
      }}
      onKeyDown={(event) => {
        if ((event.key === "Enter" || event.key === " ") && event.target === event.currentTarget) {
          event.preventDefault();
          props.onOpen();
        }
      }}
    >
      <span class="market-row-media">{props.media}</span>
      <div class="market-row-text">
        <span class="market-row-title truncate">
          {props.title}
          <Show when={props.byline}>
            <span class="market-row-byline">{`  ${props.byline}`}</span>
          </Show>
        </span>
        <span class="market-row-subtitle truncate">{props.subtitle}</span>
      </div>
      <Show when={props.accessory}>
        <span class="market-row-accessory">{props.accessory}</span>
      </Show>
    </div>
  );
}

/** What a plugin's row offers on the picked Runner: Add, Added, Connect, Set Up, or its state. */
function PluginAccessory(props: { plugin: MarketplacePlugin; market: Market }) {
  const market = props.market;
  const state = () => market.installedPlugin(props.plugin.id);
  return (
    <Show
      when={!market.installing()[props.plugin.id]}
      fallback={
        <span role="progressbar" aria-label={L("Adding %@", props.plugin.name)}>
          <Spinner size={14} />
        </span>
      }
    >
      <Show
        when={state()}
        fallback={
          <Button
            disabled={!market.runner()}
            tooltip={market.runner() ? L("Install %@ on %@, for every bot there", props.plugin.name, market.runner()!.name) : L("Pair a Runner first.")}
            onClick={() => market.install(props.plugin)}
          >
            {L("Add")}
          </Button>
        }
      >
        {(current) => <>{installedAccessory(current(), props.plugin.id, market)}</>}
      </Show>
    </Show>
  );
}

function installedAccessory(current: InstalledPlugin, pluginID: string, market: Market): JSX.Element {
  switch (current.state) {
    case "ready":
      return (
        <span class="market-added">
          <Icon name="checkmark" size={12} strokeWidth={2.6} />
          <span>{L("Added")}</span>
        </span>
      );
    case "needs_auth":
      return <Button onClick={() => market.manage(pluginID)}>{L("Connect")}</Button>;
    case "needs_setup":
      return <Button onClick={() => market.manage(pluginID)}>{L("Set Up")}</Button>;
    default:
      return (
        <span class="market-state" style={{ color: pluginStateColor(current.state) }}>
          {current.detail}
        </span>
      );
  }
}

function ItemRow(props: { item: Item; market: Market }) {
  const market = props.market;
  const item = props.item;
  if (item.kind === "plugin") {
    const plugin = item.plugin;
    return (
      <MarketRow
        media={<PluginIcon pluginID={plugin.id} symbol={pluginSymbol(plugin)} size={40} />}
        title={plugin.name}
        subtitle={plugin.description}
        accessory={<PluginAccessory plugin={plugin} market={market} />}
        onOpen={() => market.show({ kind: "plugin", id: plugin.id })}
      />
    );
  }
  const template = item.bot;
  return (
    <MarketRow
      media={<Avatar content={{ kind: "bot", symbolName: template.symbolName, accent: template.accent }} size={40} />}
      title={template.name}
      byline={template.author === "" ? undefined : L("by %@", template.author)}
      subtitle={template.summary}
      accessory={
        <Button
          disabled={!market.runner()}
          tooltip={market.runner() ? L("Add %@ to %@", template.name, market.runner()!.name) : L("Pair a Runner first.")}
          onClick={() => market.add(template)}
        >
          {L("Add")}
        </Button>
      }
      onOpen={() => market.show({ kind: "bot", id: template.id })}
    />
  );
}

/** Rows two to a line: equal columns 8 apart, a line 2 below the one before. */
function Grid(props: { items: Item[]; market: Market }) {
  return (
    <div class="market-grid">
      <For each={props.items} keyed={itemID}>
        {(item) => <ItemRow item={item()} market={props.market} />}
      </For>
    </div>
  );
}

/** A section: its title with View all when there is more, or a control, over its grid. */
function MarketSection(props: { title: string; items: Item[]; market: Market; accessory?: JSX.Element; onViewAll?: () => void }) {
  return (
    <div class="market-section">
      <div class="market-section-header">
        <span class="market-section-title">{props.title}</span>
        <Show when={props.onViewAll} fallback={props.accessory}>
          <HoverButton title={L("View all")} onClick={() => props.onViewAll?.()} />
        </Show>
      </div>
      <Grid items={props.items} market={props.market} />
    </div>
  );
}

/** A page's own title, large, with an optional control at its trailing end. */
function PageTitle(props: { text: string; accessory?: JSX.Element }) {
  return (
    <div class="market-page-title">
      <span>{props.text}</span>
      <Show when={props.accessory}>{props.accessory}</Show>
    </div>
  );
}

/** A line of secondary text on the rows' leading edge, for loading, empty, and notes. */
function StatusLine(props: { text: string }) {
  return <div class="market-status">{props.text}</div>;
}

/** All, Plugins, and Bots, when both kinds are there. */
function KindFilter(props: { value: number; onChange: (value: number) => void }) {
  return (
    <Segmented
      label={L("Filter results")}
      options={[
        { value: 0, label: L("All") },
        { value: 1, label: L("Plugins") },
        { value: 2, label: L("Bots") },
      ]}
      value={props.value}
      onChange={props.onChange}
    />
  );
}

function filtered(items: Item[], kind: number): { shown: Item[]; mixed: boolean } {
  const plugins = items.filter((item) => item.kind === "plugin");
  const bots = items.filter((item) => item.kind === "bot");
  const mixed = plugins.length > 0 && bots.length > 0;
  if (!mixed) return { shown: items, mixed };
  return { shown: kind === 1 ? plugins : kind === 2 ? bots : items, mixed };
}

// MARK: - Pages

/** The first page: its title with what the Runner has installed, the search, and then the featured
 * plugins and bots and each category, or what the search found. */
function HomePage(props: { market: Market }) {
  const market = props.market;
  const [query, setQuery] = createSignal("");
  const [kind, setKind] = createSignal(0);
  let search: HTMLInputElement | undefined;
  onSettled(() => search?.focus());

  const section = (title: string, items: Item[], all: (catalog: Marketplace) => Item[], listTitle?: string) => ({
    title,
    items: items.slice(0, previewCount),
    viewAll: all(market.catalog()).length > Math.min(items.length, previewCount) ? () => market.show({ kind: "list", title: listTitle ?? title, items: all }) : undefined,
  });

  /** Featured Plugins, Featured Bots, then a section per category, plugins before bots. A featured
   * section's View all lists every plugin or every bot, the featured ones first. A category leads
   * with what the featured sections do not already show, and one with both kinds keeps half its rows
   * for bots, so its plugins never hide them. */
  const homeSections = createMemo(() => {
    const catalog = market.catalog();
    const sections: ReturnType<typeof section>[] = [];
    const featuredPlugins = catalog.plugins.filter((plugin) => plugin.isFeatured);
    if (featuredPlugins.length > 0) {
      sections.push(
        section(
          L("Featured Plugins"),
          featuredPlugins.map((plugin): Item => ({ kind: "plugin", plugin })),
          (all) => [...all.plugins.filter((plugin) => plugin.isFeatured), ...all.plugins.filter((plugin) => !plugin.isFeatured)].map((plugin): Item => ({ kind: "plugin", plugin })),
          L("Plugins"),
        ),
      );
    }
    const featuredBots = catalog.bots.filter((bot) => bot.isFeatured);
    if (featuredBots.length > 0) {
      sections.push(
        section(
          L("Featured Bots"),
          featuredBots.map((bot): Item => ({ kind: "bot", bot })),
          (all) => [...all.bots.filter((bot) => bot.isFeatured), ...all.bots.filter((bot) => !bot.isFeatured)].map((bot): Item => ({ kind: "bot", bot })),
          L("Bots"),
        ),
      );
    }
    const shownPlugins = new Set(featuredPlugins.slice(0, previewCount).map((plugin) => plugin.id));
    const shownBots = new Set(featuredBots.slice(0, previewCount).map((bot) => bot.id));
    for (const key of categoryKeys(allItems(catalog))) {
      const plugins = catalog.plugins.filter((plugin) => plugin.category === key);
      const bots = catalog.bots.filter((bot) => bot.category === key);
      const freshPlugins = [...plugins.filter((plugin) => !shownPlugins.has(plugin.id)), ...plugins.filter((plugin) => shownPlugins.has(plugin.id))];
      const freshBots = [...bots.filter((bot) => !shownBots.has(bot.id)), ...bots.filter((bot) => shownBots.has(bot.id))];
      const botRows = Math.min(bots.length, Math.max(previewCount - plugins.length, previewCount / 2));
      const preview: Item[] = [
        ...freshPlugins.slice(0, previewCount - botRows).map((plugin): Item => ({ kind: "plugin", plugin })),
        ...freshBots.slice(0, botRows).map((bot): Item => ({ kind: "bot", bot })),
      ];
      sections.push(section(categoryTitle(key), preview, (all) => allItems(all).filter((item) => itemCategory(item) === key)));
    }
    return sections;
  });

  const results = createMemo(() => {
    const words = query().trim().split(/\s+/).filter((word) => word !== "");
    return allItems(market.catalog()).filter((item) => matches(item, words));
  });

  const installedCount = () => market.installedPlugins().length;
  return (
    <>
      <div class="market-home-header">
        <span class="market-home-title">{L("Marketplace")}</span>
        <Show when={market.runner()}>
          {(on) => (
            <HoverButton title={L("%d installed", installedCount())} trailingSymbol="chevron.right" tooltip={L("Plugins on %@", on().name)} onClick={() => market.show({ kind: "installed" })} />
          )}
        </Show>
      </div>
      <SearchField ref={(element) => (search = element)} class="large market-search" value={query()} placeholder={L("Search plugins and bots")} onInput={setQuery} />
      <div class="market-body">
        <Show
          when={!(allItems(market.catalog()).length === 0 && market.loading() !== "loaded")}
          fallback={
            <Show when={market.loading() === "failed"} fallback={<StatusLine text={L("Loading the marketplace…")} />}>
              <div class="market-failed">
                <StatusLine text={L("The marketplace isn't available right now.")} />
                <div class="market-retry">
                  <Button onClick={() => market.load()}>{L("Try Again")}</Button>
                </div>
              </div>
            </Show>
          }
        >
          <Show
            when={query().trim() !== ""}
            fallback={
              <Show when={homeSections().length > 0} fallback={<StatusLine text={L("Nothing in the marketplace yet.")} />}>
                <For each={homeSections()}>
                  {(each) => <MarketSection title={each.title} items={each.items} market={market} onViewAll={each.viewAll} />}
                </For>
              </Show>
            }
          >
            <Show when={results().length > 0} fallback={<StatusLine text={L("No results match “%@”", query().trim())} />}>
              <MarketSection
                title={L("Results")}
                items={filtered(results(), kind()).shown}
                market={market}
                accessory={filtered(results(), kind()).mixed ? <KindFilter value={kind()} onChange={setKind} /> : undefined}
              />
            </Show>
          </Show>
        </Show>
      </div>
    </>
  );
}

/** Everything in one of the home page's sections: the featured plugins or bots, or a category. */
function ListPage(props: { market: Market; heading: string; items: (catalog: Marketplace) => Item[] }) {
  const [kind, setKind] = createSignal(0);
  const list = () => filtered(props.items(props.market.catalog()), kind());
  return (
    <div class="market-stack tight">
      <PageTitle text={props.heading} accessory={list().mixed ? <KindFilter value={kind()} onChange={setKind} /> : undefined} />
      <Show when={list().shown.length > 0} fallback={<StatusLine text={L("Nothing here yet.")} />}>
        <Grid items={list().shown} market={props.market} />
      </Show>
    </div>
  );
}

/** The plugins the picked Runner has. Each opens its own sheet: its sign-in, its setup, and Remove. */
function InstalledPage(props: { market: Market }) {
  const market = props.market;
  return (
    <Show
      when={market.runner()}
      fallback={
        <div class="market-stack tight">
          <PageTitle text={L("Installed")} />
          <StatusLine text={L("Pair a Runner first.")} />
        </div>
      }
    >
      {(on) => (
        <div class="market-stack tight">
          <PageTitle text={L("Plugins on %@", on().name)} />
          <StatusLine text={L("Every bot on %@ can use these. Sign-ins and keys stay on %@.", on().name, on().name)} />
          <Section title={L("Installed")} style="heading">
            <Show when={market.installedPlugins().length > 0} fallback={<NoteRow text={L("Nothing installed yet. Find plugins in the marketplace.")} />}>
              <For each={market.installedPlugins()} keyed={(plugin) => plugin.id}>
                {(plugin) => <PluginRow plugin={plugin()} tooltip={L("Open %@", plugin().name)} onClick={() => market.manage(plugin().id)} />}
              </For>
            </Show>
          </Section>
        </div>
      )}
    </Show>
  );
}

function hostOf(link: string | undefined): string | undefined {
  if (!link) return undefined;
  try {
    return new URL(link).host || undefined;
  } catch {
    return undefined;
  }
}

/** A card in System Settings' look, its count beside the title. */
function Card(props: { title: string; count?: number; children: JSX.Element }) {
  return (
    <Section title={props.title} style="heading" accessory={props.count !== undefined ? <span class="market-card-count">{props.count}</span> : undefined}>
      {props.children}
    </Section>
  );
}

/** A plugin in full: what it does, its servers and skills, the setup it asks for, and who makes it,
 * with Add, or its state and Manage once the Runner has it. */
function PluginPage(props: { market: Market; pluginID: string }) {
  const market = props.market;
  const plugin = () => market.plugin(props.pluginID);
  return (
    <Show
      when={plugin()}
      fallback={<StatusLine text={market.loading() === "loaded" ? L("This plugin is no longer in the marketplace.") : L("Loading…")} />}
    >
      {(current) => {
        const byline = () => [current().author !== "" ? L("by %@", current().author) : "", current().category !== "" ? categoryTitle(current().category) : ""].filter((part) => part !== "").join(" · ");
        const installed = () => market.installedPlugin(current().id);
        return (
          <div class="market-stack loose">
            <div class="market-detail-header">
              <div class="market-detail-top">
                <PluginIcon pluginID={current().id} symbol={pluginSymbol(current())} size={56} />
                <div class="market-detail-titles">
                  <span class="market-detail-name">{current().name}</span>
                  <span class="market-detail-byline">{byline()}</span>
                </div>
                <div class="market-detail-actions">
                  <Show when={current().homepage && hostOf(current().homepage)}>
                    <Button tooltip={current().homepage} onClick={() => void host.openExternal(current().homepage!)}>
                      <span class="button-with-symbol">
                        {L("Website")}
                        <Icon name="arrow.up.right" size={11} strokeWidth={2.4} />
                      </span>
                    </Button>
                  </Show>
                  <Show
                    when={!market.installing()[current().id]}
                    fallback={<Spinner size={16} />}
                  >
                    <Show
                      when={installed()}
                      fallback={
                        <Button
                          kind="primary"
                          large
                          disabled={!market.runner()}
                          tooltip={market.runner() ? L("Install %@ on %@, for every bot there", current().name, market.runner()!.name) : L("Pair a Runner first.")}
                          onClick={() => market.install(current())}
                        >
                          {L("Add")}
                        </Button>
                      }
                    >
                      <Button onClick={() => market.manage(current().id)}>{L("Manage…")}</Button>
                    </Show>
                  </Show>
                </div>
              </div>
              <div class="market-detail-description selectable">{current().description}</div>
            </div>
            <Show when={market.runner() && installed()}>
              <Card title={L("On %@", market.runner()!.name)}>
                <StatusRow
                  symbol={deviceSymbol(market.runner()!)}
                  title={installed()!.detail}
                  subtitle={L("Every bot on %@ can use it.", market.runner()!.name)}
                  actionTitle={installed()!.state === "needs_auth" ? L("Connect") : installed()!.state === "needs_setup" ? L("Set Up") : undefined}
                  onAction={() => market.manage(current().id)}
                />
              </Card>
            </Show>
            <Show when={current().servers.length > 0}>
              <Card title={L("Servers")} count={current().servers.length}>
                <For each={current().servers}>
                  {(server) => (
                    <StatusRow
                      symbol={server.isRemote ? "network" : "terminal"}
                      title={server.name}
                      subtitle={
                        server.isRemote
                          ? [hostOf(server.address) ?? server.address, server.signsIn ? L("Signs in with your account") : ""].filter((part) => part !== "").join(" · ")
                          : L("Runs on the Runner: %@", server.address)
                      }
                    />
                  )}
                </For>
              </Card>
            </Show>
            <Show when={current().skills.length > 0}>
              <Card title={L("Skills")} count={current().skills.length}>
                <For each={current().skills}>{(skill) => <StatusRow symbol="cube" title={skill.name} subtitle={skill.description} />}</For>
              </Card>
            </Show>
            <Show when={current().variables.length > 0}>
              <Card title={Lc("Setup", "plugin variables")}>
                <For each={current().variables}>
                  {(variable) => (
                    <StatusRow
                      symbol={variable.secret ? "key" : "slider.horizontal.3"}
                      title={variable.name}
                      subtitle={variable.description}
                      state={variable.required ? L("Required") : L("Optional")}
                    />
                  )}
                </For>
              </Card>
            </Show>
            <Show when={current().author !== "" || current().category !== "" || hostOf(current().homepage)}>
              <Card title={L("Information")}>
                <Show when={current().author !== ""}>
                  <KeyValueRow label={L("Developer")} value={current().author} />
                </Show>
                <Show when={current().category !== ""}>
                  <KeyValueRow label={L("Category")} value={categoryTitle(current().category)} />
                </Show>
                <Show when={hostOf(current().homepage)}>{(site) => <KeyValueRow label={L("Website")} value={site()} />}</Show>
              </Card>
            </Show>
          </div>
        );
      }}
    </Show>
  );
}

type Part = "instructions" | "memories" | "routines" | "plugins";

function partTitle(part: Part): string {
  switch (part) {
    case "instructions":
      return L("Instructions");
    case "memories":
      return L("Memories");
    case "routines":
      return L("Routines");
    case "plugins":
      return L("Plugins");
  }
}

function partSubtitle(part: Part): string {
  switch (part) {
    case "instructions":
      return L("How this bot should work");
    case "memories":
      return L("Facts it already knows");
    case "routines":
      return L("Jobs that run on their own");
    case "plugins":
      return L("What it works with");
  }
}

/** A bot in full: who it is with Add Bot, then its instructions, what it already knows, its
 * routines, and its plugins, one at a time from the list beside them. */
function BotPage(props: { market: Market; templateID: string }) {
  const market = props.market;
  const [picked, setPicked] = createSignal<Part>("instructions");
  const template = () => market.catalog().bots.find((bot) => bot.id === props.templateID);
  return (
    <Show when={template()} fallback={<StatusLine text={market.loading() === "loaded" ? L("This bot is no longer in the marketplace.") : L("Loading…")} />}>
      {(current) => {
        const parts = (): Part[] => [
          "instructions",
          ...(current().memory.length > 0 ? (["memories"] as Part[]) : []),
          ...(current().routines.length > 0 ? (["routines"] as Part[]) : []),
          ...(current().plugins.length > 0 ? (["plugins"] as Part[]) : []),
        ];
        const selected = () => (parts().includes(picked()) ? picked() : "instructions");
        return (
          <div class="market-stack">
            <div class="market-bot-header">
              <Avatar content={{ kind: "bot", symbolName: current().symbolName, accent: current().accent }} size={64} />
              <div class="market-bot-title-row">
                <div class="market-detail-titles">
                  <span class="market-bot-name">{current().name}</span>
                  <Show when={current().author !== ""}>
                    <span class="market-detail-byline">{L("By %@", current().author)}</span>
                  </Show>
                </div>
                <Button kind="primary" large disabled={!market.runner()} onClick={() => market.add(current())}>
                  {L("Add Bot")}
                </Button>
              </div>
              <div class="market-bot-summary">{current().summary}</div>
              <div class="market-bot-note">
                {market.runner()
                  ? L("Adds %@ to %@. Its routines start paused, and it asks before it installs a plugin.", current().name, market.runner()!.name)
                  : L("Pair a Runner first: a bot runs on a computer.")}
              </div>
            </div>
            <div class="market-divider" />
            <div class="market-bot-body">
              <div class="market-rail">
                <For each={parts()}>
                  {(part) => (
                    <button class={["market-rail-item", { selected: part === selected() }]} aria-pressed={part === selected() ? "true" : "false"} onClick={() => setPicked(part)}>
                      <span class="market-rail-title">{partTitle(part)}</span>
                      <span class="market-rail-subtitle">{partSubtitle(part)}</span>
                    </button>
                  )}
                </For>
              </div>
              <div class={["market-panel", selected()]}>
                <BotPanel part={selected()} template={current()} market={market} />
              </div>
            </div>
          </div>
        );
      }}
    </Show>
  );
}

/** What the panel beside the list shows for the part picked there. */
function BotPanel(props: { part: Part; template: BotTemplate; market: Market }) {
  return <>{panelContent(props.part, props.template, props.market)}</>;
}

function panelContent(part: Part, template: BotTemplate, market: Market): JSX.Element {
  const props = { template };
  switch (part) {
    case "instructions":
      return <p class="market-paragraph primary selectable">{props.template.description}</p>;
    case "memories":
      return <For each={props.template.memory}>{(fact) => <p class="market-paragraph primary selectable">{fact}</p>}</For>;
    case "routines":
      return (
        <>
          <For each={props.template.routines}>
            {(routine) => (
              <div class="market-routine">
                <span class="market-routine-name">{routine.name}</span>
                <span class="market-routine-schedule">{Format.schedule(routine.scheduleText)}</span>
                <p class="market-paragraph selectable">{routine.prompt}</p>
              </div>
            )}
          </For>
          <p class="market-paragraph faint">{L("They start paused. %@ asks whether to turn them on.", props.template.name)}</p>
        </>
      );
    case "plugins":
      return (
        <>
          <For each={props.template.plugins.flatMap((id) => market.plugin(id) ?? [])}>
            {(plugin) => {
              const installed = () => market.installedPlugin(plugin.id);
              const state = () => {
                const current = installed();
                if (!current) return { text: L("Not installed"), color: "var(--label-2)" };
                return current.state === "ready" ? { text: L("Installed"), color: "var(--green)" } : { text: current.detail, color: pluginStateColor(current.state) };
              };
              return (
                <MarketRow
                  onCard
                  media={<PluginIcon pluginID={plugin.id} symbol={pluginSymbol(plugin)} size={36} />}
                  title={plugin.name}
                  subtitle={plugin.description}
                  accessory={
                    <span class="market-state" style={{ color: state().color }}>
                      {state().text}
                    </span>
                  }
                  onOpen={() => market.show({ kind: "plugin", id: plugin.id })}
                />
              );
            }}
          </For>
          <Show when={market.runner()}>
            {(on) => <p class="market-paragraph faint">{L("%@ asks before it installs one on %@.", props.template.name, on().name)}</p>}
          </Show>
        </>
      );
  }
}
