// Workflow feedback from the bot's Runner. It keeps all of it and applies every decision;
// `feedback.*` reaches it as a sealed request through the core, and each change reads the list
// again, since the Runner announces its changes only on its own Device.

import { create } from "zustand";
import * as core from "../../modules/lorca-core";
import { toBotFeedback, type BotFeedback, type FeedbackChange, type FeedbackKind, type FeedbackNote, type FeedbackSuggestion, type FeedbackTarget, type WireFeedback } from "../ui/feedback";

/// Each bot's feedback as its Runner last answered, for Details and the feedback screens.
export const useFeedbackStore = create<{ byBot: Record<string, BotFeedback> }>(() => ({ byBot: {} }));

export function useFeedback(botId: string | undefined): BotFeedback | undefined {
  return useFeedbackStore((s) => (botId ? s.byBot[botId] : undefined));
}

/// Asks the bot's Runner again; one that cannot be asked leaves what was shown.
export async function loadFeedback(botId: string): Promise<BotFeedback | undefined> {
  try {
    const f = toBotFeedback(await core.request<WireFeedback>("feedback.list", { bot_id: botId }));
    useFeedbackStore.setState((s) => ({ byBot: { ...s.byBot, [botId]: f } }));
    return f;
  } catch (error) {
    console.warn("loading feedback", error instanceof Error ? error.message : error);
    return undefined;
  }
}

/// A change to the bot's feedback, then the list again.
async function change(botId: string, method: string, params: Record<string, unknown>): Promise<void> {
  await core.request(method, { ...params, bot_id: botId });
  await loadFeedback(botId);
}

export const recordFeedback = (botId: string, input: { kind: FeedbackKind; chatId: string; messageId: string; note: string; before?: string; after?: string; target?: FeedbackTarget }) =>
  change(botId, "feedback.record", {
    feedback: {
      kind: input.kind,
      origin: { chat_id: input.chatId, message_id: input.messageId },
      note: input.note,
      ...(input.kind === "edited" ? { before: input.before, after: input.after } : {}),
      ...(input.target ? { target: input.target } : {}),
    },
  });

export const decideSuggestion = (botId: string, suggestion: FeedbackSuggestion, accept: boolean) =>
  change(botId, accept ? "feedback.accept" : "feedback.reject", { id: suggestion.id, diff_hash: suggestion.diffHash });

export const undoChange = (botId: string, item: FeedbackChange) => change(botId, "feedback.rollback", { id: item.id, expected_hash: item.currentHash ?? "" });

export const setReviewEvery = (botId: string, seconds: number | undefined) => change(botId, "feedback.settings", { review_every_secs: seconds ?? null });

export const excludeNote = (botId: string, note: FeedbackNote, wholeChat: boolean) => change(botId, "feedback.exclude", wholeChat ? { chat_id: note.chatId } : { id: note.id });

/// Has the bot look through its new feedback now; answers how many changes it suggests.
export async function suggestChanges(botId: string): Promise<number> {
  const review = await core.request<{ proposals: unknown[] }>("feedback.review", { bot_id: botId });
  await loadFeedback(botId);
  return review.proposals.length;
}
