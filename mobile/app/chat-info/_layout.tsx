// The chat info sheet's own stack: Details first, with Look and Description editors sliding in
// inside the one form sheet the root presents.

import { Stack } from "expo-router";
import { Platform } from "react-native";
import { usePalette } from "../../src/ui/theme";

export default function ChatInfoLayout() {
  const p = usePalette();
  return (
    <Stack
      screenOptions={{
        headerShadowVisible: false,
        headerStyle: { backgroundColor: Platform.OS === "android" ? p.groupedBackground : p.dark ? "#1C1C1E" : "#F2F2F7" },
        headerTintColor: (Platform.OS === "android" ? p.label : p.tint) as any,
        headerTitleStyle: { color: p.label as any },
        headerTitleAlign: Platform.OS === "android" ? "left" : undefined,
        headerBackButtonDisplayMode: "minimal",
        contentStyle: { backgroundColor: p.groupedBackground },
      }}
    >
      <Stack.Screen name="[id]" />
      <Stack.Screen name="look/[id]" />
      <Stack.Screen name="description/[id]" />
    </Stack>
  );
}
