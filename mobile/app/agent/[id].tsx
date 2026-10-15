// A coding agent's page as a sheet over the chat, opened from one of its markers.

import { useLocalSearchParams } from "expo-router";
import { AgentScreen } from "../../src/ui/AgentScreen";

export default function AgentSheet() {
  const { id, chat } = useLocalSearchParams<{ id: string; chat: string }>();
  return <AgentScreen chatId={chat} messageId={id} presented />;
}
