// The preview above a chat row's context menu: the chat's title and its last few messages,
// drawn with the same rows as the transcript, so a long press peeks at the conversation.

import { useMemo } from "react";
import { StyleSheet, Text, View } from "react-native";
import type { Bot, Chat } from "../core/model";
import { AvatarCluster } from "./Avatar";
import { usePalette } from "./theme";
import { buildRows, DayRow, MarkerRow, MessageRow, NoticeRow } from "./transcript";

const PEEK_MESSAGES = 6;
const HEADER_HEIGHT = 48;

export function ChatPeek({ chat, bots, title }: { chat: Chat; bots: Map<string, Bot>; title: string }) {
  const p = usePalette();
  const members = chat.bot_ids.map((id) => bots.get(id)).filter((b): b is Bot => !!b);
  const rows = useMemo(() => {
    const tail: Chat = { ...chat, messages: chat.messages.slice(-PEEK_MESSAGES) };
    return buildRows(tail, bots, [], false, null);
  }, [chat, bots]);
  return (
    <View style={[styles.card, { backgroundColor: p.background }]}>
      {/* The newest rows sit at the bottom; older ones are clipped under the header. */}
      <View style={styles.body}>
        <View style={styles.tail}>
          {rows.length === 0 ? (
            <Text style={[styles.empty, { color: p.tertiaryLabel }]}>No messages yet</Text>
          ) : (
            rows.map((row) => {
              switch (row.type) {
                case "day":
                  return <DayRow key={row.key} at={row.at} />;
                case "message":
                  return <MessageRow key={row.key} row={row} bots={bots} isGroup={chat.kind === "group"} />;
                case "marker":
                  return <MarkerRow key={row.key} row={row} />;
                case "notice":
                  return <NoticeRow key={row.key} row={row} />;
                default:
                  return null;
              }
            })
          )}
        </View>
      </View>
      <View style={[styles.header, { backgroundColor: p.background, borderBottomColor: p.separator }]}>
        <AvatarCluster bots={members} size={28} />
        <Text style={[styles.title, { color: p.label }]} numberOfLines={1}>
          {title}
        </Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  card: { flex: 1, overflow: "hidden" },
  body: { flex: 1, overflow: "hidden", justifyContent: "flex-end", paddingTop: HEADER_HEIGHT },
  tail: { paddingBottom: 12 },
  header: { position: "absolute", top: 0, left: 0, right: 0, height: HEADER_HEIGHT, flexDirection: "row", alignItems: "center", gap: 8, paddingHorizontal: 14, borderBottomWidth: StyleSheet.hairlineWidth },
  title: { fontSize: 15, fontWeight: "600", flexShrink: 1 },
  empty: { textAlign: "center", padding: 24, fontSize: 14 },
});
