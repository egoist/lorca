import { FlashList } from "@shopify/flash-list";
import { MenuView, type MenuAction, type MenuComponentRef } from "@expo/ui/community/menu";
import * as Haptics from "expo-haptics";
import { Link, Stack, useRouter } from "expo-router";
import { useEffect, useMemo, useRef, useState } from "react";
import { Alert, Platform, Pressable, StyleSheet, Text, useWindowDimensions, type StyleProp, type TextStyle, View } from "react-native";
import { chatTitle, engine } from "../src/core/engine";
import type { Bot, Chat, ChatSearchResults } from "../src/core/model";
import { markRead, useBotMap, useStore, useWorkingBotIds } from "../src/core/store";
import { t } from "../src/i18n";
import { AvatarCluster } from "../src/ui/Avatar";
import { ChatPeek } from "../src/ui/ChatPeek";
import { ChatRow } from "../src/ui/ChatRow";
import { lastActivity, preview, stamp } from "../src/ui/format";
import { Symbol } from "../src/ui/Symbol";
import { Font, usePalette } from "../src/ui/theme";
import { AndroidIcons } from "../src/ui/navigation";

export default function ChatsScreen() {
  const p = usePalette();
  const router = useRouter();
  const chats = useStore((s) => s.chats);
  const running = useStore((s) => s.running);
  const relayConnected = useStore((s) => s.relayConnected);
  const bots = useBotMap();
  const workingBots = useWorkingBotIds();
  const [query, setQuery] = useState("");
  const [matches, setMatches] = useState<ChatSearchResults>({ chats: [], messages: [] });
  const [searching, setSearching] = useState(false);

  function updateQuery(value: string) {
    setQuery(value);
    setMatches({ chats: [], messages: [] });
    setSearching(!!value.trim());
  }

  useEffect(() => {
    const value = query.trim();
    if (!value) {
      setSearching(false);
      return;
    }
    let active = true;
    setSearching(true);
    const timer = setTimeout(() => {
      void engine
        .searchChats(value)
        .then((results) => {
          if (active) setMatches(results);
        })
        .catch((error) => console.warn("searching chats", error instanceof Error ? error.message : error))
        .finally(() => {
          if (active) setSearching(false);
        });
    }, 140);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [query]);

  const items = useMemo(() => {
    const q = query.trim().toLowerCase();
    const visible = chats
      .filter((c) => !q || chatTitle(c).toLowerCase().includes(q) || c.bot_ids.some((id) => bots.get(id)?.name.toLowerCase().includes(q)))
      .sort((a, b) => lastActivity(b) - lastActivity(a));
    return [...visible.filter((c) => c.is_pinned), ...visible.filter((c) => !c.is_pinned)];
  }, [chats, bots, query]);

  const searchRows = useMemo(() => {
    if (!query.trim()) return [];
    const byId = new Map(chats.map((chat) => [chat.id, chat]));
    const seen = new Set<string>();
    const rows: SearchRow[] = [];
    for (const chat of items) {
      seen.add(chat.id);
      rows.push({ key: `chat:${chat.id}`, kind: "chat", chat, snippet: preview(chat, bots) });
    }
    for (const hit of matches.chats) {
      const chat = byId.get(hit.chat_id);
      if (!chat || seen.has(chat.id)) continue;
      seen.add(chat.id);
      rows.push({ key: `chat:${chat.id}`, kind: "chat", chat, snippet: hit.snippet });
    }
    for (const hit of matches.messages) {
      const chat = byId.get(hit.chat_id);
      if (!chat) continue;
      rows.push({ key: `message:${hit.message_id}`, kind: "message", chat, snippet: hit.snippet, createdAt: hit.created_at });
    }
    return rows;
  }, [chats, bots, items, matches, query]);

  const searchingText = query.trim();
  const data: (Chat | SearchRow)[] = searchingText ? searchRows : items;

  function isWorking(chat: Chat): boolean {
    return Object.values(running).some((r) => r.chatId === chat.id) || chat.bot_ids.some((id) => workingBots.has(id));
  }

  function confirmDelete(chat: Chat) {
    Alert.alert(t("Delete “{name}”?", { name: chatTitle(chat) }), t("The chat and its messages are removed from every paired Device."), [
      { text: t("Cancel"), style: "cancel" },
      { text: t("Delete"), style: "destructive", onPress: () => engine.deleteChat(chat.id) },
    ]);
  }

  return (
    <>
      <Stack.SearchBar
        placeholder={t("Search chats and messages")}
        onChangeText={(e) => updateQuery(e.nativeEvent.text)}
        onCancelButtonPress={() => updateQuery("")}
        hideWhenScrolling
        autoCapitalize="none"
        headerIconColor={Platform.OS === "android" ? p.secondaryLabel : undefined}
      />
      {Platform.OS === "ios" ? (
        <>
          <Stack.Toolbar placement="left">
            <Stack.Toolbar.Button icon="gearshape" accessibilityLabel={t("Settings")} onPress={() => router.push("/settings")} />
          </Stack.Toolbar>
          <Stack.Toolbar placement="right">
            <Stack.Toolbar.Menu icon="square.and.pencil" accessibilityLabel={t("New")}>
              <Stack.Toolbar.MenuAction icon="person.2.fill" onPress={() => router.push("/new-group")}>
                {t("New Group Chat")}
              </Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon="person.badge.plus" onPress={() => router.push("/new-bot")}>
                {t("New Bot")}
              </Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
          </Stack.Toolbar>
        </>
      ) : (
        <>
          <Stack.Toolbar placement="left" tintColor={p.secondaryLabel}>
            <Stack.Toolbar.Button icon={AndroidIcons.settings} accessibilityLabel={t("Settings")} onPress={() => router.push("/settings")} />
          </Stack.Toolbar>
          <Stack.Toolbar placement="right" tintColor={p.secondaryLabel}>
            <Stack.Toolbar.Menu icon={AndroidIcons.add} accessibilityLabel={t("New")}>
              <Stack.Toolbar.MenuAction icon={AndroidIcons.group} onPress={() => router.push("/new-group")}>
                {t("New Group Chat")}
              </Stack.Toolbar.MenuAction>
              <Stack.Toolbar.MenuAction icon={AndroidIcons.personAdd} onPress={() => router.push("/new-bot")}>
                {t("New Bot")}
              </Stack.Toolbar.MenuAction>
            </Stack.Toolbar.Menu>
          </Stack.Toolbar>
        </>
      )}
      <View style={styles.screen}>
        <FlashList
          style={styles.list}
          data={data}
          keyExtractor={(item) => ("key" in item ? item.key : item.id)}
          contentInsetAdjustmentBehavior="automatic"
          keyboardDismissMode="on-drag"
          contentContainerStyle={{ paddingBottom: 24 }}
          ListHeaderComponent={
            relayConnected ? null : (
              <View style={[styles.banner, { backgroundColor: p.fill }]}>
                <Symbol name="antenna.radiowaves.left.and.right" size={14} color={p.secondaryLabel} />
                <Text style={[styles.bannerText, { color: p.secondaryLabel }]}>{t("Connecting to the relay…")}</Text>
              </View>
            )
          }
          ListEmptyComponent={
            <View style={styles.empty}>
              <Symbol name="sparkles" size={36} color={p.tertiaryLabel} />
              <Text style={[styles.emptyTitle, { color: p.label }]}>{query ? (searching ? t("Searching…") : t("No matches")) : t("No chats yet")}</Text>
              <Text style={[styles.emptyText, { color: p.secondaryLabel }]}>{query ? t("Try another word.") : t("Your bots and their chats sync from the relay once this phone hears from your Runner.")}</Text>
            </View>
          }
          renderItem={({ item }) => {
            if ("key" in item) {
              return <SearchResultRow item={item} bots={bots} query={searchingText} onPress={() => router.push(`/chat/${item.chat.id}`)} />;
            }
            const chat = item;
            const title = chatTitle(chat);
            if (Platform.OS === "android") {
              return (
                <AndroidChatRow
                  chat={chat}
                  bots={bots}
                  title={title}
                  working={isWorking(chat)}
                  onPress={() => router.push(`/chat/${chat.id}`)}
                  onDelete={() => confirmDelete(chat)}
                />
              );
            }
            const row = (
              <ChatRow
                chat={chat}
                bots={bots}
                title={title}
                working={isWorking(chat)}
                onPress={() => router.push(`/chat/${chat.id}`)}
              />
            );
            return (
              <Link href={`/chat/${chat.id}`} asChild>
                <Link.Trigger>{row}</Link.Trigger>
                <Link.Preview style={{ width: 340, height: 420 }}>
                  <ChatPeek chat={chat} bots={bots} title={title} />
                </Link.Preview>
                <Link.Menu>
                  <Link.MenuAction icon={chat.is_pinned ? "pin.slash" : "pin"} onPress={() => engine.pinChat(chat.id, !chat.is_pinned)}>
                    {chat.is_pinned ? t("Unpin") : t("Pin")}
                  </Link.MenuAction>
                  {chat.unread_count > 0 ? (
                    <Link.MenuAction icon="checkmark.circle" onPress={() => markRead(chat.id)}>
                      {t("Mark as Read")}
                    </Link.MenuAction>
                  ) : null}
                  <Link.MenuAction icon="trash" destructive onPress={() => confirmDelete(chat)}>
                    {t("Delete")}
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
      </View>
    </>
  );
}

function AndroidChatRow({ chat, bots, title, working, onPress, onDelete }: { chat: Chat; bots: Map<string, Bot>; title: string; working: boolean; onPress: () => void; onDelete: () => void }) {
  const menuRef = useRef<MenuComponentRef>(null);
  const { width } = useWindowDimensions();
  const actions: MenuAction[] = [
    { id: "pin", title: chat.is_pinned ? t("Unpin") : t("Pin"), image: AndroidIcons.pin, state: chat.is_pinned ? "on" : "off" },
    ...(chat.unread_count > 0 ? [{ id: "read", title: t("Mark as Read"), image: AndroidIcons.read } satisfies MenuAction] : []),
    { id: "delete", title: t("Delete"), image: AndroidIcons.delete, attributes: { destructive: true } },
  ];
  return (
    <MenuView
      ref={menuRef}
      actions={actions}
      shouldOpenOnLongPress
      style={{ width }}
      onOpenMenu={() => void Haptics.selectionAsync()}
      onPressAction={({ nativeEvent }) => {
        if (nativeEvent.event === "pin") void engine.pinChat(chat.id, !chat.is_pinned);
        else if (nativeEvent.event === "read") markRead(chat.id);
        else if (nativeEvent.event === "delete") onDelete();
      }}
    >
      <View style={{ width }}>
        <ChatRow chat={chat} bots={bots} title={title} working={working} onPress={onPress} onLongPress={() => menuRef.current?.show()} />
      </View>
    </MenuView>
  );
}

type SearchRow = {
  key: string;
  kind: "chat" | "message";
  chat: Chat;
  snippet: string;
  createdAt?: number;
};

function SearchResultRow({ item, bots, query, onPress }: { item: SearchRow; bots: Map<string, Bot>; query: string; onPress: () => void }) {
  const p = usePalette();
  const members = item.chat.bot_ids.map((id) => bots.get(id)).filter((bot): bot is Bot => !!bot);
  const at = item.createdAt ?? lastActivity(item.chat);
  return (
    <Pressable onPress={onPress} style={({ pressed }) => [styles.searchRow, { backgroundColor: pressed ? p.fill : "transparent" }]}>
      <AvatarCluster bots={members} size={44} />
      <View style={styles.searchText}>
        <View style={styles.searchTitleLine}>
          <HighlightedText text={chatTitle(item.chat)} query={query} style={[styles.searchTitle, { color: p.label }]} numberOfLines={1} />
          <Text style={[styles.searchStamp, { color: p.secondaryLabel }]}>{stamp(new Date(at * 1000))}</Text>
        </View>
        <HighlightedText text={item.snippet} query={query} style={[styles.searchSnippet, { color: p.secondaryLabel }]} numberOfLines={2} />
      </View>
      {item.kind === "message" ? <Symbol name="text.bubble" size={13} color={p.tertiaryLabel} /> : null}
    </Pressable>
  );
}

function HighlightedText({ text, query, style, numberOfLines }: { text: string; query: string; style: StyleProp<TextStyle>; numberOfLines?: number }) {
  const p = usePalette();
  const matchStyle: TextStyle = { color: p.label, backgroundColor: p.secondaryFill, fontWeight: "700" };
  const terms = query.match(/[\p{L}\p{N}]+/gu) ?? [];
  if (!terms.length) return <Text style={style} numberOfLines={numberOfLines}>{text}</Text>;
  const expression = new RegExp(`(${terms.map((term) => term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`, "giu");
  const folded = new Set(terms.map((term) => term.toLocaleLowerCase()));
  return (
    <Text style={style} numberOfLines={numberOfLines}>
      {text.split(expression).map((part, index) =>
        folded.has(part.toLocaleLowerCase()) ? (
          <Text key={`${index}:${part}`} style={matchStyle}>
            {part}
          </Text>
        ) : (
          part
        ),
      )}
    </Text>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1 },
  list: { flex: 1 },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: 78 },
  banner: { flexDirection: "row", alignItems: "center", gap: 8, marginHorizontal: 16, marginTop: 4, marginBottom: 6, paddingHorizontal: 12, paddingVertical: 8, borderRadius: 10 },
  bannerText: { fontSize: Font.small },
  empty: { alignItems: "center", paddingTop: 120, paddingHorizontal: 40, gap: 8 },
  emptyTitle: { fontSize: 20, fontWeight: "600", marginTop: 8 },
  emptyText: { fontSize: 15, textAlign: "center", lineHeight: 21 },
  searchRow: { flexDirection: "row", alignItems: "center", paddingHorizontal: 16, paddingVertical: 9, gap: 12 },
  searchText: { flex: 1, gap: 3 },
  searchTitleLine: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 8 },
  searchTitle: { flex: 1, fontSize: Font.body, fontWeight: "600" },
  searchStamp: { fontSize: 14 },
  searchSnippet: { fontSize: 15, lineHeight: 20 },
});
