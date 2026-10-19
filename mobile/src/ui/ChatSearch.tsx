// Searching the chats: their titles and members on the phone, and their messages through the
// core's `chats.search`. The phone's Search tab and the sidebar's field at the foot of the list
// both show these rows.

import { FlashList } from "@shopify/flash-list";
import { Stack, useRouter } from "expo-router";
import { useEffect, useMemo, useRef, useState } from "react";
import { Platform, StyleSheet, Text, TextInput, View, type StyleProp, type TextStyle } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { chatTitle, engine } from "../core/engine";
import type { Bot, Chat, ChatSearchResults } from "../core/model";
import { useBotMap, useStore } from "../core/store";
import { t, useLanguage } from "../i18n";
import { AvatarCluster } from "./Avatar";
import { Surface } from "./Composer";
import { lastActivity, preview, stamp } from "./format";
import { Pressable } from "./Pressable";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";

/// The rows for a query: chats whose title or a member's name has it, then chats and messages
/// the core found it in, asked a moment after the last keystroke. `searching` holds until the
/// core answers for the query on screen.
export function useChatSearch(query: string): { rows: SearchRow[]; searching: boolean } {
  const { language } = useLanguage();
  const chats = useStore((s) => s.chats);
  const bots = useBotMap();
  const q = query.trim();
  const [found, setFound] = useState<{ query: string; matches: ChatSearchResults }>({ query: "", matches: { chats: [], messages: [] } });

  useEffect(() => {
    if (!q) return;
    let active = true;
    const timer = setTimeout(() => {
      void engine
        .searchChats(q)
        .then((matches) => {
          if (active) setFound({ query: q, matches });
        })
        .catch((error) => {
          console.warn("searching chats", error instanceof Error ? error.message : error);
          if (active) setFound({ query: q, matches: { chats: [], messages: [] } });
        });
    }, 140);
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [q]);

  const rows = useMemo(() => {
    if (!q) return [];
    const lower = q.toLowerCase();
    const named = chats
      .filter((c) => chatTitle(c).toLowerCase().includes(lower) || c.bot_ids.some((id) => bots.get(id)?.name.toLowerCase().includes(lower)))
      .sort((a, b) => lastActivity(b) - lastActivity(a));
    const byId = new Map(chats.map((chat) => [chat.id, chat]));
    const seen = new Set<string>();
    const rows: SearchRow[] = [];
    for (const chat of [...named.filter((c) => c.is_pinned), ...named.filter((c) => !c.is_pinned)]) {
      seen.add(chat.id);
      rows.push({ key: `chat:${chat.id}`, kind: "chat", chat, snippet: preview(chat, bots) });
    }
    // Only the core's answer for this query; an older one would list what no longer matches.
    const matches = found.query === q ? found.matches : { chats: [], messages: [] };
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
  }, [chats, bots, found, q, language]);

  return { rows, searching: !!q && found.query !== q };
}

/// The phone's Search tab. On iOS the field is the native bar's, under the large title; on
/// Android it is a filled bar at the top, as Material's search screens have.
export function SearchScreen() {
  useLanguage();
  const p = usePalette();
  const router = useRouter();
  const bots = useBotMap();
  const [query, setQuery] = useState("");
  const { rows, searching } = useChatSearch(query);
  const text = query.trim();
  const openChat = (chat: Chat) => router.push(`/chat/${chat.id}`);

  return (
    <>
      {Platform.OS === "ios" ? (
        <Stack.SearchBar
          placeholder={t("Search")}
          onChangeText={(e) => setQuery(e.nativeEvent.text)}
          onCancelButtonPress={() => setQuery("")}
          // Under the title and always there: the screen is for searching.
          placement="stacked"
          hideWhenScrolling={false}
          autoFocus
          autoCapitalize="none"
        />
      ) : null}
      <View style={styles.screen}>
        {Platform.OS === "android" ? <TopSearchField value={query} onChangeText={setQuery} /> : null}
        <FlashList
          style={styles.screen}
          data={rows}
          keyExtractor={(item) => item.key}
          contentInsetAdjustmentBehavior="automatic"
          keyboardDismissMode="on-drag"
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={{ paddingBottom: 24 }}
          renderItem={({ item }) => <SearchResultRow item={item} bots={bots} query={text} onOpen={openChat} />}
          ItemSeparatorComponent={Separator}
          ListEmptyComponent={
            <View style={styles.empty}>
              {text ? (
                <>
                  <Text style={[styles.emptyTitle, { color: p.label }]}>{searching ? t("Searching…") : t("No matches")}</Text>
                  {searching ? null : <Text style={[styles.emptyText, { color: p.secondaryLabel }]}>{t("Try another word.")}</Text>}
                </>
              ) : (
                <Text style={[styles.emptyText, { color: p.secondaryLabel }]}>{t("Search your chats and messages.")}</Text>
              )}
            </View>
          }
        />
      </View>
    </>
  );
}

function Separator() {
  const p = usePalette();
  return <View style={[styles.separator, { backgroundColor: p.separator }]} />;
}

/// Android's field: a filled capsule under the status bar, focused when the tab opens.
function TopSearchField({ value, onChangeText }: { value: string; onChangeText: (value: string) => void }) {
  const p = usePalette();
  const input = useRef<TextInput>(null);
  const top = useSafeAreaInsets().top;
  return (
    <View style={[styles.topField, { paddingTop: top + 8 }]}>
      <Surface style={styles.field} tint={p.cell} edge={p.dark ? "rgba(255,255,255,0.14)" : "rgba(0,0,0,0.1)"} onPress={() => input.current?.focus()}>
        <Symbol name="magnifyingglass" size={20} color={p.secondaryLabel} />
        <TextInput
          ref={input}
          style={[styles.input, { color: p.label }]}
          // Uncontrolled, as the sidebar's field is: the list re-renders on every keystroke.
          onChangeText={onChangeText}
          placeholder={t("Search")}
          placeholderTextColor={p.secondaryLabel as any}
          autoFocus
          autoCapitalize="none"
          autoCorrect={false}
          returnKeyType="search"
        />
        {value ? (
          <Pressable
            onPress={() => {
              input.current?.clear();
              onChangeText("");
            }}
            hitSlop={10}
            accessibilityRole="button"
            accessibilityLabel={t("Clear")}
          >
            <Symbol name="xmark.circle.fill" size={20} color={p.secondaryLabel} />
          </Pressable>
        ) : null}
      </Surface>
    </View>
  );
}

export type SearchRow = {
  key: string;
  kind: "chat" | "message";
  chat: Chat;
  snippet: string;
  createdAt?: number;
};

export function SearchResultRow({ item, bots, query, onOpen }: { item: SearchRow; bots: Map<string, Bot>; query: string; onOpen: (chat: Chat) => void }) {
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
  screen: { flex: 1 },
  separator: { height: StyleSheet.hairlineWidth, marginLeft: 78 },
  topField: { paddingHorizontal: 16, paddingBottom: 8 },
  field: { height: 56, borderRadius: 28, flexDirection: "row", alignItems: "center", gap: 12, paddingHorizontal: 16 },
  input: { flex: 1, fontSize: Font.body, paddingVertical: 0 },
  empty: { alignItems: "center", paddingTop: 120, paddingHorizontal: 40, gap: 8 },
  emptyTitle: { fontSize: 20, fontWeight: "600" },
  emptyText: { fontSize: 15, textAlign: "center", lineHeight: 21 },
  searchRow: { flexDirection: "row", alignItems: "center", paddingHorizontal: 16, paddingVertical: 9, gap: 12 },
  searchText: { flex: 1, gap: 3 },
  searchTitleLine: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 8 },
  searchTitle: { flex: 1, fontSize: Font.body, fontWeight: "600" },
  searchStamp: { fontSize: 14 },
  searchSnippet: { fontSize: 15, lineHeight: 20 },
});
