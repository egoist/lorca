import { DarkTheme, DefaultTheme, Stack, ThemeProvider } from "expo-router";
import { StatusBar } from "expo-status-bar";
import { useEffect, useMemo } from "react";
import { Platform, useColorScheme } from "react-native";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import { KeyboardProvider } from "react-native-keyboard-controller";
import { engine } from "../src/core/engine";
import { useStore } from "../src/core/store";
import { t, useLanguage } from "../src/i18n";
import { usePalette } from "../src/ui/theme";

export default function RootLayout() {
  const ready = useStore((s) => s.ready);
  const paired = useStore((s) => s.paired);
  const p = usePalette();
  const scheme = useColorScheme();
  const { language } = useLanguage();

  useEffect(() => {
    void engine.start();
  }, []);

  const navigationTheme = useMemo(() => {
    const base = p.dark ? DarkTheme : DefaultTheme;
    return {
      ...base,
      colors: {
        ...base.colors,
        primary: p.tint,
        background: p.background,
        card: p.background,
        text: p.label,
        border: p.separator,
        notification: p.red,
      },
    } as any;
  }, [p]);

  if (!ready) return null;

  // Presented editors use a form sheet on iOS and a full-screen Material modal on Android.
  const sheet =
    Platform.OS === "android"
      ? ({
          presentation: "modal" as const,
          headerShown: true,
          headerShadowVisible: false,
          headerStyle: { backgroundColor: p.groupedBackground },
          contentStyle: { backgroundColor: p.groupedBackground },
        })
      : ({
          presentation: "formSheet" as const,
          sheetAllowedDetents: [1],
          headerShown: true,
          headerShadowVisible: false,
          // UIKit's native bar takes a plain color, not a dynamic system color.
          headerStyle: { backgroundColor: p.dark ? "#1C1C1E" : "#F2F2F7" },
          contentStyle: { backgroundColor: p.groupedBackground },
        });
  // react-native-screens cannot render a nested stack inside an Android form sheet. Settings
  // and Details therefore use Android's native full-screen modal presentation; their own
  // native stacks still slide child pages in exactly as they do on iOS.
  const nestedSheet = { ...sheet, headerShown: false };

  return (
    <GestureHandlerRootView style={{ flex: 1 }}>
      <KeyboardProvider>
        <StatusBar style={scheme === "dark" ? "light" : "dark"} />
        {/* The native bar takes its light/dark appearance from the navigation theme, not the OS. */}
        {/* A new language mounts the screens again, so every title and label is said anew. */}
        <ThemeProvider key={language} value={navigationTheme}>
          <Stack
            screenOptions={{
              headerTintColor: (Platform.OS === "android" ? p.label : p.tint) as any,
              headerTitleStyle: { color: p.label as any },
              headerTitleAlign: Platform.OS === "android" ? "left" : undefined,
              headerBackButtonDisplayMode: "minimal",
              contentStyle: { backgroundColor: p.background },
            }}
          >
            <Stack.Protected guard={paired}>
              <Stack.Screen name="index" options={{ title: t("Chats"), headerTitle: Platform.OS === "ios" ? "" : "Lorca", headerLargeTitle: false, headerShadowVisible: false, headerTransparent: Platform.OS === "ios" }} />
              <Stack.Screen
                name="chat/[id]"
                options={Platform.OS === "android" ? { headerShown: false } : { headerTransparent: true, headerShadowVisible: false, headerTitleAlign: "center" }}
              />
              <Stack.Screen name="chat-info" options={Platform.OS === "android" ? nestedSheet : { ...nestedSheet, sheetAllowedDetents: [0.7, 1], sheetGrabberVisible: true }} />
              <Stack.Screen name="message/[id]" options={Platform.OS === "android" ? sheet : { ...sheet, sheetAllowedDetents: [0.5, 1], sheetGrabberVisible: true }} />
              <Stack.Screen name="new-bot" options={sheet} />
              <Stack.Screen name="new-group" options={sheet} />
              <Stack.Screen name="settings" options={nestedSheet} />
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
