// The Devices tab's stack: the account's Devices, and a Device's screen and the plugin accounts
// and coding agents it slides in, the same screens the settings sheet has beside the sidebar.

import { Stack } from "expo-router";
import { Platform } from "react-native";
import { t, useLanguage } from "../../../../src/i18n";
import { usePalette } from "../../../../src/ui/theme";

export default function DevicesLayout() {
  useLanguage();
  const p = usePalette();
  return (
    <Stack
      screenOptions={{
        headerShadowVisible: false,
        headerTintColor: (Platform.OS === "android" ? p.label : p.tint) as any,
        headerTitleStyle: { color: p.label as any },
        headerTitleAlign: Platform.OS === "android" ? "left" : undefined,
        headerBackButtonDisplayMode: "minimal",
        // The bar's material over the grouped background, as a large-title list's bar is.
        headerTransparent: Platform.OS === "ios",
        headerStyle: Platform.OS === "android" ? { backgroundColor: p.groupedBackground } : undefined,
        contentStyle: { backgroundColor: p.groupedBackground },
      }}
    >
      <Stack.Screen name="index" options={{ title: t("Devices"), headerLargeTitle: Platform.OS === "ios" }} />
      <Stack.Screen name="device/[id]" />
      <Stack.Screen name="account/[id]" />
      <Stack.Screen name="coding-agents" />
    </Stack>
  );
}
