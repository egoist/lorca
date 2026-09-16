import { FlashList } from "@shopify/flash-list";
import * as Haptics from "expo-haptics";
import { Link, Stack, useRouter } from "expo-router";
import { useMemo, useState } from "react";
import { Alert, Platform, StyleSheet, Text, View } from "react-native";
import { chatTitle, engine } from "../src/core/engine";
import type { Chat } from "../src/core/model";
import { useBotMap, useStore, useWorkingBotIds } from "../src/core/store";
import { ChatPeek } from "../src/ui/ChatPeek";
import { ChatRow } from "../src/ui/ChatRow";
import { lastActivity } from "../src/ui/format";
import { Symbol } from "../src/ui/Symbol";
import { Font, usePalette } from "../src/ui/theme";

export default function ChatsScreen() {
  const p = usePalette();
  const router = useRouter();
  const chats = useStore((s) => s.chats);
  const running = useStore((s) => s.running);
  const relayConnected = useStore((s) => s.relayConnected);
  const bots = useBotMap();
  const workingBots = useWorkingBotIds();
  const [query, setQuery] = useState("");

  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    const visible = chats
      .filter((c) => !q || chatTitle(c).toLowerCase().includes(q) || c.bot_ids.some((id) => bots.get(id)?.name.toLowerCase().includes(q)))
      .sort((a, b) => lastActivity(b) - lastActivity(a));
    return [...visible.filter((c) => c.is_pinned), ...visible.filter((c) => !c.is_pinned)];
  }, [chats, bots, query]);

  function isWorking(chat: Chat): boolean {
    return Object.values(running).some((r) => r.chatId === chat.id) || chat.bot_ids.some((id) => workingBots.has(id));
  }

  function confirmDelete(chat: Chat) {
    Alert.alert(`Delete “${chatTitle(chat)}”?`, "The chat and its messages are removed from every paired Device.", [
      { text: "Cancel", style: "cancel" },
      { text: "Delete", style: "destructive", onPress: () => engine.deleteChat(chat.id) },
    ]);
  }

  return (
    <>
      <Stack.SearchBar placeholder="Search" onChangeText={(e) => setQuery(e.nativeEvent.text)} onCancelButtonPress={() => setQuery("")} hideWhenScrolling autoCapitalize="none" />
      <Stack.Toolbar placement="left">
        <Stack.Toolbar.Button icon="gearshape" accessibilityLabel="Settings" onPress={() => router.push("/settings")} />
      </Stack.Toolbar>
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Menu icon="square.and.pencil" accessibilityLabel="New">
          <Stack.Toolbar.MenuAction icon="person.2.fill" onPress={() => router.push("/new-group")}>
            New Group Chat
          </Stack.Toolbar.MenuAction>
          <Stack.Toolbar.MenuAction icon="person.badge.plus" onPress={() => router.push("/new-bot")}>
            New Bot
          </Stack.Toolbar.MenuAction>
        </Stack.Toolbar.Menu>
      </Stack.Toolbar>
      <FlashList
        data={items}
        keyExtractor={(chat) => chat.id}
        contentInsetAdjustmentBehavior="automatic"
        keyboardDismissMode="on-drag"
        contentContainerStyle={{ paddingBottom: 24 }}
        ListHeaderComponent={
          relayConnected ? null : (
            <View style={[styles.banner, { backgroundColor: p.fill }]}>
              <Symbol name="antenna.radiowaves.left.and.right" size={14} color={p.secondaryLabel} />
              <Text style={[styles.bannerText, { color: p.secondaryLabel }]}>Connecting to the relay…</Text>
            </View>
          )
        }
        ListEmptyComponent={
          <View style={styles.empty}>
            <Symbol name="sparkles" size={36} color={p.tertiaryLabel} />
            <Text style={[styles.emptyTitle, { color: p.label }]}>{query ? "No matches" : "No chats yet"}</Text>
            <Text style={[styles.emptyText, { color: p.secondaryLabel }]}>{query ? "Try another name." : "Your bots and their chats sync from the relay once this phone hears from your Runner."}</Text>
          </View>
        }
        renderItem={({ item: chat }) => {
          const title = chatTitle(chat);
          const row = <ChatRow chat={chat} bots={bots} title={title} working={isWorking(chat)} onPress={() => router.push(`/chat/${chat.id}`)} />;
          if (Platform.OS !== "ios") return row;
          return (
            <Link href={`/chat/${chat.id}`} asChild>
              <Link.Trigger>{row}</Link.Trigger>
              <Link.Preview style={{ width: 340, height: 420 }}>
                <ChatPeek chat={chat} bots={bots} title={title} />
              </Link.Preview>
              <Link.Menu>
                <Link.MenuAction icon={chat.is_pinned ? "pin.slash" : "pin"} onPress={() => engine.pinChat(chat.id, !chat.is_pinned)}>
                  {chat.is_pinned ? "Unpin" : "Pin"}
                </Link.MenuAction>
                {chat.unread_count > 0 ? (
                  <Link.MenuAction icon="checkmark.circle" onPress={() => useStore.setState((s) => ({ chats: s.chats.map((c) => (c.id === chat.id ? { ...c, unread_count: 0 } : c)) }))}>
                    Mark as Read
                  </Link.MenuAction>
                ) : null}
                <Link.MenuAction icon="trash" destructive onPress={() => confirmDelete(chat)}>
                  Delete
                </Link.MenuAction>
              </Link.Menu>
            </Link>
          );
        }}
        ItemSeparatorComponent={() => <View style={[styles.separator, { backgroundColor: p.separator }]} />}
        onRefresh={() => {
          void Haptics.selectionAsync();
          engine.notify();
        }}
        refreshing={false}
      />
    </>
  );
}

const styles = StyleSheet.create({
  separator: { height: StyleSheet.hairlineWidth, marginLeft: 78 },
  banner: { flexDirection: "row", alignItems: "center", gap: 8, marginHorizontal: 16, marginTop: 4, marginBottom: 6, paddingHorizontal: 12, paddingVertical: 8, borderRadius: 10 },
  bannerText: { fontSize: Font.small },
  empty: { alignItems: "center", paddingTop: 120, paddingHorizontal: 40, gap: 8 },
  emptyTitle: { fontSize: 20, fontWeight: "600", marginTop: 8 },
  emptyText: { fontSize: 15, textAlign: "center", lineHeight: 21 },
});
