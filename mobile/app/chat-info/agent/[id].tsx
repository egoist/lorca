// A coding agent's page slid in from Details' Coding section.

import { useLocalSearchParams } from "expo-router";
import { AgentScreen } from "../../../src/ui/AgentScreen";

export default function AgentInDetails() {
  const { id, chat } = useLocalSearchParams<{ id: string; chat: string }>();
  return <AgentScreen chatId={chat} messageId={id} presented={false} />;
}
