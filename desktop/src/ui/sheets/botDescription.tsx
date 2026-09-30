// The bot's full behavioral description, after the macOS app's BotDescriptionViewController, opened
// from the compact Profile row in the inspector. Saving publishes the changed profile, so the bot's
// next turn and every paired Device see it.

import { L } from "../../l10n";
import { store } from "../../model/store";
import { TextArea } from "../controls";
import { presentSheet, Sheet } from "../overlay";

export function presentBotDescription(botID: string): void {
  presentSheet((dismiss) => {
    let text = store.bot(botID)?.description ?? "";
    const save = () => {
      const bot = store.bot(botID);
      if (bot) {
        const description = text.trim();
        if (description !== bot.description) store.updateBotProfile(bot.id, bot.name, description);
      }
      dismiss();
    };
    return (
      <Sheet title={L("Description")} subtitle={L("What it does and how it should work")} width={520} confirm={L("Save")} onConfirm={save} onCancel={dismiss}>
        <TextArea value={text} autofocus class="sheet-editor description-editor" label={L("Description")} onInput={(value) => (text = value)} />
      </Sheet>
    );
  });
}
