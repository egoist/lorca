// The Running tasks sheet's own stack: the chat's commands as rows first, a command's details
// sliding in from its row, inside the one sheet the root presents.

import { Stack } from "expo-router";
import { Platform } from "react-native";
import { usePalette } from "../../src/ui/theme";

export default function TasksLayout() {
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
      <Stack.Screen name="command/[id]" />
    </Stack>
  );
}
