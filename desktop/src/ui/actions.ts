// The window's sheets, after the macOS app's RootSplitViewController actions: a new bot, a new group
// chat, a bot added to a chat, the marketplace, and pairing. Each lands where the Mac app does: a new
// bot in its direct chat, a new group in itself.

import { host } from "../host";
import { L } from "../l10n";
import { store } from "../model/store";
import { presentMarketplace } from "./marketplace";
import { botsAvailableToAdd, open, selectedChatID } from "./root";
import { presentBotPicker, presentNewGroupChat } from "./sheets/botPicker";
import { presentNewBot } from "./sheets/newBot";
import { presentPairing } from "./sheets/pairing";

export { presentMarketplace, presentPairing };

/** Every bot has a direct chat, so creating one lands in that chat right away. */
export function newBot(): void {
  presentNewBot((botID) => open(store.dm(botID)));
}

export function newGroupChat(): void {
  presentNewGroupChat((botIDs, title) => open(store.createChat("group", botIDs, title)));
}

/** A bot for the chat (the selected one unless named), from those that can still join it. */
export function addBotToChat(chatID: string | null = selectedChatID()): void {
  const chat = chatID ? store.chat(chatID) : undefined;
  const available = chat ? botsAvailableToAdd(chat.id) : [];
  if (!chat || available.length === 0) {
    void host.beep();
    return;
  }
  presentBotPicker(L("Add a bot to %@", store.title(chat)), available, (botID) => store.addBot(botID, chat.id));
}
