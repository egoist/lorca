// Edits a bot's MEMORY.md, the notes that open every turn, after the macOS app's
// MemoryViewController. A save carries the hash of the text that was opened, so it is refused when
// the bot wrote in between; the user then reloads the bot's version or overwrites it knowingly.

import { createSignal } from "solid-js";
import { L } from "../../l10n";
import * as Format from "../../model/format";
import type { Bot, BotMemory } from "../../model/models";
import { errorText, store } from "../../model/store";
import { TextArea } from "../controls";
import { alert, presentSheet, Sheet } from "../overlay";

export function presentMemory(bot: Bot, memory: BotMemory, onSaved: () => void): void {
  presentSheet((dismiss) => <MemorySheet bot={bot} memory={memory} onSaved={onSaved} dismiss={dismiss} />);
}

/** Lines and bytes against the load budget; amber from 80%, red once anything stops loading. */
function gauge(text: string, memory: BotMemory): { text: string; color: string } {
  const lines = text === "" ? 0 : text.split("\n").length;
  const bytes = new TextEncoder().encode(text).length;
  const overLines = lines > memory.maxLines;
  const overBytes = bytes > memory.maxBytes;
  let status =
    lines === 1
      ? L("%d line of %d · %@ of %@", lines, memory.maxLines, Format.kilobytes(bytes), Format.kilobytes(memory.maxBytes))
      : L("%d lines of %d · %@ of %@", lines, memory.maxLines, Format.kilobytes(bytes), Format.kilobytes(memory.maxBytes));
  if (overLines || overBytes) {
    const hidden = overLines ? lines - memory.maxLines : 0;
    status +=
      " · " +
      (hidden > 0
        ? hidden === 1
          ? L("%d line past the budget will not load", hidden)
          : L("%d lines past the budget will not load", hidden)
        : L("the end will not load"));
    return { text: status, color: "var(--red)" };
  }
  if (lines >= memory.maxLines * 0.8 || bytes >= memory.maxBytes * 0.8) return { text: status, color: "var(--orange)" };
  return { text: status, color: "var(--label-2)" };
}

function MemorySheet(props: { bot: Bot; memory: BotMemory; onSaved: () => void; dismiss: () => void }) {
  let memory = props.memory;
  let editor: HTMLTextAreaElement | undefined;
  const [text, setText] = createSignal(memory.text);
  const [saving, setSaving] = createSignal(false);

  const save = async (expectedHash: string | null) => {
    setSaving(true);
    try {
      await store.writeBotMemory(props.bot.id, editor?.value ?? text(), expectedHash);
      props.onSaved();
      props.dismiss();
    } catch (error) {
      setSaving(false);
      if (errorText(error).includes("changed since")) void resolveConflict();
      else void alert({ message: L("Couldn't save the memory"), informative: errorText(error) });
    }
  };

  /** The bot wrote while the user was editing: show the bot's version, or write over it. */
  const resolveConflict = async () => {
    const answer = await alert({
      message: L("%@ changed this file while you were editing", props.bot.name),
      informative: L("Reload shows %@'s version and discards your draft. Overwrite saves yours over it.", props.bot.name),
      buttons: [{ title: L("Reload") }, { title: L("Overwrite with mine") }, { title: L("Cancel") }],
    });
    if (answer === 0) {
      try {
        const fresh = await store.botMemory(props.bot.id);
        memory = fresh;
        if (editor) editor.value = fresh.text;
        setText(fresh.text);
      } catch {
        // The draft stays; the next save tries again.
      }
    } else if (answer === 1) {
      void save(null);
    }
  };

  const status = () => gauge(text(), memory);
  return (
    <Sheet
      title={L("%@'s memory", props.bot.name)}
      subtitle={L(
        "MEMORY.md opens at the start of every turn: the first %d lines or %@, whichever cuts first. Longer notes belong in memory/<topic>.md files the bot reads on demand.",
        props.memory.maxLines,
        Format.kilobytes(props.memory.maxBytes),
      )}
      width={560}
      confirm={L("Save")}
      confirmDisabled={saving()}
      onConfirm={() => void save(memory.hash)}
      onCancel={props.dismiss}
    >
      <TextArea
        ref={(element) => (editor = element)}
        value={props.memory.text}
        monospaced
        autofocus
        class="sheet-editor memory-editor"
        label={L("%@'s memory", props.bot.name)}
        onInput={setText}
      />
      <div class="memory-gauge" style={{ color: status().color }}>
        {status().text}
      </div>
    </Sheet>
  );
}
