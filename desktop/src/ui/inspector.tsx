// The pane beside a chat, after the macOS app's InspectorViewController: the bots in the chat, a
// group's name and description, and for a DM the bot's profile, what it runs with (provider, model, thinking, the credential, and what
// the turns used), its memory, its routines, the plugins on its Runner, and where turns run.

import { createEffect, createMemo, For, onSettled, Show } from "solid-js";
import { files } from "../host";
import { L } from "../l10n";
import * as Format from "../model/format";
import {
  canAddBot,
  canRemoveBot,
  chatOwner,
  contextSummary,
  deviceSymbol,
  isDM,
  isGroup,
  memoryBudgetSummary,
  memoryFilesSummary,
  providerModels,
  providerName,
  routineDetail,
  spendSummary,
  thinkingLevels,
  withCustomModels,
  type Bot,
  type BotMemory,
  type Chat,
} from "../model/models";
import { onStoreEvent, track } from "../model/reactive";
import { errorText, store } from "../model/store";
import { addBotToChat, presentMarketplace } from "./actions";
import { box } from "./box";
import { Button } from "./controls";
import { popupMenu, separator } from "./menu";
import { chatActions, openDevice } from "./root";
import { ActionRow, BotRow, EditableRow, KeyValueRow, NoteRow, PluginRow, PopUpRow, Section, StatusRow, SummaryActionRow, SwitchRow } from "./sections";
import { presentBotDescription, presentGroupDescription } from "./sheets/description";
import { presentBotLook } from "./sheets/botLook";
import { presentConnectProvider } from "./sheets/connectProvider";
import { presentMemory } from "./sheets/memory";
import { presentPlugin } from "./sheets/plugin";
import { presentRoutine } from "./sheets/routine";

// What each bot's Runner last said about its memory, refreshed when the pane opens on a chat and
// after every turn in it; why the last fetch failed (the Runner is offline, or did not answer); and
// the fetches on their way. Kept while the pane is closed, as the macOS inspector keeps them.
const memoryByBot = box<Record<string, BotMemory>>({});
const memoryErrors = box<Record<string, string>>({});
const memoryFetches = box<Record<string, true>>({});

/** Asks the CLI for the bot's memory; the section redraws when it answers. */
function refreshMemory(botID: string): void {
  if (store.isMock && memoryByBot.get()[botID]) return;
  if (memoryFetches.get()[botID]) return;
  memoryFetches.set({ ...memoryFetches.get(), [botID]: true });
  store
    .botMemory(botID)
    .then(
      (memory) => {
        memoryByBot.set({ ...memoryByBot.get(), [botID]: memory });
        const errors = { ...memoryErrors.get() };
        delete errors[botID];
        memoryErrors.set(errors);
      },
      (error) => memoryErrors.set({ ...memoryErrors.get(), [botID]: errorText(error) }),
    )
    .finally(() => {
      const fetches = { ...memoryFetches.get() };
      delete fetches[botID];
      memoryFetches.set(fetches);
    });
}

/** Asks for the memory of the bot whose DM is showing. */
function refreshShownMemory(chatID: string): void {
  const chat = store.chat(chatID);
  if (!chat || !isDM(chat)) return;
  const bot = store.botsIn(chat)[0];
  if (bot) refreshMemory(bot.id);
}

/** The Runner's path joined with a file name, in the Runner's own separators. */
function joinPath(directory: string, name: string): string {
  const separator = directory.includes("\\") && !directory.includes("/") ? "\\" : "/";
  return directory.endsWith(separator) ? directory + name : directory + separator + name;
}

const sameBots = (a: Bot[], b: Bot[]) => a.length === b.length && a.every((bot, index) => bot === b[index]);

export function Inspector(props: { chatID: string }) {
  const chat = createMemo(() => {
    track.chats();
    track.chat(props.chatID);
    return store.chat(props.chatID);
  });
  const members = createMemo(
    () => {
      track.roster();
      const current = chat();
      return current ? store.botsIn(current) : [];
    },
    { equals: sameBots },
  );
  /** A direct chat is one bot, so its profile, provider, and model are edited right here. */
  const single = createMemo(() => {
    const current = chat();
    const bots = members();
    return current && isDM(current) && bots.length === 1 ? bots[0] : undefined;
  });

  createEffect(
    () => props.chatID,
    (chatID) => refreshShownMemory(chatID),
  );
  onSettled(() =>
    onStoreEvent((event) => {
      // A turn ended (or started): what the bot remembers may have moved.
      if (event.kind === "respondingChanged" && event.chatID === props.chatID && !store.isResponding(event.chatID)) refreshShownMemory(event.chatID);
    }),
  );

  return (
    <div class="inspector">
      <div class="inspector-column">
        <Show when={chat()}>
          {(current) => (
            <>
              <Participants chat={current()} members={members()} />
              <Show when={!isDM(current())}>
                <div class="inspector-add">
                  <Button disabled={!(canAddBot(current()) && members().length < store.bots.length)} onClick={() => addBotToChat(current().id)}>
                    {L("Add Bot…")}
                  </Button>
                </div>
                <Group chat={current()} members={members()} />
              </Show>
              <Show when={single()}>
                {(bot) => (
                  <>
                    <Profile bot={bot()} />
                    <Runtime bot={bot()} chat={current()} />
                    <Memory bot={bot()} />
                    <Routines bot={bot()} />
                    <Plugins bot={bot()} />
                  </>
                )}
              </Show>
              <Routing members={members()} />
            </>
          )}
        </Show>
      </div>
    </div>
  );
}

function Participants(props: { chat: Chat; members: Bot[] }) {
  return (
    <Section title={L("Bots in this chat")} class="participants">
      <For each={props.members} keyed={(bot) => bot.id}>
        {(bot) => {
          const host = () => {
            track.roster();
            return store.device(bot().runnerID)?.name ?? L("unassigned");
          };
          const isOwner = () => chatOwner(props.chat) === bot().id;
          // In a group of several, a click or a right-click on a member offers to make it the owner.
          const hasMenu = () => isGroup(props.chat) && props.members.length > 1;
          const menu = async (event: MouseEvent) => {
            const chatID = props.chat.id;
            const botID = bot().id;
            const picked = await popupMenu(
              [{ id: "owner", label: L("Make Owner"), checked: isOwner() }, ...(canRemoveBot(props.chat) ? [separator, { id: "remove", label: L("Remove from Chat") }] : [])],
              { x: event.clientX, y: event.clientY },
            );
            if (picked === "owner") store.setOwner(botID, chatID);
            else if (picked === "remove") store.removeBot(botID, chatID);
          };
          return (
            <BotRow
              bot={bot()}
              detail={`${isOwner() ? `${L("Owner")} · ` : ""}${providerName(bot().provider, store.providers)} · ${host()}`}
              accessorySymbol={canRemoveBot(props.chat) ? "minus.circle" : undefined}
              accessoryTooltip={L("Remove from chat")}
              onAccessory={() => store.removeBot(bot().id, props.chat.id)}
              onClick={hasMenu() ? menu : undefined}
              onContextMenu={hasMenu() ? menu : undefined}
              // The avatar is the way to a bot's look: symbol, color, or an image.
              onAvatarClick={() => presentBotLook(bot().id)}
            />
          );
        }}
      </For>
    </Section>
  );
}

/** A group's name and what it is for. Without a name of its own, a group goes by its members' names. */
function Group(props: { chat: Chat; members: Bot[] }) {
  const commit = (value: string) => {
    const chat = store.chat(props.chat.id);
    if (chat && value !== (chat.customTitle ?? "")) store.rename(chat.id, value);
  };
  return (
    <Section title={L("Group")}>
      <EditableRow
        label={L("Name")}
        value={props.chat.customTitle ?? ""}
        placeholder={props.members.map((bot) => bot.name).join(", ")}
        alignRight
        onCommit={commit}
      />
      <SummaryActionRow label={L("Description")} value={props.chat.groupDescription} actionTitle={L("Edit…")} onAction={() => presentGroupDescription(props.chat.id)} />
    </Section>
  );
}

function Profile(props: { bot: Bot }) {
  /** Saves the Name row when it finishes editing. An emptied value keeps the old name; Description
   * has its own sheet. */
  const commit = (value: string) => {
    const bot = store.bot(props.bot.id);
    if (!bot) return;
    const name = value === "" ? bot.name : value;
    if (name !== bot.name) store.updateBotProfile(bot.id, name);
  };
  return (
    <Section title={L("Profile")}>
      <EditableRow label={L("Name")} value={props.bot.name} placeholder={L("Name")} alignRight onCommit={commit} />
      <SummaryActionRow label={L("Description")} value={props.bot.description} actionTitle={L("Edit…")} onAction={() => presentBotDescription(props.bot.id)} />
    </Section>
  );
}

function Runtime(props: { bot: Bot; chat: Chat }) {
  const bot = () => props.bot;
  const providers = () => {
    track.roster();
    return store.providers;
  };
  /** The built-in providers, then the custom ones. A custom provider the account deleted stays
   * listed, under its slug, while the bot is still on it. */
  const kinds = () => {
    track.roster();
    const kinds = store.providerKinds;
    return kinds.includes(bot().provider) ? kinds : [...kinds, bot().provider];
  };
  // The CLI's catalog, which comes with each snapshot, and the custom providers' saved models.
  const catalog = () => {
    track.roster();
    return withCustomModels(store.models, store.providers);
  };
  const models = () => providerModels(catalog(), bot().provider);
  const levels = () => thinkingLevels(catalog(), bot().provider, bot().model);
  const credential = () => {
    track.roster();
    return store.credential(bot().provider);
  };
  const connected = () => credential()?.isConnected ?? false;
  return (
    <Section title={L("Runs with")}>
      <PopUpRow
        label={L("Provider")}
        options={kinds().map((kind) => ({ value: kind, label: providerName(kind, providers()) }))}
        value={bot().provider}
        // A new provider starts on its default model and thinking level.
        onChange={(kind) => store.setBotRuntime(bot().id, kind, undefined, undefined)}
      />
      <PopUpRow
        label={L("Model")}
        options={[{ value: "", label: L("Default (%@)", models()[0]?.label ?? "") }, ...models().map((model) => ({ value: model.id, label: model.label }))]}
        value={models().some((model) => model.id === bot().model) ? bot().model! : ""}
        onChange={(id) => {
          const model = id === "" ? undefined : id;
          if (model === bot().model) return;
          // A level the new model does not take goes back to the default.
          const kept = thinkingLevels(catalog(), bot().provider, model).some((level) => level.id === bot().thinking);
          store.setBotRuntime(bot().id, bot().provider, model, kept ? bot().thinking : undefined);
        }}
      />
      {/* Only the levels this model takes; a model without any has no choice to make. */}
      <Show when={levels().length > 0}>
        <PopUpRow
          label={L("Thinking")}
          options={[{ value: "", label: L("Default") }, ...levels().map((level) => ({ value: level.id, label: level.label }))]}
          value={levels().some((level) => level.id === bot().thinking) ? bot().thinking! : ""}
          onChange={(id) => {
            const thinking = id === "" ? undefined : id;
            if (thinking !== bot().thinking) store.setBotRuntime(bot().id, bot().provider, bot().model, thinking);
          }}
        />
      </Show>
      {/* Connected: the masked key and a Change link. Not connected: just the Connect link. A custom
          provider's link opens its own sheet: to edit it, or to add it again once it is deleted. */}
      <ActionRow
        label={L("Credential")}
        value={connected() ? (credential()?.detail ?? L("Connected")) : ""}
        tint={connected() ? "var(--label)" : "var(--label-2)"}
        actionTitle={connected() ? L("Change") : L("Connect")}
        onAction={() => void presentConnectProvider(bot().provider)}
      />
      {/* What the turns here have used, and a way to shorten the context by hand. */}
      <Show when={props.chat.usage}>
        {(usage) => (
          <>
            <ActionRow label={L("Context")} value={contextSummary(usage())} tint="var(--label)" actionTitle={L("Compact")} onAction={() => store.compactChat(props.chat.id)} />
            <KeyValueRow label={L("Spent")} value={spendSummary(usage())} />
          </>
        )}
      </Show>
    </Section>
  );
}

/** What the bot remembers, as its Runner reports it: the index against its load budget with an
 * editor, and the folder of topic files and daily logs. A bot on another Runner is read and edited
 * through the relay; only the folder cannot be opened from here. */
function Memory(props: { bot: Bot }) {
  const memory = () => memoryByBot.read()[props.bot.id];
  const error = () => memoryErrors.read()[props.bot.id];
  return (
    <Section title={L("Memory")}>
      <Show
        when={memory()}
        fallback={
          <Show
            when={error()}
            fallback={<KeyValueRow label={L("Notes")} value={memoryFetches.read()[props.bot.id] ? L("Loading…") : ""} tint="var(--label-2)" />}
          >
            {(text) => <ActionRow label={L("Notes")} value={text()} tint="var(--label-2)" actionTitle={L("Retry")} onAction={() => refreshMemory(props.bot.id)} />}
          </Show>
        }
      >
        {(known) => (
          <>
            <ActionRow
              label={L("Notes")}
              value={memoryBudgetSummary(known())}
              tint={known().truncated ? "var(--orange)" : "var(--label)"}
              actionTitle={L("Edit…")}
              tooltip={
                known().truncated
                  ? L("Only the first %d lines or %@ open each turn; the rest is not read.", known().maxLines, Format.kilobytes(known().maxBytes))
                  : L("MEMORY.md opens at the start of every turn.")
              }
              onAction={() => {
                // The rows outlive a rename, so the sheet takes the bot as it is now.
                const bot = store.bot(props.bot.id);
                if (bot) presentMemory(bot, known(), () => refreshMemory(bot.id));
              }}
            />
            <ActionRow
              label={L("Folder")}
              value={known().here ? memoryFilesSummary(known()) : L("%@ · on %@", memoryFilesSummary(known()), known().runner)}
              tint="var(--label-2)"
              actionTitle={known().here ? L("Show") : undefined}
              tooltip={known().path}
              onAction={() => void files.showInFolder(joinPath(known().path, "MEMORY.md"))}
            />
          </>
        )}
      </Show>
    </Section>
  );
}

/** The bot's routines: a row per routine with a pause switch, and the details in a sheet. With
 * none, the sentence that says how to get one. */
function Routines(props: { bot: Bot }) {
  const routines = () => {
    // A run shows as it starts, with the turns in the chats.
    track.roster();
    track.chats();
    return store.routinesFor(props.bot.id);
  };
  return (
    <Section title={L("Routines")}>
      <Show when={routines().length > 0} fallback={<NoteRow text={L("Routines are recurring tasks this bot runs on a schedule. Ask it in chat to set one up.")} />}>
        <For each={routines()} keyed={(routine) => routine.id}>
          {(routine) => (
            <SwitchRow
              symbol={routine().isRunning ? "arrow.triangle.2.circlepath" : routine().isEnabled ? "clock" : "pause.circle"}
              tint={routine().isRunning ? "var(--accent)" : routine().isEnabled ? "var(--label-2)" : "var(--label-3)"}
              title={routine().name}
              detail={routineDetail(routine())}
              isOn={routine().isEnabled}
              toggleTooltip={routine().isEnabled ? L("Pause %@", routine().name) : L("Resume %@", routine().name)}
              tooltip={routine().prompt}
              onToggle={(enabled) => store.setRoutineEnabled(routine().id, enabled)}
              onClick={() => {
                const bot = store.bot(props.bot.id);
                if (bot) presentRoutine(routine().id, bot, (text) => chatActions.prefill?.(text));
              }}
            />
          )}
        </For>
      </Show>
    </Section>
  );
}

/** The plugins the bot's Runner has, which every bot there may use, and a way to the marketplace.
 * A plugin that needs setup says so; clicking opens it. */
function Plugins(props: { bot: Bot }) {
  const runner = () => {
    track.roster();
    return store.device(props.bot.runnerID);
  };
  return (
    <Section title={L("Plugins")}>
      <For each={runner()?.plugins ?? []} keyed={(plugin) => plugin.id}>
        {(plugin) => (
          <PluginRow
            plugin={plugin()}
            tooltip={L("Open %@", plugin().name)}
            onClick={() => {
              const host = runner();
              if (host) presentPlugin(plugin().id, host);
            }}
          />
        )}
      </For>
      <Show when={(runner()?.plugins.length ?? 0) === 0}>
        <NoteRow text={L("No plugins on %@ yet. Add one from the marketplace, or ask %@ to find one.", runner()?.name ?? L("its Runner"), props.bot.name)} />
      </Show>
      <ActionRow label={L("Marketplace")} value="" tint="var(--label-2)" actionTitle={L("Add from Plugins…")} onAction={() => presentMarketplace(props.bot.runnerID)} />
    </Section>
  );
}

function Routing(props: { members: Bot[] }) {
  const runners = () => {
    track.roster();
    const ids = [...new Set(props.members.map((bot) => bot.runnerID))].sort();
    return ids.flatMap((id) => {
      const device = store.device(id);
      return device ? [device] : [];
    });
  };
  return (
    <Section title={L("Where turns run")}>
      <For each={runners()} keyed={(runner) => runner.id}>
        {(runner) => {
          const online = () => runner().status === "online";
          return (
            <StatusRow
              symbol={deviceSymbol(runner())}
              title={runner().name}
              subtitle={props.members
                .filter((bot) => bot.runnerID === runner().id)
                .map((bot) => bot.name)
                .join(", ")}
              state={online() ? L("Online") : Format.lastSeen(runner().lastSeen)}
              stateColor={online() ? "var(--green)" : "var(--label-2)"}
              onClick={() => openDevice(runner().id)}
            />
          );
        }}
      </For>
    </Section>
  );
}
