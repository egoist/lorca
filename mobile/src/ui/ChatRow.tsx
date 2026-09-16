// A chat in the list: avatar (a breathing dot while a bot works), title, stamp, a two-line
// preview without a "You:" prefix, and the blue unread dot.

import { memo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import type { Bot, Chat } from "../core/model";
import { AvatarCluster } from "./Avatar";
import { lastActivity, preview, stamp } from "./format";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";

export const ChatRow = memo(function ChatRow({ chat, bots, title, working, onPress }: { chat: Chat; bots: Map<string, Bot>; title: string; working: boolean; onPress: () => void }) {
  const p = usePalette();
  const members = chat.bot_ids.map((id) => bots.get(id)).filter((b): b is Bot => !!b);
  const unread = chat.unread_count > 0;
  return (
    <Pressable onPress={onPress} style={({ pressed }) => [styles.row, { backgroundColor: pressed ? p.fill : "transparent" }]}>
      <View style={styles.unreadColumn}>{unread && <View style={[styles.unread, { backgroundColor: p.tint }]} />}</View>
      <AvatarCluster bots={members} size={50} working={working} />
      <View style={styles.text}>
        <View style={styles.titleLine}>
          <Text style={[styles.title, { color: p.label }]} numberOfLines={1}>
            {title}
          </Text>
          <View style={styles.stampLine}>
            <Text style={[styles.stamp, { color: p.secondaryLabel }]}>{stamp(new Date(lastActivity(chat) * 1000))}</Text>
            <Symbol name="chevron.right" size={11} color={p.tertiaryLabel} weight="semibold" />
          </View>
        </View>
        <Text style={[styles.preview, { color: p.secondaryLabel }]} numberOfLines={2}>
          {preview(chat, bots)}
        </Text>
      </View>
    </Pressable>
  );
});

const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center", paddingRight: 16, paddingVertical: 9, gap: 12 },
  unreadColumn: { width: 20, alignItems: "center" },
  unread: { width: 10, height: 10, borderRadius: 5 },
  text: { flex: 1, gap: 2 },
  titleLine: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 8 },
  title: { fontSize: Font.body, fontWeight: "600", flexShrink: 1 },
  stampLine: { flexDirection: "row", alignItems: "center", gap: 4 },
  stamp: { fontSize: 15 },
  preview: { fontSize: 15, lineHeight: 20, minHeight: 40 },
});
