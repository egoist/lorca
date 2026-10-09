// The chat info sheet's own stack: Details first, with Look, the bot's or group's Description
// editors, the chat's outputs, its durable tasks, what its bots left for review, a plugin's named
// account, the bot's browser profiles, and the limits of its turns, tasks, and routines sliding in
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
      <Stack.Screen name="group-description/[id]" />
      <Stack.Screen name="output/[id]" />
      <Stack.Screen name="outputs/[id]" />
      <Stack.Screen name="durable-task/[id]" />
      <Stack.Screen name="review/[id]" />
      <Stack.Screen name="account/[id]" />
      <Stack.Screen name="browser/[id]" />
      <Stack.Screen name="limits" />
    </Stack>
  );
}
