// The chat info sheet's own stack: Details first, and the bot's Look sliding in from its avatar,
// inside the one form sheet the root presents.

import { Stack } from "expo-router";
import { usePalette } from "../../src/ui/theme";

export default function ChatInfoLayout() {
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
      <Stack.Screen name="[id]" />
      <Stack.Screen name="look/[id]" />
    </Stack>
  );
}
