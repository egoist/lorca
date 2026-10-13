// Channels in the phone's words: what a channel takes, where it listens, and its state, as the
// desktop apps' channel rows and sheet say them.

import type { ChannelListen, ChannelStatus, ChatRequest } from "../core/model";
import { t, tc } from "../i18n";

/// "Mentions, replies, #feedback", or "Every message".
export function listenSummary(listen: ChannelListen): string {
  if (listen.every) return t("Every message");
  const parts = [...(listen.mentions ? [t("mentions")] : []), ...(listen.replies ? [t("replies")] : []), ...(listen.tags ?? []).map((tag) => `#${tag}`)];
  const joined = parts.join(tc(", ", "list"));
  return joined.charAt(0).toUpperCase() + joined.slice(1);
}

/// The service as people call it.
export function serviceName(service: string): string {
  return service === "slack" ? "Slack" : "Telegram";
}

/// The symbol that stands for the service on a row.
export function serviceSymbol(service: string): string {
  return service === "slack" ? "number" : "paperplane";
}

/// A state the user has something to do about, in a word or two; none while it listens fully or rests.
export function channelProblem(channel: ChannelStatus): string | undefined {
  if (channel.state === "held") return t("On hold");
  if (channel.state === "offline") return t("Can’t connect");
  const waiting = channel.requests?.length ?? 0;
  if (waiting === 1) return t("A chat is waiting");
  if (waiting > 1) return t("{count} chats are waiting", { count: waiting });
  // A listening channel's only detail is the Telegram privacy-mode note.
  if (channel.state === "listening" && channel.detail) return t("Privacy mode on");
  return undefined;
}

/// Where it listens: the chats the user allowed.
export function channelChats(channel: ChannelStatus): string {
  if (!channel.chats?.length) return t("None yet");
  return channel.chats.map((chat) => chat.title || chat.id).join(tc(", ", "list"));
}

/// What kind of chat asks: a direct message, or a group and who wrote there or added the bot.
export function requestKind(request: ChatRequest): string {
  if (request.private) return t("Direct message");
  return request.from ? t("Group · {name}", { name: request.from }) : t("Group");
}
