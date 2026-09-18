// The settings sheet's own stack: Settings first, and a Device sliding in from its row, inside
// the one form sheet the root presents.

import { Stack } from "expo-router";
import { usePalette } from "../../src/ui/theme";

export default function SettingsLayout() {
  const p = usePalette();
  return (
    <Stack
      screenOptions={{
        headerShadowVisible: false,
        // The native bar takes a plain color, not a dynamic system one.
        headerStyle: { backgroundColor: p.dark ? "#1C1C1E" : "#F2F2F7" },
        headerTintColor: p.tint as any,
        headerTitleStyle: { color: p.label as any },
        headerBackButtonDisplayMode: "minimal",
        contentStyle: { backgroundColor: p.groupedBackground },
      }}
    >
      <Stack.Screen name="index" />
      <Stack.Screen name="device/[id]" />
    </Stack>
  );
}
