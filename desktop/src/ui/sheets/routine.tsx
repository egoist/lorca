// One routine's details, after the macOS app's RoutineViewController: the schedule and its next and
// last runs, the task, the check when it has one, and the actions on it. Run Now starts it on the
// bot's Runner; Pause and Resume flip the switch the inspector shows; Edit in Chat hands the bot the
// change to make, since the bot owns its routines; Delete asks first.

import { createEffect, createMemo, Show } from "solid-js";
import { L } from "../../l10n";
import * as Format from "../../model/format";
import { lastRunSummary, type Bot } from "../../model/models";
import { track } from "../../model/reactive";
import { store } from "../../model/store";
import { Button } from "../controls";
import { alert, presentSheet, Sheet } from "../overlay";
import { KeyValueRow, Section } from "../sections";

/** `onEditInChat` puts text in the chat's composer, for the bot to change the routine. */
export function presentRoutine(routineID: string, bot: Bot, onEditInChat: (text: string) => void): void {
  presentSheet((dismiss) => <RoutineSheet routineID={routineID} bot={bot} onEditInChat={onEditInChat} dismiss={dismiss} />);
}

function RoutineSheet(props: { routineID: string; bot: Bot; onEditInChat: (text: string) => void; dismiss: () => void }) {
  const title = store.routine(props.routineID)?.name ?? L("Routine");
  const routine = createMemo(() => {
    track.roster();
    track.chats();
    return store.routinesFor(props.bot.id).find((candidate) => candidate.id === props.routineID);
  });
  // The sheet closes when the routine is gone.
  createEffect(
    () => routine() === undefined,
    (gone) => {
      if (gone) props.dismiss();
    },
  );

  const state = (): [string, string] => {
    const current = routine();
    if (!current) return ["", "var(--label-2)"];
    if (current.isRunning) return [L("Running…"), "var(--accent)"];
    if (current.isEnabled) return [L("On"), "var(--green)"];
    return [current.pausedReason === "away" ? L("Paused while you were away") : L("Paused"), "var(--label-2)"];
  };
  const runTooltip = () => {
    track.roster();
    const runner = store.device(props.bot.runnerID);
    return runner ? L("Runs on %@ now", runner.name) : L("Runs on the bot's Runner now");
  };

  const confirmDelete = async () => {
    const current = store.routine(props.routineID);
    if (!current) return;
    const answer = await alert({
      message: L("Delete “%@”?", current.name),
      informative: L("This deletes the routine and stops its future runs. This can't be undone."),
      style: "warning",
      buttons: [{ title: L("Delete routine") }, { title: L("Cancel") }],
    });
    if (answer !== 0) return;
    store.deleteRoutine(props.routineID);
    props.dismiss();
  };

  return (
    <Sheet
      title={title}
      subtitle={L("A task %@ runs on its own in this chat. %@ set it up and can change it: ask in chat.", props.bot.name, props.bot.name)}
      width={520}
      confirm={L("Done")}
      cancel={null}
      onConfirm={props.dismiss}
      onCancel={props.dismiss}
    >
      <Show when={routine()}>
        {(current) => (
          <>
            <Section title={L("Schedule")}>
              <KeyValueRow label={L("State")} value={state()[0]} tint={state()[1]} />
              <KeyValueRow label={L("Schedule")} value={current().scheduleText} tooltip={current().schedule} />
              <KeyValueRow
                label={current().check === undefined ? L("Next run") : L("Next check")}
                value={current().nextRunAt !== undefined ? Format.upcoming(current().nextRunAt!) : "—"}
              />
              <KeyValueRow label={L("Last run")} value={lastRunSummary(current())} />
            </Section>
            <Section title={L("Task")}>
              <div class="routine-prompt selectable">{current().prompt}</div>
            </Section>
            <Show when={current().check}>
              {(check) => (
                <Section title={L("Check")}>
                  <div class="routine-check selectable">{check()}</div>
                </Section>
              )}
            </Show>
            <div class="sheet-actions">
              <Button disabled={current().isRunning} tooltip={runTooltip()} onClick={() => store.runRoutine(props.routineID)}>
                {L("Run Now")}
              </Button>
              <Button onClick={() => store.setRoutineEnabled(props.routineID, !current().isEnabled)}>{current().isEnabled ? L("Pause") : L("Resume")}</Button>
              <Button
                onClick={() => {
                  props.onEditInChat(L('Edit my routine "%@": ', current().name));
                  props.dismiss();
                }}
              >
                {L("Edit in Chat…")}
              </Button>
              <span class="sheet-spacer" />
              <Button onClick={() => void confirmDelete()}>{L("Delete…")}</Button>
            </div>
          </>
        )}
      </Show>
    </Sheet>
  );
}
