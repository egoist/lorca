// A chat with no messages yet, after the macOS app's ChatEmptyStateView: its bots, its title, where
// it runs, and two prompts to start with.

import { For } from "solid-js";
import { L } from "../../l10n";
import { isDM, providerName, type Bot, type Chat } from "../../model/models";
import { store } from "../../model/store";
import { AvatarStack } from "../avatar";

function prompts(bots: Bot[]): string[] {
  const first = bots[0];
  if (!first) return [];
  if (bots.length > 1) return [L("@%@ break this down and hand off what you can", first.name), L("@everyone what would you check first?")];
  switch (first.id) {
    case "bot-patch":
      return [L("Write the smallest version that works"), L("What would you delete first?")];
    case "bot-scout":
      return [L("Find the prior art and cite it"), L("What do we not know yet?")];
    case "bot-quill":
      return [L("Rewrite this without adjectives"), L("One paragraph, no hype")];
    case "bot-ember":
      return [L("What is the blast radius?"), L("Status on the last deploy")];
    default:
      return [L("What should I work on next?"), L("Plan this and delegate the parts")];
  }
}

export function ChatEmptyState(props: { chat: Chat; bots: Bot[]; onPick: (prompt: string) => void }) {
  const subtitle = () => {
    const only = props.bots[0];
    if (isDM(props.chat) && only) {
      const host = store.device(only.runnerID);
      return L("Runs on %@ with %@", host?.name ?? L("an unassigned Runner"), providerName(only.provider, store.providers));
    }
    if (props.bots.length > 1) {
      return `${props.bots.map((bot) => bot.name).join(L(", "))}\n${L("Address one with @, or say @everyone to hear from all of them.")}`;
    }
    if (props.bots.length > 0) return L("A group of one for now. Add bots from the inspector.");
    return L("No bots in this group yet.");
  };
  return (
    <div class="chat-empty">
      <div class="chat-empty-column">
        <AvatarStack bots={props.bots} size={52} overlap={16} />
        <div class="chat-empty-title">{store.title(props.chat)}</div>
        <div class="chat-empty-subtitle">{subtitle()}</div>
        <div class="chat-empty-suggestions">
          <For each={prompts(props.bots)}>
            {(prompt) => (
              <button class="suggestion-chip" onClick={() => props.onPick(prompt)}>
                {prompt}
              </button>
            )}
          </For>
        </div>
      </div>
    </div>
  );
}
