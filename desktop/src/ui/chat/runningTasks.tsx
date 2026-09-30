// A chat's running tasks, after the macOS app's RunningTasksViewController: the commands its bots
// have running in their terminals, each with what it does in the bot's words, who runs it and for
// how long, the command (a click shows all of it), its last lines, and Stop. One that ends while
// the list is open stays, saying how it ended, until the list closes.

import { createEffect, createMemo, createSignal, For, onSettled, Show } from "solid-js";
import { L } from "../../l10n";
import * as Format from "../../model/format";
import { firstLine, hasEnded, isGroup, type CommandState } from "../../model/models";
import { track } from "../../model/reactive";
import { errorText, store } from "../../model/store";
import { Button } from "../controls";
import { Icon } from "../icons";
import { CommandBlock, CommandOutput, presentCommandSheet } from "./cards";

interface RunningTask {
  id: string;
  title: string;
  botName: string;
  showsBotName: boolean;
  command: string;
  firstLine: string;
  output: string;
  state: CommandState;
  startedAt: number;
}

const isLive = (task: RunningTask) => task.state === "running" || task.state === "waiting";

/** "Scout · Running · 1:05", or how it ended: "Finished". */
function status(task: RunningTask, now: number): string {
  const parts = task.showsBotName ? [task.botName] : [];
  switch (task.state) {
    case "running":
      parts.push(L("Running"), Format.elapsed(task.startedAt, now));
      break;
    case "waiting":
      parts.push(L("Waiting for input"), Format.elapsed(task.startedAt, now));
      break;
    case "exited":
      parts.push(L("Finished"));
      break;
    case "failed":
      parts.push(L("Failed"));
      break;
    default:
      parts.push(L("Stopped"));
  }
  return parts.join(" · ");
}

export function RunningTasks(props: { chatID: string; onEmpty: () => void; onClose: () => void }) {
  /** Every command listed since the list opened, so one that ends stays. */
  const listed = new Set<string>();
  const [errors, setErrors] = createSignal<Record<string, string>>({});
  const [busy, setBusy] = createSignal<Record<string, boolean>>({});
  const [now, setNow] = createSignal(Date.now());
  let hadTasks = false;

  const tasks = createMemo((): RunningTask[] => {
    track.chat(props.chatID);
    track.roster();
    const chat = store.chat(props.chatID);
    if (!chat) return [];
    const running = new Set(store.runningCommands(props.chatID).map((message) => message.id));
    const list: RunningTask[] = [];
    for (const message of chat.messages) {
      if (message.body.kind !== "tool" || !message.body.tool.run) continue;
      const run = message.body.tool.run;
      if (!running.has(message.id) && !(listed.has(message.id) && hasEnded(run))) continue;
      listed.add(message.id);
      list.push({
        id: message.id,
        title: message.body.tool.description ?? firstLine(run),
        botName: (message.author.kind === "bot" && store.bot(message.author.botID)?.name) || L("The bot"),
        showsBotName: isGroup(chat),
        command: run.command,
        firstLine: firstLine(run),
        output: run.output ?? "",
        state: run.state,
        startedAt: run.startedAt ?? message.createdAt,
      });
    }
    return list;
  });

  onSettled(() => {
    const clock = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(clock);
  });
  // Nothing left to show: the commands' rows are gone.
  createEffect(
    () => tasks().length,
    (count) => {
      if (count > 0) hadTasks = true;
      else if (hadTasks) props.onEmpty();
    },
  );

  const stop = async (task: RunningTask) => {
    if (busy()[task.id]) return;
    setBusy({ ...busy(), [task.id]: true });
    try {
      await store.stopCommand(props.chatID, task.id);
      const next = { ...errors() };
      delete next[task.id];
      setErrors(next);
    } catch (error) {
      setErrors({ ...errors(), [task.id]: errorText(error) });
    } finally {
      setBusy({ ...busy(), [task.id]: false });
    }
  };

  return (
    <div class="running-tasks">
      <For each={tasks()} keyed={(task) => task.id}>
        {(task) => {
          return (
            <div class="running-task">
              <span class="running-task-icon">
                <Icon name="terminal" size={17} strokeWidth={1.9} />
              </span>
              <div class="running-task-body">
                <div class="running-task-title-row">
                  <span class="running-task-title truncate" title={task().title}>
                    {task().title}
                  </span>
                  <Show when={isLive(task())}>
                    <Button small disabled={!!busy()[task().id]} onClick={() => void stop(task())}>
                      {L("Stop")}
                    </Button>
                  </Show>
                </div>
                <div class="running-task-status">{status(task(), now())}</div>
                <CommandBlock
                  text={`$ ${task().firstLine}`}
                  lines={1}
                  onClick={() => {
                    props.onClose();
                    presentCommandSheet(L("%@'s command", task().botName), task().command);
                  }}
                />
                <Show when={task().output !== ""}>
                  <CommandOutput text={task().output} lines={10} />
                </Show>
                <Show when={errors()[task().id]}>{(error) => <div class="running-task-error">{error()}</div>}</Show>
              </div>
            </div>
          );
        }}
      </For>
    </div>
  );
}
