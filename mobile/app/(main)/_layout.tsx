import { Stack } from "expo-router";
import { Platform, StyleSheet, useWindowDimensions, View } from "react-native";
import { CompactWidthView } from "../../modules/lorca-core/CompactWidthView";
import { t } from "../../src/i18n";
import { PaneWidth, useSidebarWidth, useWide } from "../../src/ui/layout";
import { useStackScreenOptions } from "../../src/ui/navigation";
import { Sidebar } from "../../src/ui/Sidebar";
import { usePalette } from "../../src/ui/theme";

// A chat opened from a link or a notification still has the list under it.
export const unstable_settings = { initialRouteName: "index" };

/// The chat list and the open chat. A narrow window stacks the chat over the list. A wide one
/// keeps the list in a sidebar, and this stack is the pane beside it: its first screen is the
/// empty pane and a chat takes the place of the one before it.
export default function MainLayout() {
  const p = usePalette();
  const wide = useWide();
  const sidebarWidth = useSidebarWidth();
  const { width } = useWindowDimensions();
  const screenOptions = useStackScreenOptions();

  return (
    <View style={[styles.panes, { backgroundColor: p.background }]}>
      {wide ? (
        // Compact, as a split view's sidebar column is: the list's bar keeps its phone arrangement.
        <CompactWidthView style={[styles.sidebar, { width: sidebarWidth, borderRightColor: p.separator }]}>
          <Sidebar />
        </CompactWidthView>
      ) : null}
      <View style={styles.pane}>
        <PaneWidth value={wide ? width - sidebarWidth : width}>
          <Stack screenOptions={screenOptions}>
            <Stack.Screen name="index" options={{ title: t("Chats"), headerTitle: "", headerLargeTitle: false, headerShadowVisible: false, headerTransparent: Platform.OS === "ios" }} />
            <Stack.Screen
              name="chat/[id]"
              options={{
                ...(Platform.OS === "android" ? { headerShown: false } : { headerTransparent: true, headerShadowVisible: false, headerTitleAlign: "center" as const }),
                // Beside the sidebar there is nothing to go back to.
                ...(wide ? { headerBackVisible: false, gestureEnabled: false, animation: "none" as const } : null),
              }}
            />
          </Stack>
        </PaneWidth>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  panes: { flex: 1, flexDirection: "row" },
  sidebar: { borderRightWidth: StyleSheet.hairlineWidth },
  pane: { flex: 1 },
});
