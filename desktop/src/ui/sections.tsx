// Titled cards of rows, after the macOS app's DetailViews: the inspector's captioned, bordered card
// and System Settings' heading and plain fill, with dividers between the rows (edge to edge under
// a caption, between the rows' text margins under a heading), and the rows they hold.

import { createSignal, For, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { L } from "../l10n";
import type { Bot, InstalledPlugin } from "../model/models";
import { pluginSymbol, pluginStateColor } from "../model/models";
import { Avatar, botAvatar } from "./avatar";
import { Button, LinkButton, PopUpButton, Switch, type PopUpOption } from "./controls";
import { Icon } from "./icons";
import { showTextPopover } from "./overlay";
import { pluginMarks } from "./pluginMarks";

export function Section(props: {
  title: string;
  style?: "caption" | "heading";
  accessory?: JSX.Element;
  children: JSX.Element;
  /** The label a search result scrolls to: the section's title. */
  class?: string;
  hidden?: boolean;
}) {
  return (
    <section class={["section", props.style ?? "caption", props.class]} hidden={props.hidden} data-label={props.title}>
      <div class="section-header">
        <span class="section-title">{(props.style ?? "caption") === "caption" ? props.title.toUpperCase() : props.title}</span>
        <Show when={props.accessory}>
          <span class="section-accessory">{props.accessory}</span>
        </Show>
      </div>
      <div class="section-card">{props.children}</div>
    </section>
  );
}

/** Key on the left, a value on the right that wraps and can be selected. */
export function KeyValueRow(props: { label: string; value: string; monospaced?: boolean; tint?: string; tooltip?: string }) {
  return (
    <div class="row key-value-row" data-label={props.label} title={props.tooltip}>
      <span class="row-key">{props.label}</span>
      <span class={["row-value", "selectable", { mono: !!props.monospaced }]} style={props.tint ? { color: props.tint } : undefined}>
        {props.value}
      </span>
    </div>
  );
}

/** A bot, as the inspector, the Device pane, and pickers list them. */
export function BotRow(props: {
  bot: Bot;
  detail: string;
  accessorySymbol?: string;
  accessoryTooltip?: string;
  onAccessory?: () => void;
  onClick?: () => void;
  onAvatarClick?: () => void;
  working?: boolean;
}) {
  return (
    <div class={["row", "bot-row", { clickable: !!props.onClick }]} data-label={props.bot.name} onClick={() => props.onClick?.()}>
      <Avatar
        content={botAvatar(props.bot)}
        size={28}
        working={props.working}
        onClick={
          props.onAvatarClick
            ? () => {
                props.onAvatarClick?.();
              }
            : undefined
        }
      />
      <div class="row-text">
        <span class="row-title truncate">{props.bot.name}</span>
        <span class="row-subtitle truncate">{props.detail}</span>
      </div>
      <Show when={props.accessorySymbol}>
        {(symbol) => (
          <button
            class="row-accessory"
            title={props.accessoryTooltip}
            aria-label={props.accessoryTooltip}
            onClick={(event) => {
              event.stopPropagation();
              props.onAccessory?.();
            }}
          >
            <Icon name={symbol()} size={14} strokeWidth={1.8} />
          </button>
        )}
      </Show>
    </div>
  );
}

/** A plugin's real mark on a white tile, which stays white in dark mode so a dark brand color
 * still reads, or its symbol when it has no mark. */
export function PluginTile(props: { pluginID: string; symbol: string; size: number }) {
  const mark = () => pluginMarks[props.pluginID];
  return (
    <Show
      when={mark()}
      fallback={<Icon name={props.symbol} size={Math.round(props.size * 0.8)} strokeWidth={1.8} class="plugin-symbol" />}
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

export interface StatusRowProps {
  symbol: string;
  pluginID?: string;
  title: string;
  subtitle?: string;
  state?: string;
  /** With a symbol, the state's words are its tooltip. */
  stateSymbol?: string;
  stateColor?: string;
  /** What a state that needs something says in full, a click on the state away. */
  stateDetail?: string;
  actionTitle?: string;
  destructive?: boolean;
  onAction?: () => void;
  onClick?: () => void;
  tooltip?: string;
}

/** A leading symbol, a title over a subtitle, and a trailing state, as words or a symbol, or an
 * action button. */
export function StatusRow(props: StatusRowProps) {
  let stateElement: HTMLElement | undefined;
  const showDetail = (event: MouseEvent) => {
    if (!props.stateDetail) return;
    event.stopPropagation();
    if (stateElement) showTextPopover(stateElement, props.stateDetail);
  };
  return (
    <div class={["row", "status-row", { clickable: !!props.onClick }]} data-label={props.title} title={props.tooltip} onClick={() => props.onClick?.()}>
      <span class="row-icon">
        <Show when={props.pluginID} fallback={<Icon name={props.symbol} size={16} strokeWidth={1.7} />}>
          <PluginTile pluginID={props.pluginID!} symbol={props.symbol} size={18} />
        </Show>
      </span>
      <div class="row-text">
        <span class="row-title">{props.title}</span>
        <Show when={props.subtitle}>
          <span class="row-subtitle wrap">{props.subtitle}</span>
        </Show>
      </div>
      <Show
        when={props.actionTitle}
        fallback={
          <Show when={props.state}>
            <Show
              when={props.stateSymbol}
              fallback={
                <span
                  ref={(el) => (stateElement = el)}
                  class={["row-state", { clickable: !!props.stateDetail }]}
                  style={{ color: props.stateColor ?? "var(--label-2)" }}
                  onClick={showDetail}
                >
                  {props.state}
                </span>
              }
            >
              {(symbol) => (
                <span
                  ref={(el) => (stateElement = el)}
                  class={["row-state-symbol", { clickable: !!props.stateDetail }]}
                  style={{ color: props.stateColor ?? "var(--label-2)" }}
                  title={props.state}
                  aria-label={props.state}
                  onClick={showDetail}
                >
                  <Icon name={symbol()} size={14} strokeWidth={2.4} />
                </span>
              )}
            </Show>
          </Show>
        }
      >
        <Button
          small
          kind={props.destructive ? "destructive" : "default"}
          onClick={(event) => {
            event.stopPropagation();
            props.onAction?.();
          }}
        >
          {props.actionTitle}
        </Button>
      </Show>
    </div>
  );
}

/** A plugin and its state as a symbol: a check when it is ready, an exclamation mark when it needs
 * something, whose words are its tooltip and a click away. */
export function PluginRow(props: { plugin: InstalledPlugin; onClick?: () => void; tooltip?: string }) {
  const ready = () => props.plugin.state === "ready";
  return (
    <StatusRow
      symbol={pluginSymbol(props.plugin)}
      pluginID={props.plugin.id}
      title={props.plugin.name}
      subtitle={props.plugin.description}
      state={ready() ? L("Ready") : props.plugin.detail}
      stateSymbol={ready() ? "checkmark" : "exclamationmark.circle.fill"}
      stateColor={ready() ? "var(--green)" : pluginStateColor("needs_setup")}
      stateDetail={ready() ? undefined : props.plugin.detail}
      onClick={props.onClick}
      tooltip={props.tooltip}
    />
  );
}

/** Label on the left, a pop-up on the right. */
export function PopUpRow<T>(props: { label: string; options: PopUpOption<T>[]; value: T; onChange: (value: T) => void }) {
  return (
    <div class="row popup-row" data-label={props.label}>
      <span class="row-key">{props.label}</span>
      <PopUpButton options={props.options} value={props.value} onChange={props.onChange} style="bordered" class="row-popup" />
    </div>
  );
}

/** Key on the left, a status value on the right, and a text action after it. A monospaced value is
 * something to copy (a sign-in code), so it is selectable. */
export function ActionRow(props: {
  label: string;
  value?: string;
  tint?: string;
  actionTitle?: string;
  monospaced?: boolean;
  tooltip?: string;
  onAction?: () => void;
}) {
  return (
    <div class="row action-row" data-label={props.label} title={props.tooltip}>
      <span class="row-key">{props.label}</span>
      <span class={["row-value", "truncate", { mono: !!props.monospaced, selectable: !!props.monospaced }]} style={props.tint ? { color: props.tint } : undefined}>
        {props.value ?? ""}
      </span>
      <Show when={props.actionTitle}>
        <LinkButton onClick={() => props.onAction?.()}>{props.actionTitle}</LinkButton>
      </Show>
    </div>
  );
}

/** A key and an action on the first line, with a wrapping two-line preview under them. With no
 * preview, the key and the action sit alone on one line. */
export function SummaryActionRow(props: { label: string; value: string; actionTitle: string; onAction: () => void }) {
  return (
    <div class={["row", "summary-row", { empty: props.value === "" }]} data-label={props.label}>
      <div class="summary-line">
        <span class="row-key">{props.label}</span>
        <LinkButton onClick={() => props.onAction()}>{props.actionTitle}</LinkButton>
      </div>
      <Show when={props.value !== ""}>
        <span class="summary-preview">{props.value}</span>
      </Show>
    </div>
  );
}

/** Key on the left, an editable value on the right that looks like a value until it is clicked,
 * and commits when editing ends (Return, Tab, or focus leaving). While the user types, the value
 * from the model waits. */
export function EditableRow(props: {
  label: string;
  value: string;
  placeholder?: string;
  monospaced?: boolean;
  alignRight?: boolean;
  onCommit: (value: string) => void;
}) {
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal("");
  let committed = false;
  return (
    <div class="row editable-row" data-label={props.label}>
      <span class="row-key fixed">{props.label}</span>
      <input
        class={["row-field", { mono: !!props.monospaced, right: !!props.alignRight }]}
        value={editing() ? draft() : props.value}
        placeholder={props.placeholder}
        spellcheck={false}
        onFocus={(event) => {
          committed = false;
          setDraft(event.currentTarget.value);
          setEditing(true);
        }}
        onInput={(event) => setDraft(event.currentTarget.value)}
        onBlur={(event) => {
          setEditing(false);
          if (!committed) props.onCommit(event.currentTarget.value.trim());
        }}
        onKeyDown={(event) => {
          if (event.key === "Enter" && !event.isComposing) {
            committed = true;
            props.onCommit(event.currentTarget.value.trim());
            event.currentTarget.blur();
          } else if (event.key === "Escape") {
            committed = true;
            event.currentTarget.value = props.value;
            event.currentTarget.blur();
          }
        }}
      />
    </div>
  );
}

/** A row with an icon for its state, a title over a detail line, and a switch: a routine that
 * pauses or resumes. A click anywhere but the switch opens its details. */
export function SwitchRow(props: {
  symbol: string;
  tint: string;
  title: string;
  detail: string;
  isOn: boolean;
  toggleTooltip: string;
  tooltip?: string;
  onToggle: (on: boolean) => void;
  onClick?: () => void;
}) {
  return (
    <div class={["row", "switch-row", { clickable: !!props.onClick }]} data-label={props.title} title={props.tooltip} onClick={() => props.onClick?.()}>
      <span class="row-icon" style={{ color: props.tint }}>
        <Icon name={props.symbol} size={15} strokeWidth={1.8} />
      </span>
      <div class="row-text">
        <span class="row-title truncate">{props.title}</span>
        <span class="row-subtitle truncate">{props.detail}</span>
      </div>
      <Switch small checked={props.isOn} tooltip={props.toggleTooltip} onChange={props.onToggle} />
    </div>
  );
}

/** A sentence inside a card, for an empty state. */
export function NoteRow(props: { text: string }) {
  return (
    <div class="row note-row">
      <span>{props.text}</span>
    </div>
  );
}

/** Key on the left, any control on the right. */
export function AccessoryRow(props: { label: string; children: JSX.Element; tooltip?: string }) {
  return (
    <div class="row accessory-row" data-label={props.label} title={props.tooltip}>
      <span class="row-label">{props.label}</span>
      <span class="row-control">{props.children}</span>
    </div>
  );
}

/** Rows of a list, each keyed by its id, so a row that stays keeps its element. */
export function Rows<T>(props: { each: T[]; key: (item: T) => string; children: (item: () => T) => JSX.Element }) {
  return (
    <For each={props.each} keyed={(item) => props.key(item)}>
      {(item) => props.children(item)}
    </For>
  );
}
