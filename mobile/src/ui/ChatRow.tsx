// A chat in the list: avatar (a breathing dot while a bot works), title, stamp, a two-line
// preview without a "You:" prefix, and the unread count pill beside it.

import { memo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import type { Bot, Chat } from "../core/model";
import { t } from "../i18n";
import { AvatarCluster } from "./Avatar";
import { lastActivity, preview, stamp } from "./format";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";

export const ChatRow = memo(function ChatRow({ chat, bots, title, working, onPress, onLongPress }: { chat: Chat; bots: Map<string, Bot>; title: string; working: boolean; onPress: () => void; onLongPress?: () => void }) {
  const p = usePalette();
  const members = chat.bot_ids.map((id) => bots.get(id)).filter((b): b is Bot => !!b);
  const unread = chat.unread_count;
  return (
    <Pressable onPress={onPress} onLongPress={onLongPress} style={({ pressed }) => [styles.row, { backgroundColor: pressed ? p.fill : "transparent" }]}>
      <AvatarCluster bots={members} size={50} working={working} />
      <View style={styles.text}>
        <View style={styles.titleLine}>
          <View style={styles.titleGroup}>
            <Text style={[styles.title, { color: p.label }]} numberOfLines={1}>
              {title}
            </Text>
            {chat.is_pinned && <Symbol name="pin.fill" size={12} color={p.tertiaryLabel} />}
          </View>
          <View style={styles.stampLine}>
            <Text style={[styles.stamp, { color: p.secondaryLabel }]}>{stamp(new Date(lastActivity(chat) * 1000))}</Text>
            <Symbol name="chevron.right" size={11} color={p.tertiaryLabel} weight="semibold" />
          </View>
        </View>
        <View style={styles.previewLine}>
          <Text style={[styles.preview, { color: p.secondaryLabel }]} numberOfLines={2}>
            {preview(chat, bots)}
          </Text>
          {unread > 0 && (
            <View style={[styles.badge, { backgroundColor: p.tertiaryLabel }]} accessibilityLabel={t("{count} unread", { count: unread })}>
              <Text style={styles.badgeText}>{unread > 999 ? "999+" : unread}</Text>
            </View>
          )}
        </View>
      </View>
    </Pressable>
  );
});

const styles = StyleSheet.create({
  // The avatar's left edge lines up with the toolbar buttons (16pt).
  row: { flexDirection: "row", alignItems: "center", paddingLeft: 16, paddingRight: 16, paddingVertical: 9, gap: 12 },
  text: { flex: 1, gap: 2 },
  titleLine: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 8 },
  titleGroup: { flexDirection: "row", alignItems: "center", gap: 5, flexShrink: 1 },
  title: { fontSize: Font.body, fontWeight: "600", flexShrink: 1 },
  stampLine: { flexDirection: "row", alignItems: "center", gap: 4 },
  stamp: { fontSize: 15 },
  previewLine: { flexDirection: "row", alignItems: "center", gap: 8 },
  preview: { flex: 1, fontSize: 15, lineHeight: 20, minHeight: 40 },
  badge: { minWidth: 22, height: 22, borderRadius: 11, paddingHorizontal: 7, alignItems: "center", justifyContent: "center" },
  badgeText: { color: "#fff", fontSize: 14, fontWeight: "600", fontVariant: ["tabular-nums"] },
});
