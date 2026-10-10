// A channel, slid in from its row in Details: whether it listens, and what holds it when a
// message's turn didn't finish or its account can't be read; where it listens and what it takes;
// the task the bot does with each message; and its conversations, each a tap away. The bot sets a
// channel up and changes it when asked in chat; here the user pauses it, settles a held message,
// or removes it.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { useShallow } from "zustand/react/shallow";
import { chatTitle, engine } from "../../../src/core/engine";
import { useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { channelChats, listenSummary, serviceName } from "../../../src/ui/channels";
import { Row, Section, ToggleRow } from "../../../src/ui/forms";
import { stamp } from "../../../src/ui/format";
import { usePalette } from "../../../src/ui/theme";

export default function ChannelScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const runner = useStore((s) => s.devices.find((d) => d.channels?.some((c) => c.id === id)));
  const channel = runner?.channels?.find((c) => c.id === id);
  const bot = useStore((s) => s.bots.find((b) => b.id === channel?.bot_id));
  const account = runner?.plugins?.find((plugin) => plugin.id === channel?.account_id);
  const conversations = useStore(useShallow((s) => s.chats.filter((c) => c.channel?.channel_id === id)));
  if (!channel) return null;
  const kept = [...conversations].sort((a, b) => (b.messages.at(-1)?.created_at ?? b.created_at) - (a.messages.at(-1)?.created_at ?? a.created_at)).slice(0, 6);
  const held = channel.state === "held";
  // A held message can be tried again or skipped; a channel waiting for the user has nothing to settle.
  const settles = held && !!channel.held_delivery;
  const footer = settles
    ? t("A message’s turn didn’t finish, so later messages wait. Read its conversation, then try it again or skip it.")
    : channel.detail || undefined;

  function confirmRemove() {
    alert(t("Remove “{name}”?", { name: channel!.name }), t("{name} stops listening there. Its conversations stay.", { name: bot?.name ?? t("The bot") }), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Remove"),
        style: "destructive",
        onPress: () => {
          engine.removeChannel(channel!.id);
          router.back();
        },
      },
    ]);
  }

  return (
    <>
      <Stack.Screen options={{ title: channel.name }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content}>
        <Section footer={footer}>
          <ToggleRow title={t("Listening")} value={channel.state !== "paused"} onValueChange={(on) => engine.setChannelPaused(channel.id, !on)} />
          {held && <Row title={t("State")} detail={t("On hold")} />}
          {settles && <Row title={t("Try Again")} action onPress={() => engine.settleHeldMessage(channel.id, true)} />}
          {settles && <Row title={t("Skip")} action onPress={() => engine.settleHeldMessage(channel.id, false)} />}
          {channel.state === "offline" && <Row title={t("State")} detail={t("Can’t connect")} />}
        </Section>
        <Section>
          <Row title={t("Account")} detail={account?.name ?? serviceName(channel.service)} />
          <Row title={t("Chats")} subtitle={channelChats(channel)} subtitleLines={3} />
          <Row title={t("Messages")} subtitle={listenSummary(channel.listen)} subtitleLines={2} />
        </Section>
        <Section title={t("Task")}>
          <View style={styles.task}>
            <Text style={[styles.taskText, { color: p.label }]} selectable>
              {channel.task}
            </Text>
          </View>
        </Section>
        {kept.length > 0 && (
          <Section title={t("Conversations")}>
            {kept.map((chat) => (
              <Row
                key={chat.id}
                title={chatTitle(chat)}
                detail={stamp(new Date((chat.messages.at(-1)?.created_at ?? chat.created_at) * 1000))}
                chevron
                onPress={() => {
                  router.dismissAll();
                  router.push(`/chat/${chat.id}`);
                }}
              />
            ))}
          </Section>
        )}
        <Section>
          <Row title={t("Remove Channel…")} destructive onPress={confirmRemove} />
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 32 },
  task: { paddingHorizontal: 16, paddingVertical: 12 },
  taskText: { fontSize: 15, lineHeight: 21 },
});
