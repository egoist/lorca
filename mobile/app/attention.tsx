// What waits on the user across chats, after the Mac's Attention popover: each coordinator's
// latest brief, then the items, urgent first. A tap opens the chat an item came from; a long
// press opens another of its chats or marks it resolved. The ⋯ menu holds the notification
// switches and the coordinator. Bots keep the list; the core sends it whole (`attention.changed`).

import { MenuView, type MenuAction, type MenuComponentRef } from "@expo/ui/community/menu";
import * as Haptics from "expo-haptics";
import { Stack, useRouter } from "expo-router";
import { useRef, useState } from "react";
import { Alert, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { request } from "../modules/lorca-core";
import type { AttentionBrief, AttentionItem } from "../src/core/attention";
import { chatTitle } from "../src/core/engine";
import type { Chat } from "../src/core/model";
import { useBotMap, useStore } from "../src/core/store";
import { t, useLanguage } from "../src/i18n";
import { BotAvatar } from "../src/ui/Avatar";
import { stamp } from "../src/ui/format";
import { Section } from "../src/ui/forms";
import { Pressable } from "../src/ui/Pressable";
import { AndroidIcons, CloseToolbar } from "../src/ui/navigation";
import { Symbol } from "../src/ui/Symbol";
import { Font, usePalette } from "../src/ui/theme";

const SYMBOLS: Record<AttentionItem["category"], string> = {
  review: "doc.text.magnifyingglass",
  blocker: "hand.raised",
  commitment: "calendar",
  change: "arrow.triangle.2.circlepath",
};

function categoryTitle(item: AttentionItem): string {
  return { review: t("Review"), blocker: t("Blocker"), commitment: t("Commitment"), change: t("Change") }[item.category];
}

/// A change the core makes; what went wrong shows in an alert.
async function perform(method: string, params: Record<string, unknown>) {
  try {
    await request(method, params);
  } catch (error) {
    Alert.alert(t("Couldn’t update Attention"), error instanceof Error ? error.message : String(error));
  }
}

export default function AttentionScreen() {
  useLanguage();
  const router = useRouter();
  const p = usePalette();
  const attention = useStore((s) => s.attention);
  const chats = useStore((s) => s.chats);
  const open = (chatId: string) => router.dismissTo(`/chat/${chatId}`);
  // A row's menu is a native view that keeps the width its content first had, so the rows wait
  // for the sheet's width and take the cell's from the start.
  const [width, setWidth] = useState(0);
  const sourceChats = (item: AttentionItem) =>
    item.sources.map((source) => chats.find((chat) => chat.id === source.chat_id)).filter((chat, index, all): chat is Chat => !!chat && all.indexOf(chat) === index);
  return (
    <>
      <Stack.Screen options={{ title: t("Attention") }} />
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.back()} />
      <OptionsMenu />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content} onLayout={(e) => setWidth(e.nativeEvent.layout.width - 32)}>
        {attention.briefs.map((brief) => (
          <Section key={brief.coordinator_bot_id}>
            <BriefCell brief={brief} onPress={() => open(brief.chat_id)} />
          </Section>
        ))}
        {attention.items.length > 0 && width > 0 ? (
          <Section>
            {attention.items.map((item) => (
              <ItemRow key={item.id} item={item} chats={sourceChats(item)} width={width} onOpen={open} />
            ))}
          </Section>
        ) : null}
        {attention.items.length === 0 && attention.briefs.length === 0 ? (
          <Text style={[styles.empty, { color: p.secondaryLabel }]}>{t("Nothing needs your attention")}</Text>
        ) : null}
      </ScrollView>
    </>
  );
}

/// Notifications: Briefs and Urgent Items. Coordinator: Automatic (a group's owner, a direct
/// chat's bot) or one bot for every chat without an owner.
function OptionsMenu() {
  const preferences = useStore((s) => s.attention.preferences);
  const bots = useStore((s) => s.bots);
  const ios = Platform.OS === "ios";
  const coordinator = preferences.default_coordinator_bot_id;
  return (
    <Stack.Toolbar placement={ios ? "left" : "right"}>
      <Stack.Toolbar.Menu icon={ios ? "ellipsis" : AndroidIcons.more} accessibilityLabel={t("Options")}>
        <Stack.Toolbar.Menu inline title={t("Notifications")}>
          <Stack.Toolbar.MenuAction isOn={preferences.summaries} onPress={() => void perform("attention.preferences", { summaries: !preferences.summaries })}>
            {t("Briefs")}
          </Stack.Toolbar.MenuAction>
          <Stack.Toolbar.MenuAction isOn={preferences.urgent_direct} onPress={() => void perform("attention.preferences", { urgent_direct: !preferences.urgent_direct })}>
            {t("Urgent Items")}
          </Stack.Toolbar.MenuAction>
        </Stack.Toolbar.Menu>
        <Stack.Toolbar.Menu inline title={t("Coordinator")}>
          <Stack.Toolbar.MenuAction isOn={!coordinator} onPress={() => void perform("attention.preferences", { default_coordinator_bot_id: null })}>
            {t("Automatic")}
          </Stack.Toolbar.MenuAction>
          {bots.map((bot) => (
            <Stack.Toolbar.MenuAction key={bot.id} isOn={coordinator === bot.id} onPress={() => void perform("attention.preferences", { default_coordinator_bot_id: bot.id })}>
              {bot.name}
            </Stack.Toolbar.MenuAction>
          ))}
        </Stack.Toolbar.Menu>
      </Stack.Toolbar.Menu>
    </Stack.Toolbar>
  );
}

/// The coordinator and when, then what was decided, what changed, and what comes next, each
/// word in a column as wide as the widest.
function BriefCell({ brief, onPress }: { brief: AttentionBrief; onPress: () => void }) {
  const p = usePalette();
  const bot = useBotMap().get(brief.coordinator_bot_id);
  const [keyWidth, setKeyWidth] = useState(0);
  const lines: [string, string][] = [
    ...brief.decisions.map((text): [string, string] => [t("Decision"), text]),
    ...brief.changes.map((text): [string, string] => [t("Changed"), text]),
    [t("Next"), brief.next_action],
  ];
  return (
    <Pressable onPress={onPress} style={({ pressed }) => [styles.cell, pressed && { backgroundColor: p.fill }]}>
      <View style={styles.briefHeader}>
        <BotAvatar bot={bot} size={22} />
        <Text style={[styles.briefName, { color: p.label }]} numberOfLines={1}>
          {bot?.name ?? t("Coordinator")}
        </Text>
        <Text style={[styles.footnote, { color: p.secondaryLabel }]}>{stamp(new Date(brief.updated_at * 1000))}</Text>
      </View>
      {lines.map(([key, text], index) => (
        <View key={index} style={styles.briefLine}>
          <Text
            style={[styles.briefKey, { color: p.secondaryLabel, minWidth: keyWidth }]}
            onLayout={(e) => {
              const width = Math.ceil(e.nativeEvent.layout.width);
              setKeyWidth((current) => Math.max(current, width));
            }}
          >
            {key}
          </Text>
          <Text style={[styles.briefText, { color: p.label }]} numberOfLines={4}>
            {text}
          </Text>
        </View>
      ))}
    </Pressable>
  );
}

/// The category's symbol, the title, the next action, and "Review · Launch copy, Writer".
function ItemRow({ item, chats, width, onOpen }: { item: AttentionItem; chats: Chat[]; width: number; onOpen: (chatId: string) => void }) {
  useLanguage();
  const p = usePalette();
  const menu = useRef<MenuComponentRef>(null);
  const ios = Platform.OS === "ios";
  const actions: MenuAction[] = [
    ...(chats.length > 1 ? chats.map((chat) => ({ id: `open:${chat.id}`, title: t("Open “{name}”", { name: chatTitle(chat) }) })) : []),
    { id: "resolve", title: t("Mark as Resolved"), image: ios ? "checkmark.circle" : AndroidIcons.check },
  ];
  const first = chats[0];
  return (
    <MenuView
      ref={menu}
      actions={actions}
      shouldOpenOnLongPress
      style={{ width }}
      onOpenMenu={() => void Haptics.selectionAsync()}
      onPressAction={({ nativeEvent }) => {
        const id = nativeEvent.event;
        if (id === "resolve") void perform("attention.resolve", { id: item.id, expected_revision: item.revision });
        else if (id.startsWith("open:")) onOpen(id.slice(5));
      }}
    >
      <Pressable
        onPress={first ? () => onOpen(first.id) : undefined}
        onLongPress={ios ? undefined : () => menu.current?.show()}
        accessibilityRole="button"
        accessibilityLabel={[item.urgent ? t("Urgent") : categoryTitle(item), item.title, item.next_action].join(". ")}
        style={({ pressed }) => [styles.item, { width }, pressed && { backgroundColor: p.fill }]}
      >
        <Symbol name={SYMBOLS[item.category]} size={20} color={item.urgent ? p.red : p.tint} style={styles.itemSymbol} />
        <View style={styles.itemText}>
          <Text style={[styles.itemTitle, { color: p.label }]} numberOfLines={2}>
            {item.title}
          </Text>
          <Text style={[styles.itemNext, { color: p.label }]} numberOfLines={3}>
            {item.next_action}
          </Text>
          <Text style={[styles.footnote, { color: p.secondaryLabel }]} numberOfLines={1}>
            <Text style={item.urgent ? { color: p.red } : undefined}>{item.urgent ? t("Urgent") : categoryTitle(item)}</Text>
            {chats.length ? ` · ${chats.map(chatTitle).join(", ")}` : ""}
          </Text>
        </View>
      </Pressable>
    </MenuView>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  empty: { textAlign: "center", marginTop: 32, fontSize: 15 },
  cell: { paddingHorizontal: 16, paddingVertical: 12, gap: 6 },
  briefHeader: { flexDirection: "row", alignItems: "center", gap: 8, marginBottom: 2 },
  briefName: { flex: 1, fontSize: Font.body, fontWeight: "600" },
  briefLine: { flexDirection: "row", gap: 10 },
  briefKey: { fontSize: 15, lineHeight: 20 },
  briefText: { flex: 1, fontSize: 15, lineHeight: 20 },
  item: { flexDirection: "row", alignItems: "flex-start", paddingHorizontal: 16, paddingVertical: 11, gap: 12 },
  itemSymbol: { marginTop: 1 },
  itemText: { flex: 1, gap: 2 },
  itemTitle: { fontSize: Font.body, fontWeight: "600" },
  itemNext: { fontSize: 15, lineHeight: 20 },
  footnote: { fontSize: Font.small, marginTop: 1 },
});
