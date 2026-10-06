// A description edited in a sheet of its own, after the macOS app's DescriptionViewController, opened
// from a compact row in the inspector: a bot's (what it does and how it should work) or a group's
// (what it is for). Saving publishes it, so the next turn and every paired Device see it.

import { L } from "../../l10n";
import { store } from "../../model/store";
import { TextArea } from "../controls";
import { presentSheet, Sheet } from "../overlay";

function presentDescription(subtitle: string, initial: string, save: (description: string) => void): void {
  presentSheet((dismiss) => {
    let text = initial;
    const confirm = () => {
      save(text.trim());
      dismiss();
    };
    return (
      <Sheet title={L("Description")} subtitle={subtitle} width={520} confirm={L("Save")} onConfirm={confirm} onCancel={dismiss}>
        <TextArea value={text} autofocus class="sheet-editor description-editor" label={L("Description")} onInput={(value) => (text = value)} />
      </Sheet>
    );
  });
}

/** The bot's full behavioral description. */
export function presentBotDescription(botID: string): void {
  presentDescription(L("What it does and how it should work"), store.bot(botID)?.description ?? "", (description) => {
    const bot = store.bot(botID);
    if (bot && description !== bot.description) store.updateBotProfile(bot.id, bot.name, description);
  });
}

/** What a group is for, which every bot in it reads. */
export function presentGroupDescription(chatID: string): void {
  presentDescription(L("What this group is for. Every bot in it reads this."), store.chat(chatID)?.groupDescription ?? "", (description) =>
    store.setDescription(description, chatID),
  );
}
