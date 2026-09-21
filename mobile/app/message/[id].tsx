// The whole bot-to-bot message behind a transcript marker ("Messaged ◉ Paddock"), as a sheet.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { Platform, ScrollView, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import { useChat } from "../../src/core/store";
import { t } from "../../src/i18n";
import { Markdown } from "../../src/ui/Markdown";
import { usePalette } from "../../src/ui/theme";
import { CloseToolbar } from "../../src/ui/navigation";

export default function MessageScreen() {
  const { id, chat: chatId, title } = useLocalSearchParams<{ id: string; chat: string; title?: string }>();
  const router = useRouter();
  const p = usePalette();
  const chat = useChat(chatId);
  const { width } = useWindowDimensions();
  const message = chat?.messages.find((m) => m.id === id);
  const body = message?.body;
  const text = body?.kind === "tool" ? body.detail : body?.kind === "handoff" ? body.reason : "";
  return (
    <View style={styles.screen}>
      <Stack.Screen options={{ title: title ?? t("Message") }} />
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.dismiss()} />
      <ScrollView contentContainerStyle={styles.content} contentInsetAdjustmentBehavior="automatic">
        {text ? <Markdown text={text} color={p.label} maxWidth={width - 40} /> : <Text style={{ color: p.secondaryLabel }}>{t("This message is no longer available.")}</Text>}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1 },
  content: { paddingHorizontal: 20, paddingTop: 16, paddingBottom: 40 },
});
