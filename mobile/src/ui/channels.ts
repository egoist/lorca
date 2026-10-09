// Channels in the phone's words: what a channel takes, where it listens, and its state, as the
// desktop apps' channel rows and sheet say them.

import type { ChannelListen, ChannelStatus } from "../core/model";
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

/// A state the user has something to do about, in a word or two; none while it listens or rests.
export function channelProblem(channel: ChannelStatus): string | undefined {
  if (channel.state === "held") return t("On hold");
  if (channel.state === "offline") return t("Can’t connect");
  return undefined;
}

/// Where it listens: the chats it names, or every chat the account's bot is in.
export function channelChats(channel: ChannelStatus): string {
  if (!channel.chats?.length) return t("Every chat the bot is in");
  return channel.chats.map((chat) => chat.title || chat.id).join(tc(", ", "list"));
}
