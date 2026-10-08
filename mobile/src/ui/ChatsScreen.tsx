import { FlashList } from "@shopify/flash-list";
import { MenuView, type MenuAction, type MenuComponentRef } from "@expo/ui/community/menu";
import { haptic } from "./haptics";
import { Link, Stack, useRouter } from "expo-router";
import Swipeable, { type SwipeableMethods } from "react-native-gesture-handler/ReanimatedSwipeable";
import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ActivityIndicator, Platform, StyleSheet, Text, useWindowDimensions, type StyleProp, type TextStyle, View } from "react-native";
import { Pressable } from "./Pressable";
import { chatTitle, engine } from "../core/engine";
import type { Bot, Chat, ChatSearchResults } from "../core/model";
import { markRead, useBotMap, useStore, useWorkingBotIds } from "../core/store";
import { t, useLanguage } from "../i18n";
import { AvatarCluster } from "./Avatar";
import { ChatPeek } from "./ChatPeek";
import { ChatRow } from "./ChatRow";
import { PaneWidth, useSidebarWidth } from "./layout";
import { problemTitle, showRelayProblem } from "./relay";
import { lastActivity, preview, stamp } from "./format";
import { SidebarSearch, useSidebarSearchInset } from "./SidebarSearch";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";
import { AndroidIcons } from "./navigation";
import { alert } from "./alert";

// FlashList keeps the first visible row where it is when rows change, which for a list resting at
// its top means a chat moving to the top pushes the list down by one row: the new first row lands
// under the bar. The list holds its offset instead, so at the top the new first row shows.
const KEEP_OFFSET = { disabled: true };

/// The chat list: the first screen of a narrow window, the sidebar of a wide one. In the sidebar
/// a chat opens in the pane beside the list, in place of the one open there, and its row stays lit.
export function ChatsScreen({ sidebar = false }: { sidebar?: boolean }) {
  const { language } = useLanguage();
  const p = usePalette();
  const router = useRouter();
  // Only the sidebar lights the open chat; a phone's list would re-render under the chat it pushes.
  const openChatId = useStore((s) => (sidebar ? s.openChatId : null));
  const sidebarWidth = useSidebarWidth();
  // The search field sits at the foot of the list. An iPhone's native bar puts it there; the
  // sidebar and Android have their own (see SidebarSearch).
  const floatingSearch = sidebar || Platform.OS === "android";
  const searchInset = useSidebarSearchInset();
  const chats = useStore((s) => s.chats);
  const running = useStore((s) => s.running);
  const connecting = useConnecting();
  const updateRequired = useStore((s) => s.relayUpdateRequired);
  const relayError = useStore((s) => s.relayError);
  const relayUrl = useStore((s) => s.relayUrl);
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
  }, [chats, bots, query, language]);

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
  }, [chats, bots, items, matches, query, language]);

  const searchingText = query.trim();
  const data: (Chat | SearchRow)[] = searchingText ? searchRows : items;

  // Chats with a turn in flight: their rows read "Working…" in place of the preview.
  const responding = useMemo(() => new Set(Object.values(running).map((r) => r.chatId)), [running]);

  // The rows are memoized, so what they call back stays the same function across renders; it
  // reads this render's state through the ref.
  const latest = useRef({ sidebar, openChatId });
  latest.current = { sidebar, openChatId };
  const openChat = useCallback((chat: Chat) => {
    const { sidebar, openChatId } = latest.current;
    if (!sidebar) return router.push(`/chat/${chat.id}`);
    if (chat.id === openChatId) return;
    if (openChatId) router.replace(`/chat/${chat.id}`);
    else router.push(`/chat/${chat.id}`);
  }, [router]);

  const confirmDelete = useCallback((chat: Chat) => {
    const { sidebar, openChatId } = latest.current;
    alert(t("Delete “{name}”?", { name: chatTitle(chat) }), t("The chat and its messages are removed from every paired Device."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Delete"),
        style: "destructive",
        onPress: () => {
          // The pane beside the sidebar goes back to empty with its chat.
          if (sidebar && chat.id === openChatId) router.dismissTo("/");
          engine.deleteChat(chat.id);
        },
      },
    ]);
  }, [router]);

  const renderItem = useCallback(
    ({ item }: { item: Chat | SearchRow }) => {
      if ("key" in item) return <SearchResultRow item={item} bots={bots} query={searchingText} onOpen={openChat} />;
      const chat = item;
      const working = responding.has(chat.id) || chat.bot_ids.some((id) => workingBots.has(id));
      // A sidebar row has no peek: the chat opens beside it.
      if (Platform.OS === "android" || sidebar)
        return (
          <MenuChatRow
            chat={chat}
            bots={bots}
            working={working}
            responding={responding.has(chat.id)}
            selected={sidebar ? chat.id === openChatId : undefined}
            width={sidebar ? sidebarWidth : undefined}
            onOpen={openChat}
            onDelete={confirmDelete}
          />
        );
      return <PeekChatRow chat={chat} bots={bots} working={working} responding={responding.has(chat.id)} onDelete={confirmDelete} />;
    },
    [bots, confirmDelete, openChat, openChatId, responding, searchingText, sidebar, sidebarWidth, workingBots],
  );

  return (
    <>
            {floatingSearch ? null : (
        <Stack.SearchBar
          placeholder={t("Search")}
          onChangeText={(e) => updateQuery(e.nativeEvent.text)}
          onCancelButtonPress={() => updateQuery("")}
          hideWhenScrolling
          autoCapitalize="none"
        />
      )}
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
      {/* The relay status sits in the bar's title slot, so the list never moves. Once a try to
          connect has failed it says so, and a tap shows the error. */}
      {updateRequired ? (
        <Stack.Title asChild>
          <View style={styles.status} accessibilityRole="header" accessibilityLabel={t("Update Lorca to sync")}>
            <Text style={[styles.statusText, { color: p.secondaryLabel }]} numberOfLines={1}>
              {t("Update Lorca to sync")}
            </Text>
          </View>
        </Stack.Title>
      ) : connecting && relayError ? (
        <Stack.Title asChild>
          <Pressable
            style={({ pressed }) => [styles.status, pressed && styles.statusPressed]}
            accessibilityRole="button"
            accessibilityLabel={problemTitle(relayError)}
            onPress={() => showRelayProblem(relayError, relayUrl)}
          >
            <Symbol name="exclamationmark.triangle.fill" size={13} color={p.red} />
            <Text style={[styles.statusText, { color: p.secondaryLabel }]} numberOfLines={1}>
              {problemTitle(relayError)}
            </Text>
          </Pressable>
        </Stack.Title>
      ) : connecting ? (
        <Stack.Title asChild>
          <View style={styles.status} accessibilityRole="header" accessibilityLabel={t("Connecting…")}>
            <ActivityIndicator size="small" color={p.secondaryLabel as any} />
            <Text style={[styles.statusText, { color: p.secondaryLabel }]} numberOfLines={1}>
              {t("Connecting…")}
            </Text>
          </View>
        </Stack.Title>
      ) : null}
      <View style={styles.screen}>
        <FlashList
          style={styles.list}
          data={data}
          keyExtractor={(item) => ("key" in item ? item.key : item.id)}
          contentInsetAdjustmentBehavior="automatic"
          maintainVisibleContentPosition={KEEP_OFFSET}
          keyboardDismissMode="on-drag"
          onScrollBeginDrag={closeOpenRow}
          contentContainerStyle={{ paddingBottom: floatingSearch ? searchInset + 8 : 24 }}
          ListEmptyComponent={
            <View style={styles.empty}>
              <Symbol name="sparkles" size={36} color={p.tertiaryLabel} />
              <Text style={[styles.emptyTitle, { color: p.label }]}>{query ? (searching ? t("Searching…") : t("No matches")) : t("No chats yet")}</Text>
              <Text style={[styles.emptyText, { color: p.secondaryLabel }]}>{query ? t("Try another word.") : t("Your bots and their chats sync from the relay once this phone hears from your Runner.")}</Text>
            </View>
          }
          renderItem={renderItem}
          ItemSeparatorComponent={Separator}
          onRefresh={() => {
            haptic.refresh();
            engine.notify();
          }}
          refreshing={false}
        />
        {floatingSearch ? <SidebarSearch value={query} placeholder={t("Search")} onChangeText={updateQuery} /> : null}
      </View>
    </>
  );
}

/// True once the relay has been unreachable for a moment: a launch or a quick reconnect shows
/// nothing, and neither does the reconnect after coming back to the foreground, which replaces
/// the socket the suspension killed and can take a few seconds on a slow network or a VPN.
function useConnecting(): boolean {
  const relayConnected = useStore((s) => s.relayConnected);
  const activeSince = useStore((s) => s.activeSince);
  const [connecting, setConnecting] = useState(false);
  useEffect(() => {
    if (relayConnected) return setConnecting(false);
    const timer = setTimeout(() => setConnecting(true), Math.max(1000, activeSince + 4000 - Date.now()));
    return () => clearTimeout(timer);
  }, [relayConnected, activeSince]);
  return connecting;
}

/// The separator between rows: one component, so the list keeps its separators across renders.
function Separator() {
  const p = usePalette();
  return <View style={[styles.separator, { backgroundColor: p.separator }]} />;
}

/// A row whose long press opens a native menu: every row on Android, a sidebar row on iOS.
/// Memoized: the list sits under an open chat and renders again with every change to any chat.
const MenuChatRow = memo(function MenuChatRow({ chat, bots, working, responding, selected, width: fixedWidth, onOpen, onDelete }: { chat: Chat; bots: Map<string, Bot>; working: boolean; responding: boolean; selected?: boolean; width?: number; onOpen: (chat: Chat) => void; onDelete: (chat: Chat) => void }) {
  useLanguage();
  const menuRef = useRef<MenuComponentRef>(null);
  const onPress = useCallback(() => onOpen(chat), [chat, onOpen]);
  const onLongPress = useCallback(() => menuRef.current?.show(), []);
  const { width: windowWidth } = useWindowDimensions();
  const width = fixedWidth ?? windowWidth;
  // iOS says pinned through the Unpin title and glyph, as its Link menu does; Android checks the item.
  const ios = Platform.OS === "ios";
  const actions: MenuAction[] = [
    { id: "pin", title: chat.is_pinned ? t("Unpin") : t("Pin"), image: ios ? (chat.is_pinned ? "pin.slash" : "pin") : AndroidIcons.pin, state: !ios && chat.is_pinned ? "on" : "off" },
    ...(chat.unread_count > 0 ? [{ id: "read", title: t("Mark as Read"), image: ios ? "checkmark.circle" : AndroidIcons.read } satisfies MenuAction] : []),
    { id: "delete", title: t("Delete"), image: ios ? "trash" : AndroidIcons.delete, attributes: { destructive: true } },
  ];

  return (
    <MenuView
      ref={menuRef}
      actions={actions}
      shouldOpenOnLongPress
      style={{ width }}
      onOpenMenu={haptic.longPress}
      onPressAction={({ nativeEvent }) => {
        if (nativeEvent.event === "pin") void engine.pinChat(chat.id, !chat.is_pinned);
        else if (nativeEvent.event === "read") markRead(chat.id);
        else if (nativeEvent.event === "delete") onDelete(chat);
      }}
    >
      <View style={{ width }}>
        <ChatRow chat={chat} bots={bots} title={chatTitle(chat)} working={working} responding={responding} selected={selected} onPress={onPress} onLongPress={ios ? undefined : onLongPress} />
      </View>
    </MenuView>
  );
});

/// An iPhone row, as in Messages: a tap opens the chat, a long press peeks at it with its menu, a
/// swipe to the left offers Pin and Delete and one to the right Mark as Read.
const PeekChatRow = memo(function PeekChatRow({ chat, bots, working, responding, onDelete }: { chat: Chat; bots: Map<string, Bot>; working: boolean; responding: boolean; onDelete: (chat: Chat) => void }) {
  useLanguage();
  const router = useRouter();
  const title = chatTitle(chat);
  const onPress = useCallback(() => router.push(`/chat/${chat.id}`), [router, chat.id]);
  const unread = chat.unread_count > 0;
  const swipeRef = useRef<SwipeableMethods>(null);
  const opening = useCallback(() => {
    if (openRow && openRow !== swipeRef.current) openRow.close();
    openRow = swipeRef.current;
  }, []);
  const trailing = useCallback(
    (_progress: unknown, _translation: unknown, swipeable: SwipeableMethods) => (
      <View style={styles.swipeActions}>
        <SwipeAction
          icon={chat.is_pinned ? "pin.slash.fill" : "pin.fill"}
          label={chat.is_pinned ? t("Unpin") : t("Pin")}
          color="#FF9500"
          onPress={() => {
            swipeable.close();
            void engine.pinChat(chat.id, !chat.is_pinned);
          }}
        />
        <SwipeAction
          icon="trash.fill"
          label={t("Delete")}
          color="#FF3B30"
          onPress={() => {
            swipeable.close();
            onDelete(chat);
          }}
        />
      </View>
    ),
    [chat, onDelete],
  );
  const leading = useCallback(
    (_progress: unknown, _translation: unknown, swipeable: SwipeableMethods) => (
      <View style={styles.swipeActions}>
        <SwipeAction
          icon="checkmark.message.fill"
          label={t("Read")}
          color="#007AFF"
          onPress={() => {
            swipeable.close();
            markRead(chat.id);
          }}
        />
      </View>
    ),
    [chat.id],
  );
  return (
    <Swipeable ref={swipeRef} friction={1.6} overshootFriction={8} rightThreshold={40} leftThreshold={40} renderRightActions={trailing} renderLeftActions={unread ? leading : undefined} onSwipeableWillOpen={opening}>
    <Link href={`/chat/${chat.id}`} asChild>
      <Link.Trigger>
        <ChatRow chat={chat} bots={bots} title={title} working={working} responding={responding} onPress={onPress} />
      </Link.Trigger>
      <Link.Preview style={{ width: 340, height: 420 }}>
        <PaneWidth value={340}>
          <ChatPeek chat={chat} bots={bots} title={title} />
        </PaneWidth>
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
        <Link.MenuAction icon="trash" destructive onPress={() => onDelete(chat)}>
          {t("Delete")}
        </Link.MenuAction>
      </Link.Menu>
    </Link>
    </Swipeable>
  );
});

function closeOpenRow() {
  openRow?.close();
  openRow = null;
}

/// The row whose actions show: one at a time, closed when the list scrolls, as in Messages.
let openRow: SwipeableMethods | null = null;

/// One button behind a swiped row: a full-height colored cell with its symbol over its name.
function SwipeAction({ icon, label, color, onPress }: { icon: string; label: string; color: string; onPress: () => void }) {
  return (
    <Pressable onPress={onPress} style={[styles.swipeAction, { backgroundColor: color }]} accessibilityRole="button" accessibilityLabel={label}>
      <Symbol name={icon} size={20} color="#FFFFFF" />
      <Text style={styles.swipeLabel} numberOfLines={1}>
        {label}
      </Text>
    </Pressable>
  );
}

type SearchRow = {
  key: string;
  kind: "chat" | "message";
  chat: Chat;
  snippet: string;
  createdAt?: number;
};

function SearchResultRow({ item, bots, query, onOpen }: { item: SearchRow; bots: Map<string, Bot>; query: string; onOpen: (chat: Chat) => void }) {
  useLanguage();
  const onPress = () => onOpen(item.chat);
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
  swipeActions: { flexDirection: "row" },
  swipeAction: { width: 76, alignItems: "center", justifyContent: "center", gap: 4 },
  swipeLabel: { color: "#FFFFFF", fontSize: 13, fontWeight: "500" },
  screen: { flex: 1 },
  list: { flex: 1 },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: 78 },
  status: { flexDirection: "row", alignItems: "center", gap: 8 },
  statusPressed: { opacity: 0.4 },
  statusText: { fontSize: Font.small, fontWeight: "500" },
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
