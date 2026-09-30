// Cards in the transcript, after the macOS app's PermissionCellView and CommandCellView: a bot
// asking before a plugin action, an install, a sign-in, or a shell command runs, and a command the
// bot handed over, with its last lines, an answer field, and Stop. In a group a card sits in the
// bubbles' column with the bot's avatar beside its bottom edge.

import { createEffect, createSignal, onSettled, Show } from "solid-js";
import type { JSX } from "@solidjs/web";
import { host } from "../../host";
import { L } from "../../l10n";
import {
  asksYesOrNo,
  commandChoices,
  decisionText,
  firstLine,
  fullCommand,
  isConnect,
  isInstall,
  isLive,
  isPending,
  isShell,
  permissionChoices,
  takesInput,
  verbPhrase,
  type CommandRun,
  type PermissionRequest,
} from "../../model/models";
import { errorText } from "../../model/store";
import { Avatar, type AvatarContent } from "../avatar";
import { Button, CopyButton, TextField } from "../controls";
import { Icon } from "../icons";
import { alert, presentSheet, Sheet } from "../overlay";
import { ChatMetrics } from "./cells";

/** The whole command a card asks about, to read or copy before answering. */
export function presentCommandSheet(title: string, command: string): void {
  presentSheet((dismiss) => (
    <Sheet
      title={title}
      width={600}
      confirm={L("Done")}
      cancel={null}
      onConfirm={dismiss}
      onCancel={dismiss}
      leading={<CopyButton text={() => command} title={L("Copy")} bordered />}
    >
      <pre class="command-sheet-text selectable">{command}</pre>
    </Sheet>
  ));
}

/** A shell command's first lines in a code block that shows the whole command on click,
 * highlighting under the pointer like a button. */
export function CommandBlock(props: { text: string; lines: number; onClick: () => void }) {
  return (
    <button class="command-block" style={{ "--lines": String(props.lines) }} title={L("Show the full command")} aria-label={L("Show the full command")} onClick={() => props.onClick()}>
      <span class="command-block-text">{props.text}</span>
    </button>
  );
}

/** A command's last lines: a code block that grows to `lines` lines and then scrolls, the newest line
 * in view, fading at an edge with more lines past it. */
export function CommandOutput(props: { text: string; lines: number }) {
  let scroller: HTMLDivElement | undefined;
  const [fade, setFade] = createSignal({ above: false, below: false });
  const measure = () => {
    if (!scroller) return;
    setFade({ above: scroller.scrollTop > 1, below: scroller.scrollTop + scroller.clientHeight < scroller.scrollHeight - 1 });
  };
  createEffect(
    () => props.text,
    () => {
      if (!scroller) return;
      scroller.scrollTop = scroller.scrollHeight;
      measure();
    },
  );
  onSettled(() => {
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
    measure();
  });
  return (
    <div
      ref={(el) => (scroller = el)}
      class={["command-output", "selectable", { "fade-top": fade().above, "fade-bottom": fade().below }]}
      style={{ "--lines": String(props.lines) }}
      onScroll={measure}
    >
      {props.text}
    </div>
  );
}

// MARK: - Permission card

/** The line under the title: the call while it waits, the answer and the call once answered, or
 * where to enter a sign-in code. */
function permissionSummary(request: PermissionRequest): string {
  if (isPending(request)) return request.summary;
  if (request.decision === "allowed" && request.code !== undefined) {
    return L("Enter this code at %@, then come back.", hostOf(request.link) ?? L("the link"));
  }
  return `${decisionText(request)} · ${request.summary}`;
}

function hostOf(link: string | undefined): string | undefined {
  if (!link) return undefined;
  try {
    return new URL(link).host;
  } catch {
    return undefined;
  }
}

/** What a card says about its rule: the one Always allow would add, or the one it added. */
function ruleNote(request: PermissionRequest): string | undefined {
  if (request.rule === undefined) return undefined;
  if (isPending(request)) return L("Always allow adds the rule “%@”.", request.rule);
  return request.decision === "always" ? L("Added the rule “%@” to Auto-review.", request.rule) : undefined;
}

export function permissionSpokenText(request: PermissionRequest, botName: string): string {
  return `${botName} ${verbPhrase(request)}: ${permissionSummary(request)}`;
}

function CardFrame(props: { avatar?: AvatarContent; groupStart: boolean; children: JSX.Element }) {
  const indent = () => (props.avatar ? ChatMetrics.bubbleIndent : ChatMetrics.horizontalInset);
  return (
    <div class={["card-row", { tight: !props.groupStart }]} style={{ "padding-left": `${indent()}px` }}>
      <div class="card">{props.children}</div>
      <Show when={props.avatar}>
        {(avatar) => (
          <span class="message-avatar">
            <Avatar content={avatar()} size={ChatMetrics.avatarSize} />
          </span>
        )}
      </Show>
    </div>
  );
}

export function PermissionCard(props: {
  request: PermissionRequest;
  botName: string;
  avatar?: AvatarContent;
  groupStart: boolean;
  onDecision: (decision: string) => void;
  onShowCommand: () => void;
}) {
  const request = () => props.request;
  const hasCode = () => request().decision === "allowed" && request().code !== undefined;
  const icon = () => (isConnect(request()) ? "person.crop.circle.badge.checkmark" : isInstall(request()) ? "puzzlepiece.extension" : "hand.raised");
  const tint = () => (request().decision === "failed" ? "var(--red)" : request().decision === "connected" ? "var(--green)" : "var(--accent)");
  return (
    <CardFrame avatar={props.avatar} groupStart={props.groupStart}>
      <span class="card-icon" style={{ color: tint() }}>
        <Icon name={icon()} size={17} strokeWidth={1.9} />
      </span>
      <div class="card-body" aria-label={permissionSpokenText(request(), props.botName)}>
        <div class="card-title truncate" title={`${props.botName} ${verbPhrase(request())}`}>
          {props.botName} {verbPhrase(request())}
        </div>
        <Show
          when={isPending(request()) && isShell(request())}
          fallback={
            <div class={["card-summary", isPending(request()) ? "clamp-2" : "clamp-3"]} title={request().summary}>
              {permissionSummary(request())}
            </div>
          }
        >
          <CommandBlock text={fullCommand(request())} lines={2} onClick={props.onShowCommand} />
        </Show>
        <Show when={hasCode()}>
          <div class="card-code-row">
            <span class="card-code selectable">{request().code}</span>
            <CopyButton
              bordered
              text={() => request().code ?? ""}
              title={L("Copy code and open %@", hostOf(request().link) ?? L("link"))}
              onCopied={() => {
                const link = request().link;
                if (link) void host.openExternal(link);
              }}
            />
          </div>
        </Show>
        <Show when={isPending(request()) && request().reason}>
          <div class="card-reason">{request().reason}</div>
        </Show>
        <Show when={isPending(request())}>
          <div class="card-buttons">
            {permissionChoices(request()).map(([title, decision]) => (
              <Button small onClick={() => props.onDecision(decision)}>
                {title}
              </Button>
            ))}
          </div>
        </Show>
        <Show when={ruleNote(request())}>{(note) => <div class="card-note">{note()}</div>}</Show>
      </div>
    </CardFrame>
  );
}

// MARK: - Command card

/** "Chef wants to run a command on Workbench", "Chef's command is running", or "Chef's command
 * is waiting for input". */
export function commandTitle(run: CommandRun, botName: string): string {
  switch (run.state) {
    case "waiting":
      return L("%@'s command is waiting for input", botName);
    case "running":
      return L("%@'s command is running", botName);
    default:
      return run.device ? `${botName} ${L("wants to run a command on %@", run.device)}` : L("%@'s command", botName);
  }
}

/** The terminal's last lines with something on them. */
export function outputText(run: { output?: string }): string | undefined {
  const lines = (run.output ?? "").split("\n").filter((line) => line !== "");
  return lines.length === 0 ? undefined : lines.join("\n");
}

function commandNote(run: CommandRun): string | undefined {
  if (run.state === "asking") return run.rule !== undefined ? L("Always allow adds the rule “%@”.", run.rule) : undefined;
  if (run.state === "waiting" && run.sessionID !== undefined) return L("What you type goes straight to the command, not into the chat.");
  return undefined;
}

export function commandSpokenText(run: CommandRun, botName: string): string {
  const under = isLive(run) ? outputText(run) : run.state === "asking" ? run.reason : undefined;
  return [commandTitle(run, botName), `$ ${firstLine(run)}`, under].filter(Boolean).join(": ");
}

export function CommandCard(props: {
  run: CommandRun;
  messageID: string;
  botName: string;
  avatar?: AvatarContent;
  groupStart: boolean;
  onDecision: (decision: string) => void;
  onSend: (text: string) => Promise<void>;
  onStop: () => Promise<void>;
  onShowCommand: () => void;
}) {
  const run = () => props.run;
  const [answer, setAnswer] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  const [answerError, setAnswerError] = createSignal<string | null>(null);
  // Another state or new output clears what went wrong; ending takes back what was typed.
  createEffect(
    () => [run().state, run().output] as const,
    () => {
      setAnswerError(null);
      if (!takesInput(run())) setAnswer("");
    },
    { defer: true },
  );
  const answering = () => run().state === "waiting" && run().sessionID !== undefined;
  const send = async () => {
    if (busy() || !takesInput(run())) return;
    setBusy(true);
    setAnswerError(null);
    try {
      await props.onSend(answer());
      setAnswer("");
    } catch (error) {
      setAnswerError(errorText(error));
    } finally {
      setBusy(false);
    }
  };
  const stop = async () => {
    if (busy()) return;
    setBusy(true);
    try {
      await props.onStop();
    } catch (error) {
      // Under the answer field when there is one, else in an alert.
      if (commandNote(run()) !== undefined) setAnswerError(errorText(error));
      else void alert({ message: L("Could not stop the command"), informative: errorText(error) });
    } finally {
      setBusy(false);
    }
  };
  const note = () => answerError() ?? commandNote(run());
  return (
    <CardFrame avatar={props.avatar} groupStart={props.groupStart}>
      <span class="card-icon" style={{ color: "var(--accent)" }}>
        <Icon name="terminal" size={17} strokeWidth={1.9} />
      </span>
      <div class="card-body" aria-label={commandSpokenText(run(), props.botName)}>
        <div class="card-title-row">
          <div class="card-title truncate" title={run().command}>
            {commandTitle(run(), props.botName)}
          </div>
          <Show when={takesInput(run())}>
            <Button small disabled={busy()} onClick={() => void stop()}>
              {L("Stop")}
            </Button>
          </Show>
        </div>
        <CommandBlock text={`$ ${firstLine(run())}`} lines={1} onClick={props.onShowCommand} />
        <Show when={run().state === "asking" && run().reason}>
          <div class="card-reason clamp-4">{run().reason}</div>
        </Show>
        <Show when={isLive(run()) && outputText(run())}>{(text) => <CommandOutput text={text()} lines={6} />}</Show>
        <Show when={run().state === "asking"}>
          <div class="card-buttons">
            {commandChoices(run()).map(([title, decision]) => (
              <Button small onClick={() => props.onDecision(decision)}>
                {title}
              </Button>
            ))}
          </div>
        </Show>
        <Show when={answering()}>
          <div class="card-answer">
            <TextField
              value={answer()}
              secure={!asksYesOrNo(run())}
              placeholder={L("Type your answer")}
              label={run().prompt ?? L("Type your answer")}
              disabled={busy()}
              onInput={setAnswer}
              onEnter={() => void send()}
            />
            <Button small disabled={busy()} onClick={() => void send()}>
              {L("Send")}
            </Button>
          </div>
        </Show>
        <Show when={note()}>
          {(text) => (
            <div class={["card-note", { error: answerError() !== null }]} title={answerError() ?? undefined}>
              {text()}
            </div>
          )}
        </Show>
      </div>
    </CardFrame>
  );
}
