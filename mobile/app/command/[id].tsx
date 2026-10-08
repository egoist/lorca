// A command card's whole command, as a sheet: a form sheet on iOS, a Material full-screen dialog
// on Android, Copy among the bar's actions.

import * as Clipboard from "expo-clipboard";
import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { useChat } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { AndroidIcons, CloseToolbar } from "../../src/ui/navigation";
import { usePalette } from "../../src/ui/theme";

export default function CommandScreen() {
  useLanguage();
  const { id, chat: chatId, title } = useLocalSearchParams<{ id: string; chat: string; title?: string }>();
  const router = useRouter();
  const p = usePalette();
  const message = useChat(chatId)?.messages.find((m) => m.id === id);
  const body = message?.body;
  const command = body?.kind === "permission" ? (body.command ?? body.summary.replace(/^\$ /, "")) : body?.kind === "tool" ? (body.run?.command ?? "") : "";
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);
  const copy = async () => {
    await Clipboard.setStringAsync(command);
    setCopied(true);
  };
  return (
    <View style={styles.screen}>
      <Stack.Screen options={{ title: title ?? t("Command") }} />
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.dismiss()} />
      {Platform.OS === "android" ? (
        <Stack.Toolbar placement="right">
          <Stack.Toolbar.Button icon={AndroidIcons.copy} accessibilityLabel={copied ? t("Copied") : t("Copy")} disabled={!command} onPress={copy} />
        </Stack.Toolbar>
      ) : (
        <Stack.Toolbar placement="left">
          <Stack.Toolbar.Button disabled={!command} onPress={copy}>
            {copied ? t("Copied") : t("Copy")}
          </Stack.Toolbar.Button>
        </Stack.Toolbar>
      )}
      <ScrollView contentContainerStyle={styles.content} contentInsetAdjustmentBehavior="automatic">
        {command ? (
          <View style={[styles.code, { backgroundColor: p.code }]}>
            <Text selectable style={[styles.text, { color: p.label }]}>
              {command}
            </Text>
          </View>
        ) : (
          <Text style={{ color: p.secondaryLabel }}>{t("This message is no longer available.")}</Text>
        )}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1 },
  content: { padding: 16, paddingBottom: 40 },
  code: { borderRadius: 10, padding: 12 },
  text: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 13, lineHeight: 18 },
});
