import { Stack } from "expo-router";
import { StyleSheet, Text, View } from "react-native";
import { t, useLanguage } from "../../src/i18n";
import { ChatsScreen } from "../../src/ui/ChatsScreen";
import { useWide } from "../../src/ui/layout";
import { Symbol } from "../../src/ui/Symbol";
import { usePalette } from "../../src/ui/theme";

/// The chat list, or the empty pane beside the sidebar that holds the list in a wide window.
export default function IndexScreen() {
  useLanguage();
  const p = usePalette();
  if (!useWide()) return <ChatsScreen />;
  return (
    <View style={styles.empty}>
      <Stack.Screen options={{ headerShown: false }} />
      <Symbol name="bubble.left.and.bubble.right" size={40} color={p.tertiaryLabel} />
      <Text style={[styles.text, { color: p.secondaryLabel }]}>{t("Select a chat")}</Text>
    </View>
  );
}

const styles = StyleSheet.create({
  empty: { flex: 1, alignItems: "center", justifyContent: "center", gap: 12 },
  text: { fontSize: 17 },
});
