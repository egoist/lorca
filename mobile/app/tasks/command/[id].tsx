// A running task's details, slid in from its row in the Running tasks sheet: which bot runs the
// command and where it stands, Stop while it runs, the whole command, and its last lines. A
// command that ends while this is open stays readable, saying how it ended.

import { Stack, useLocalSearchParams } from "expo-router";
import { useState } from "react";
import { Platform, ScrollView, StyleSheet, Text } from "react-native";
import { engine } from "../../../src/core/engine";
import { isLive } from "../../../src/core/model";
import { useBotMap, useChat } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { BotAvatar } from "../../../src/ui/Avatar";
import { firstLine } from "../../../src/ui/format";
import { Row, Section } from "../../../src/ui/forms";
import { taskState, useNow } from "../../../src/ui/tasks";
import { usePalette } from "../../../src/ui/theme";

export default function CommandScreen() {
  useLanguage();
  const { id, chat: chatId } = useLocalSearchParams<{ id: string; chat: string }>();
  const p = usePalette();
  const chat = useChat(chatId);
  const bots = useBotMap();
  const now = useNow();
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const message = chat?.messages.find((m) => m.id === id);
  const body = message?.body.kind === "tool" ? message.body : undefined;
  const run = body?.run;
  if (!message || !body || !run) {
    return (
      <>
        <Stack.Screen options={{ title: t("Command") }} />
        <Text style={[styles.gone, { color: p.secondaryLabel }]}>{t("This command is no longer available.")}</Text>
      </>
    );
  }
  const bot = message.author.kind === "bot" ? bots.get(message.author.bot_id) : undefined;
  const live = isLive(run) && !!run.session_id;
  const output = (run.output ?? "").split("\n").filter(Boolean).join("\n");
  const stop = async () => {
    setStopping(true);
    setError(null);
    try {
      await engine.stopCommand(message.chat_id, message.id);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setStopping(false);
    }
  };
  return (
    <>
      <Stack.Screen options={{ title: body.description ?? firstLine(run.command) }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content}>
        <Section>
          <Row leading={<BotAvatar bot={bot} size={28} />} title={bot?.name ?? t("The bot")} detail={taskState(run, message.created_at, now)} />
          {live ? <Row title={t("Stop Command")} icon="stop.fill" destructive onPress={stopping ? undefined : () => void stop()} /> : null}
        </Section>
        {error && live ? <Text style={[styles.error, { color: p.red }]}>{error}</Text> : null}
        <Section title={t("Command")}>
          <Text selectable style={[styles.code, { color: p.label }]}>
            {run.command}
          </Text>
        </Section>
        {output ? (
          <Section title={t("Output")}>
            <Text selectable style={[styles.code, { color: p.label }]}>
              {output}
            </Text>
          </Section>
        ) : null}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  code: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 12.5, lineHeight: 17, paddingHorizontal: 16, paddingVertical: 12 },
  error: { fontSize: 13, lineHeight: 18, paddingHorizontal: 32, marginTop: 7 },
  gone: { textAlign: "center", marginTop: 32, fontSize: 15, paddingHorizontal: 24 },
});
