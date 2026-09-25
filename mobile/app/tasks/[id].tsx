// A chat's running tasks, as rows in the sheet its Running tasks button opens: the commands its
// bots have running in their terminals (`useRunningTasks`), each with what it does, who runs it
// in a group, and where it stands. A row slides the command's details in. One that ends while the
// sheet is open stays, saying how it ended.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useRef } from "react";
import { Platform, ScrollView, StyleSheet, Text } from "react-native";
import { hasEnded } from "../../src/core/model";
import { useBotMap, useChat, useRunningTasks } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { firstLine } from "../../src/ui/format";
import { Row, Section } from "../../src/ui/forms";
import { CloseToolbar } from "../../src/ui/navigation";
import { taskState, useNow } from "../../src/ui/tasks";
import { usePalette } from "../../src/ui/theme";

export default function TasksScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const chat = useChat(id);
  const bots = useBotMap();
  const running = useRunningTasks(id);
  const now = useNow();
  // Every task listed since the sheet opened, so one that ends stays.
  const listed = useRef(new Set<string>());
  for (const message of running) listed.current.add(message.id);
  const tasks = (chat?.messages ?? []).filter((m) => {
    const run = m.body.kind === "tool" ? m.body.run : undefined;
    return running.includes(m) || (!!run && listed.current.has(m.id) && hasEnded(run));
  });
  return (
    <>
      <Stack.Screen options={{ title: t("Running tasks") }} />
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.dismiss()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content}>
        {tasks.length > 0 ? (
          <Section>
            {tasks.map((message) => {
              const body = message.body.kind === "tool" ? message.body : undefined;
              if (!body?.run) return null;
              const bot = message.author.kind === "bot" ? bots.get(message.author.bot_id) : undefined;
              const state = taskState(body.run, message.created_at, now);
              return (
                <Row
                  key={message.id}
                  icon="terminal"
                  title={body.description ?? firstLine(body.run.command)}
                  subtitle={chat?.kind === "group" ? `${bot?.name ?? t("The bot")} · ${state}` : state}
                  chevron
                  onPress={() => router.push({ pathname: "/tasks/command/[id]", params: { id: message.id, chat: id } })}
                />
              );
            })}
          </Section>
        ) : (
          <Text style={[styles.empty, { color: p.secondaryLabel }]}>{t("No commands are running.")}</Text>
        )}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  empty: { textAlign: "center", marginTop: 32, fontSize: 15 },
});
