// A coding agent's transcript, opened from its card, as a sheet: what it was sent, what it said,
// and each command and edit, or what its pane shows when it runs in a terminal host on its Runner.
// It follows the agent while it is up, asking the Runner again every five seconds, and keeps to
// the end unless the user scrolled up to read. The pane itself is the Runner's to show.

import { Stack, useFocusEffect, useLocalSearchParams, useRouter } from "expo-router";
import { useCallback, useRef, useState } from "react";
import { Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../src/core/engine";
import { t, useLanguage } from "../../src/i18n";
import { CloseToolbar } from "../../src/ui/navigation";
import { usePalette } from "../../src/ui/theme";

/// How often the transcript is asked for again while it is on screen.
const REFRESH_MS = 5000;

export default function AgentTranscriptScreen() {
  useLanguage();
  const { id, chat: chatId, title } = useLocalSearchParams<{ id: string; chat: string; title?: string }>();
  const router = useRouter();
  const p = usePalette();
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const scroll = useRef<ScrollView>(null);
  const atEnd = useRef(true);

  useFocusEffect(
    useCallback(() => {
      let live = true;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const load = async () => {
        try {
          const next = await engine.agentTranscript(chatId, id);
          if (!live) return;
          setText(next);
          setError(null);
        } catch (e) {
          // What was shown stays; the next try may reach the Runner.
          if (live) setError(e instanceof Error ? e.message : String(e));
        }
        if (live) timer = setTimeout(load, REFRESH_MS);
      };
      void load();
      return () => {
        live = false;
        if (timer) clearTimeout(timer);
      };
    }, [chatId, id]),
  );

  return (
    <View style={styles.screen}>
      <Stack.Screen options={{ title: title ?? t("Transcript") }} />
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.dismiss()} />
      <ScrollView
        ref={scroll}
        contentContainerStyle={styles.content}
        contentInsetAdjustmentBehavior="automatic"
        scrollEventThrottle={64}
        onScroll={(e) => {
          const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
          atEnd.current = contentOffset.y + layoutMeasurement.height >= contentSize.height - 24;
        }}
        onContentSizeChange={() => {
          if (atEnd.current) scroll.current?.scrollToEnd({ animated: false });
        }}
      >
        {text ? (
          <View style={[styles.code, { backgroundColor: p.code }]}>
            <Text selectable style={[styles.text, { color: p.label }]}>
              {text}
            </Text>
          </View>
        ) : (
          <Text style={{ color: p.secondaryLabel }}>{error ?? (text === "" ? t("Nothing yet") : t("Loading…"))}</Text>
        )}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1 },
  content: { padding: 16, paddingBottom: 40 },
  code: { borderRadius: 10, padding: 12 },
  text: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 12, lineHeight: 17 },
});
