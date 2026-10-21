import { Stack } from "expo-router";
import { Platform } from "react-native";
import { t, useLanguage } from "../../../../src/i18n";
import { useStackScreenOptions } from "../../../../src/ui/navigation";

/// The Bots tab's stack: the chat list, its bar holding Settings, Attention, and New. A chat
/// opens over the tabs, in the stack around them. On iOS the list's bar is that stack's (TabsLayout).
export default function ChatsLayout() {
  useLanguage();
  const screenOptions = useStackScreenOptions();
  return (
    <Stack screenOptions={screenOptions}>
      <Stack.Screen name="index" options={{ title: t("Chats"), headerTitle: "", headerLargeTitle: false, headerShadowVisible: false, headerShown: Platform.OS !== "ios" }} />
    </Stack>
  );
}
