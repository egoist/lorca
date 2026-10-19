import { Stack } from "expo-router";
import { Platform } from "react-native";
import { t, useLanguage } from "../../../../src/i18n";
import { useStackScreenOptions } from "../../../../src/ui/navigation";

/// The Search tab's stack. Android's screen draws its own field under the status bar.
export default function SearchLayout() {
  useLanguage();
  const screenOptions = useStackScreenOptions();
  return (
    <Stack screenOptions={screenOptions}>
      <Stack.Screen name="index" options={Platform.OS === "android" ? { headerShown: false } : { title: t("Search"), headerLargeTitle: true, headerTransparent: true, headerShadowVisible: false }} />
    </Stack>
  );
}
