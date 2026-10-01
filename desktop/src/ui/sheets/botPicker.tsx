// Picking bots, after the macOS app's NewGroupChatViewController and BotPickerViewController: one to
// six bots for a new group chat, whose members change later, or one bot to add to a chat. Direct
// chats need no picker because every bot gets one when it is made.

import { createSignal, For, Show } from "solid-js";
import { host } from "../../host";
import { L } from "../../l10n";
import { maxGroupBots, providerName, type Bot } from "../../model/models";
import { store } from "../../model/store";
import { Avatar, botAvatar } from "../avatar";
import { TextField } from "../controls";
import { Icon } from "../icons";
import { presentSheet, Sheet } from "../overlay";
import { Section } from "../sections";

/** A bot to pick: a check, its avatar, its name over its provider and Runner, and "offline" when the
 * Runner is. */
function SelectableBotRow(props: { bot: Bot; selected: boolean; enabled: boolean; onToggle: () => void }) {
  const runner = () => store.device(props.bot.runnerID);
  return (
    <div
      class={["row", "selectable-bot-row", { disabled: !props.enabled }]}
      role="checkbox"
      aria-checked={props.selected ? "true" : "false"}
      aria-disabled={props.enabled ? "false" : "true"}
      data-label={props.bot.name}
      onClick={() => {
        if (props.enabled) props.onToggle();
      }}
    >
      <span class={["selectable-check", { on: props.selected }]}>
        <Icon name={props.selected ? "checkmark.circle.fill" : "circle"} size={16} strokeWidth={1.8} />
      </span>
      <Avatar content={botAvatar(props.bot)} size={28} />
      <div class="row-text">
        <span class="row-title truncate">{props.bot.name}</span>
        <span class="row-subtitle truncate">{`${providerName(props.bot.provider, store.providers)} · ${runner()?.name ?? L("unassigned")}`}</span>
      </div>
      <Show when={runner()?.status === "offline"}>
        <span class="selectable-offline">{L("offline")}</span>
      </Show>
    </div>
  );
}

/** `onCreate` gets the picked bots and the name, if one was typed. */
export function presentNewGroupChat(onCreate: (botIDs: string[], title: string | null) => void): void {
  presentSheet((dismiss) => {
    const bots = store.bots;
    const [selected, setSelected] = createSignal<string[]>(bots[0] ? [bots[0].id] : []);
    let name = "";
    const toggle = (id: string) => {
      const current = selected();
      if (current.includes(id)) setSelected(current.filter((each) => each !== id));
      else if (current.length < maxGroupBots) setSelected([...current, id]);
      else void host.beep();
    };
    const create = () => {
      if (selected().length === 0) return;
      const title = name.trim();
      onCreate(selected(), title === "" ? null : title);
      dismiss();
    };
    return (
      <Sheet
        title={L("New Group Chat")}
        subtitle={L("Pick up to six bots to talk with together.")}
        width={440}
        confirm={L("Create")}
        confirmDisabled={selected().length === 0}
        onConfirm={create}
        onCancel={dismiss}
      >
        <Section title={L("Bots")} class="picker-list">
          <For each={bots} keyed={(bot) => bot.id}>
            {(bot) => (
              <SelectableBotRow
                bot={bot()}
                selected={selected().includes(bot().id)}
                enabled={selected().includes(bot().id) || selected().length < maxGroupBots}
                onToggle={() => toggle(bot().id)}
              />
            )}
          </For>
        </Section>
        <TextField value="" placeholder={L("Group name (optional)")} onInput={(value) => (name = value)} />
      </Sheet>
    );
  });
}

/** One bot to add to a chat, from `bots`. */
export function presentBotPicker(title: string, bots: Bot[], onPick: (botID: string) => void): void {
  presentSheet((dismiss) => {
    const [selected, setSelected] = createSignal<string | null>(bots[0]?.id ?? null);
    const add = () => {
      const id = selected();
      if (id === null) return;
      onPick(id);
      dismiss();
    };
    return (
      <Sheet title={title} subtitle={L("A group chat holds up to six bots.")} width={420} confirm={L("Add")} confirmDisabled={selected() === null} onConfirm={add} onCancel={dismiss}>
        <Section title={L("Available")} class="picker-list">
          <For each={bots} keyed={(bot) => bot.id}>
            {(bot) => <SelectableBotRow bot={bot()} selected={selected() === bot().id} enabled onToggle={() => setSelected(bot().id)} />}
          </For>
        </Section>
      </Sheet>
    );
  });
}
