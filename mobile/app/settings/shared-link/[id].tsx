// A shared link's bot from Settings › Shared Links: see src/ui/TemplateShareScreen.tsx.

import { useLocalSearchParams } from "expo-router";
import { TemplateShareScreen } from "../../../src/ui/TemplateShareScreen";

export default function SharedLinkScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  return <TemplateShareScreen botId={id} />;
}
