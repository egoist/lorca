// Share as Template from a bot's Details: see src/ui/TemplateShareScreen.tsx.

import { useLocalSearchParams } from "expo-router";
import { TemplateShareScreen } from "../../../src/ui/TemplateShareScreen";

export default function ShareBotScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <TemplateShareScreen botId={id} />;
}
