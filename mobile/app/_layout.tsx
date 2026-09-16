import { DarkTheme, DefaultTheme, Stack, ThemeProvider } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { useEffect } from "react";
import { Platform, useColorScheme } from "react-native";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { KeyboardProvider } from "react-native-keyboard-controller";
import { engine } from "../src/core/engine";
import { loadStore, useStore } from "../src/core/store";
import { usePalette } from "../src/ui/theme";

export default function RootLayout() {
  const ready = useStore((s) => s.ready);
  const paired = useStore((s) => s.machineFile !== null);
  const p = usePalette();
  const scheme = useColorScheme();

  useEffect(() => {
    void loadStore().then(() => engine.bind(useStore.getState().machineFile));
  }, []);

  if (!ready) return null;

  // Form sheets with a native bar on the grouped background, like Settings and Contacts.
  const sheet = {
    presentation: "formSheet" as const,
    sheetAllowedDetents: [1],
    headerShown: true,
    headerShadowVisible: false,
    // The native bar takes a plain color, not a dynamic system one.
    headerStyle: { backgroundColor: p.dark ? "#1C1C1E" : "#F2F2F7" },
    contentStyle: { backgroundColor: p.groupedBackground },
  };

  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <KeyboardProvider>
        <StatusBar style={scheme === "dark" ? "light" : "dark"} />
        {/* The native bar takes its light/dark appearance from the navigation theme, not the OS. */}
        <ThemeProvider value={p.dark ? DarkTheme : DefaultTheme}>
          <Stack
            screenOptions={{
              headerTintColor: p.tint as any,
              headerTitleStyle: { color: p.label as any },
              headerBackButtonDisplayMode: "minimal",
              contentStyle: { backgroundColor: p.background },
            }}
          >
            <Stack.Protected guard={paired}>
              <Stack.Screen name="index" options={{ title: "Chats", headerTitle: "", headerLargeTitle: false, headerShadowVisible: false, headerTransparent: Platform.OS === "ios" }} />
              <Stack.Screen name="chat/[id]" options={{ headerTransparent: Platform.OS === "ios" }} />
              <Stack.Screen name="chat-info/[id]" options={{ ...sheet, sheetAllowedDetents: [0.7, 1], sheetGrabberVisible: true }} />
              <Stack.Screen name="new-bot" options={sheet} />
              <Stack.Screen name="new-group" options={sheet} />
              <Stack.Screen name="settings" options={sheet} />
              <Stack.Screen
                name="attachment/[id]"
                options={{ presentation: "fullScreenModal", headerShown: true, headerStyle: { backgroundColor: "#000000" }, headerTintColor: "#FFFFFF", headerTitleStyle: { color: "#FFFFFF" }, contentStyle: { backgroundColor: "#000000" } }}
              />
            </Stack.Protected>
            <Stack.Protected guard={!paired}>
              <Stack.Screen name="pair" options={{ headerShown: false, gestureEnabled: false }} />
            </Stack.Protected>
          </Stack>
        </ThemeProvider>
      </KeyboardProvider>
    </GestureHandlerRootView>
  );
}
