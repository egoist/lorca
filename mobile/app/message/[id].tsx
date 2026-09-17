// The whole bot-to-bot message behind a transcript marker ("Messaged ◉ Paddock"), as a sheet.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { useChat } from "../../src/core/store";
import { Markdown } from "../../src/ui/Markdown";
import { usePalette } from "../../src/ui/theme";

export default function MessageScreen() {
  const { id, chat: chatId, title } = useLocalSearchParams<{ id: string; chat: string; title?: string }>();
  const router = useRouter();
  const p = usePalette();
  const chat = useChat(chatId);
  const message = chat?.messages.find((m) => m.id === id);
  const body = message?.body;
  const text = body?.kind === "tool" ? body.detail : body?.kind === "handoff" ? body.reason : "";
  return (
    <View style={styles.screen}>
      <Stack.Screen options={{ title: title ?? "Message" }} />
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="done" onPress={() => router.dismiss()}>
          Done
        </Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentContainerStyle={styles.content} contentInsetAdjustmentBehavior="automatic">
        {text ? <Markdown text={text} color={p.label} /> : <Text style={{ color: p.secondaryLabel }}>This message is no longer available.</Text>}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1 },
  content: { paddingHorizontal: 20, paddingTop: 16, paddingBottom: 40 },
});
