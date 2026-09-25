// A chat's running tasks, as a sheet from its Running tasks button: the commands its bots have
// running in their terminals (`useRunningTasks`), each with Stop. The working row only says what
// a bot is doing, and a command's card shows only once the bot hands the command over; here every
// one shows. One that ends while the sheet is open stays, saying how it ended.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useRef } from "react";
import { Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../src/core/engine";
import { hasEnded } from "../../src/core/model";
import { useBotMap, useChat, useRunningTasks } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { CloseToolbar } from "../../src/ui/navigation";
import { TaskCard, useNow } from "../../src/ui/RunningTasks";
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
    <View style={styles.screen}>
      <Stack.Screen options={{ title: t("Running tasks") }} />
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.dismiss()} />
      <ScrollView contentContainerStyle={styles.content} contentInsetAdjustmentBehavior="automatic">
        {tasks.length > 0 ? (
          tasks.map((message) => (
            <TaskCard
              key={message.id}
              message={message}
              bot={message.author.kind === "bot" ? bots.get(message.author.bot_id) : undefined}
              isGroup={chat?.kind === "group"}
              now={now}
              onStop={() => engine.stopCommand(message.chat_id, message.id)}
            />
          ))
        ) : (
          <Text style={[styles.empty, { color: p.secondaryLabel }]}>{t("No commands are running.")}</Text>
        )}
      </ScrollView>
    </View>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1 },
  content: { paddingHorizontal: 16, paddingTop: 12, paddingBottom: 40, gap: 12 },
  empty: { textAlign: "center", marginTop: 24, fontSize: 15 },
});
