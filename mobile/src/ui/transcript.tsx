// Transcript rows, after the Mac app: bubbles (a group shows the bot's name above and its
// avatar beside the bubble's bottom edge; a DM shows neither), "Today 4:13 AM" separators after
// fifteen minutes of silence, the "is working" row, "Chef stopped without replying", and the
// centered "Message from ◉ Name" / "Messaged ◉ Name" markers. Tool calls never render.

import { useEffect } from "react";
import { Pressable, StyleSheet, Text, useWindowDimensions, View } from "react-native";
import Animated, { Easing, useAnimatedStyle, useSharedValue, withDelay, withRepeat, withSequence, withTiming } from "react-native-reanimated";
import { isSentMessage, recipientName, type Bot, type Chat, type Message } from "../core/model";
import { AttachmentBlock } from "./attachments";
import { BotAvatar } from "./Avatar";
import { daySeparator, time } from "./format";
import { Markdown } from "./Markdown";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";

export const SEPARATOR_GAP_SECS = 15 * 60;
const AVATAR = 28;
const GUTTER = 8;
const INSET = 12;

export type Row =
  | { key: string; type: "day"; at: number }
  | { key: string; type: "message"; message: Message; groupStart: boolean; groupEnd: boolean; showsName: boolean }
  | { key: string; type: "marker"; text: string; bot: Bot | undefined; tooltip?: string; groupStart: boolean }
  | { key: string; type: "notice"; text: string; groupStart: boolean }
  | { key: string; type: "working"; bots: Bot[] }
  | { key: string; type: "status"; text: string };

/// Builds the rows for a chat, the way the Mac app's ChatViewController does.
export function buildRows(chat: Chat, bots: Map<string, Bot>, workingBotIds: string[], isWorking: boolean, status: string | null): Row[] {
  const rows: Row[] = [];
  let previous: Message | null = null;
  let previousAuthorKey: string | null = null;
  const shown = chat.messages.filter((m) => m.body.kind !== "tool" || isSentMessage(m.body));
  for (let i = 0; i < shown.length; i++) {
    const message = shown[i];
    const separated = !previous || message.created_at - previous.created_at >= SEPARATOR_GAP_SECS;
    if (separated) {
      rows.push({ key: `day-${message.id}`, type: "day", at: message.created_at });
      previousAuthorKey = null;
    }
    const authorKey = message.author.kind === "bot" ? `bot:${message.author.bot_id}` : message.author.kind;
    const groupStart = separated || authorKey !== previousAuthorKey;
    switch (message.body.kind) {
      case "text": {
        const next = shown[i + 1];
        const nextKey = next ? (next.author.kind === "bot" ? `bot:${next.author.bot_id}` : next.author.kind) : null;
        const nextSeparated = next ? next.created_at - message.created_at >= SEPARATOR_GAP_SECS : true;
        const groupEnd = nextSeparated || nextKey !== authorKey || next?.body.kind !== "text";
        rows.push({
          key: message.id,
          type: "message",
          message,
          groupStart,
          groupEnd,
          showsName: chat.kind === "group" && message.author.kind === "bot" && groupStart,
        });
        previousAuthorKey = authorKey;
        break;
      }
      case "tool":
        rows.push({ key: message.id, type: "marker", text: `Messaged`, bot: bots.get(botIdNamed(bots, recipientName(message.body))), tooltip: message.body.detail, groupStart });
        previousAuthorKey = null;
        break;
      case "handoff": {
        const incoming = chat.kind !== "group" && chat.bot_ids.includes(message.body.to);
        rows.push({
          key: message.id,
          type: "marker",
          text: incoming ? "Message from" : "Handed off to",
          bot: bots.get(incoming ? message.body.from : message.body.to),
          tooltip: message.body.reason,
          groupStart,
        });
        previousAuthorKey = null;
        break;
      }
      case "notice":
        rows.push({ key: message.id, type: "notice", text: message.body.text, groupStart });
        previousAuthorKey = null;
        break;
    }
    previous = message;
  }
  if (isWorking) rows.push({ key: "working", type: "working", bots: workingBotIds.map((id) => bots.get(id)).filter((b): b is Bot => !!b) });
  else if (status) rows.push({ key: "status", type: "status", text: status });
  return rows;
}

function botIdNamed(bots: Map<string, Bot>, name: string): string {
  for (const bot of bots.values()) if (bot.name === name) return bot.id;
  return "";
}

export function DayRow({ at }: { at: number }) {
  const p = usePalette();
  return (
    <View style={styles.dayRow}>
      <Text style={[styles.caption, { color: p.secondaryLabel }]}>{daySeparator(new Date(at * 1000))}</Text>
    </View>
  );
}

export function MessageRow({ row, bots, isGroup, onLongPress }: { row: Extract<Row, { type: "message" }>; bots: Map<string, Bot>; isGroup: boolean; onLongPress?: (message: Message) => void }) {
  const p = usePalette();
  const { width: screenWidth } = useWindowDimensions();
  const { message, groupStart, groupEnd, showsName } = row;
  const isYou = message.author.kind === "you";
  const bot = message.author.kind === "bot" ? bots.get(message.author.bot_id) : undefined;
  const text = message.body.kind === "text" ? message.body.text : "";
  const attachments = message.body.kind === "text" ? (message.body.attachments ?? []) : [];
  const failed = message.state.kind === "failed";
  const showsAvatar = isGroup && !isYou;
  // The bubble column is 80% of the screen; the bubble pads 13 a side.
  const attachmentWidth = Math.floor(screenWidth * 0.8) - 26 - (showsAvatar ? AVATAR + GUTTER : 0);
  return (
    <View style={[styles.messageRow, { paddingTop: groupStart ? 14 : 3 }, isYou ? styles.messageRowYou : styles.messageRowBot]}>
      {showsAvatar && <View style={{ width: AVATAR + GUTTER, alignSelf: "flex-end" }}>{groupEnd && <BotAvatar bot={bot} size={AVATAR} />}</View>}
      <View style={[styles.bubbleColumn, isYou && styles.bubbleColumnYou]}>
        {showsName && (
          <Text style={[styles.author, { color: p.secondaryLabel }]} numberOfLines={1}>
            {bot?.name ?? "Bot"}
          </Text>
        )}
        <Pressable onLongPress={onLongPress ? () => onLongPress(message) : undefined} delayLongPress={350}>
          {({ pressed }) => (
            <View
              style={[
                styles.bubble,
                isYou ? { backgroundColor: p.userBubble, borderBottomRightRadius: groupEnd ? 6 : 18 } : { backgroundColor: failed ? "rgba(255,59,48,0.14)" : p.botBubble, borderBottomLeftRadius: groupEnd ? 6 : 18 },
                pressed && { opacity: 0.85 },
              ]}
            >
              {attachments.length > 0 && <AttachmentBlock attachments={attachments} onUserBubble={isYou} maxWidth={attachmentWidth} />}
              {text.length > 0 && <Markdown text={text} color={isYou ? p.userBubbleText : p.botBubbleText} />}
              <View style={styles.bubbleFooter}>
                {failed && <Symbol name="exclamationmark.triangle.fill" size={11} color={p.red} />}
                <Text style={[styles.time, { color: isYou ? "rgba(255,255,255,0.75)" : p.tertiaryLabel }]}>{time(new Date(message.created_at * 1000))}</Text>
              </View>
            </View>
          )}
        </Pressable>
      </View>
    </View>
  );
}

export function MarkerRow({ row }: { row: Extract<Row, { type: "marker" }> }) {
  const p = usePalette();
  return (
    <View style={[styles.centered, { paddingTop: row.groupStart ? 14 : 6 }]}>
      <View style={styles.markerLine}>
        <Text style={[styles.caption, { color: p.secondaryLabel }]}>{row.text} </Text>
        <BotAvatar bot={row.bot} size={14} />
        <Text style={[styles.caption, { color: p.secondaryLabel, fontWeight: "600" }]}> {row.bot?.name ?? "a teammate"}</Text>
      </View>
      {row.tooltip ? (
        <Text style={[styles.caption, { color: p.tertiaryLabel, marginTop: 3, maxWidth: 300, textAlign: "center" }]} numberOfLines={2}>
          {row.tooltip}
        </Text>
      ) : null}
    </View>
  );
}

export function NoticeRow({ row }: { row: Extract<Row, { type: "notice" }> }) {
  const p = usePalette();
  return (
    <View style={[styles.centered, { paddingTop: row.groupStart ? 14 : 6 }]}>
      <View style={[styles.notice, { backgroundColor: p.fill }]}>
        <Symbol name="info.circle.fill" size={14} color={p.secondaryLabel} />
        <Text style={[styles.noticeText, { color: p.secondaryLabel }]}>{row.text}</Text>
      </View>
    </View>
  );
}

export function StatusRow({ text }: { text: string }) {
  const p = usePalette();
  return (
    <View style={[styles.centered, { paddingTop: 10, paddingBottom: 4 }]}>
      <Text style={[styles.caption, { color: p.tertiaryLabel }]}>{text}</Text>
    </View>
  );
}

/// The avatar alone in a DM; "Chef is working…" in a group.
export function WorkingRow({ bots, isGroup }: { bots: Bot[]; isGroup: boolean }) {
  const p = usePalette();
  const names = bots.map((b) => b.name);
  const label = names.length === 0 ? "Working…" : names.length === 1 ? `${names[0]} is working…` : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]} are working…`;
  return (
    <View style={[styles.messageRow, styles.messageRowBot, { paddingTop: 14, alignItems: "center" }]}>
      {isGroup ? (
        <>
          <View style={{ width: AVATAR + GUTTER }}>{bots[0] && <BotAvatar bot={bots[0]} size={AVATAR} working />}</View>
          <Text style={[styles.caption, { color: p.secondaryLabel }]}>{label}</Text>
          <Dots color={p.secondaryLabel} />
        </>
      ) : (
        <>
          <View style={{ width: AVATAR + GUTTER }}>{bots[0] && <BotAvatar bot={bots[0]} size={AVATAR} working />}</View>
          <View style={[styles.bubble, { backgroundColor: p.botBubble, paddingVertical: 12 }]}>
            <Dots color={p.secondaryLabel} />
          </View>
        </>
      )}
    </View>
  );
}

function Dots({ color }: { color: any }) {
  return (
    <View style={styles.dots}>
      {[0, 1, 2].map((i) => (
        <Dot key={i} delay={i * 160} color={color} />
      ))}
    </View>
  );
}

function Dot({ delay, color }: { delay: number; color: any }) {
  const opacity = useSharedValue(0.35);
  useEffect(() => {
    opacity.value = withDelay(delay, withRepeat(withSequence(withTiming(1, { duration: 380, easing: Easing.inOut(Easing.ease) }), withTiming(0.35, { duration: 380, easing: Easing.inOut(Easing.ease) }), withTiming(0.35, { duration: 300 })), -1));
  }, [delay, opacity]);
  const style = useAnimatedStyle(() => ({ opacity: opacity.value }));
  return <Animated.View style={[styles.dot, { backgroundColor: color }, style]} />;
}

const styles = StyleSheet.create({
  dayRow: { alignItems: "center", paddingTop: 18, paddingBottom: 2 },
  caption: { fontSize: Font.caption, fontWeight: "500" },
  messageRow: { flexDirection: "row", paddingHorizontal: INSET },
  messageRowYou: { justifyContent: "flex-end" },
  messageRowBot: { justifyContent: "flex-start" },
  bubbleColumn: { maxWidth: "80%", alignItems: "flex-start" },
  bubbleColumnYou: { alignItems: "flex-end" },
  author: { fontSize: Font.author, fontWeight: "600", marginLeft: 12, marginBottom: 2 },
  bubble: { borderRadius: 18, paddingHorizontal: 13, paddingVertical: 9, gap: 2 },
  bubbleFooter: { flexDirection: "row", justifyContent: "flex-end", alignItems: "center", gap: 4, marginTop: -2 },
  time: { fontSize: 10.5, fontWeight: "500" },
  centered: { alignItems: "center", paddingHorizontal: INSET },
  markerLine: { flexDirection: "row", alignItems: "center" },
  notice: { flexDirection: "row", alignItems: "flex-start", gap: 8, borderRadius: 10, paddingHorizontal: 10, paddingVertical: 8, maxWidth: 360 },
  noticeText: { fontSize: 12.5, lineHeight: 17, flexShrink: 1 },
  dots: { flexDirection: "row", gap: 4, alignItems: "center", marginLeft: 6 },
  dot: { width: 6, height: 6, borderRadius: 3 },
});
