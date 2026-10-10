// Transcript rows, after the Mac app: bubbles (a group shows the bot's name above and its
// avatar beside the bubble's bottom edge; a DM shows neither), "Today 4:13 AM" separators after
// fifteen minutes of silence, the "is working" row, "Chef stopped without replying", and the
// centered "Message from ◉ Name" / "Messaged ◉ Name" markers. Tool calls never render, except a
// command, which shows as its card while it needs the user.

import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Linking, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { Pressable } from "./Pressable";
import * as Clipboard from "expo-clipboard";
import { haptic } from "./haptics";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, { runOnJS, useAnimatedStyle, useSharedValue, withSequence, withSpring, withTiming } from "react-native-reanimated";
import { LinearGradient } from "expo-linear-gradient";
import { useRouter } from "expo-router";
import { ShimmerView } from "../../modules/lorca-core/ShimmerView";
import { canBeQuoted, isLive, isSentMessage, showsCard, type Author, type Body, type Bot, type Chat, type CommandRun, type Message } from "../core/model";
import { engine } from "../core/engine";
import { useStore } from "../core/store";
import { language, t, useLanguage } from "../i18n";
import { AttachmentBlock } from "./attachments";
import { BotAvatar } from "./Avatar";
import { daySeparator, firstLine, workingActivity } from "./format";
import { Markdown } from "./Markdown";
import { Symbol } from "./Symbol";
import { usePaneWidth } from "./layout";
import { Font, usePalette } from "./theme";
import { alert } from "./alert";

export const SEPARATOR_GAP_SECS = 15 * 60;
const AVATAR = 28;
const GUTTER = 8;
/// A bubble stops growing here in a wide pane.
const BUBBLE_COLUMN_MAX = 620;
const INSET = 12;
/// A notice's line, and its icon, which centers on the first line as on the Mac.
const NOTICE_LINE = 17;
const NOTICE_ICON = 14;

export type Row =
  | { key: string; type: "day"; at: number }
  | { key: string; type: "message"; message: Message; groupStart: boolean; groupEnd: boolean; showsName: boolean }
  | { key: string; type: "marker"; text: string; bot: Bot | undefined; tooltip?: string; groupStart: boolean }
  | { key: string; type: "notice"; text: string; groupStart: boolean }
  | { key: string; type: "permission"; message: Message; body: Extract<Body, { kind: "permission" }>; bot: Bot | undefined; groupStart: boolean }
  | { key: string; type: "command"; message: Message; run: CommandRun; bot: Bot | undefined; groupStart: boolean }
  | { key: string; type: "working"; bots: Bot[] }
  | { key: string; type: "status"; text: string };

/// Builds the rows for a chat, the way the Mac app's ChatViewController does.
export function buildRows(chat: Chat, bots: Map<string, Bot>, workingBotIds: string[], isWorking: boolean, status: string | null): Row[] {
  const rows: Row[] = [];
  let previous: Message | null = null;
  let previousAuthorKey: string | null = null;
  const shown = chat.messages.filter((m) => m.body.kind !== "tool" || showsCard(m.body) || isSentMessage(m.body));
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
        if (message.body.run) {
          const bot = message.author.kind === "bot" ? bots.get(message.author.bot_id) : undefined;
          rows.push({ key: message.id, type: "command", message, run: message.body.run, bot, groupStart });
        } else {
          rows.push({ key: message.id, type: "marker", text: t("Messaged"), bot: bots.get(message.body.target_bot_id ?? ""), tooltip: message.body.detail, groupStart });
        }
        previousAuthorKey = null;
        break;
      case "handoff": {
        // From a bot outside the chat, as a DM's request or a handoff's report: a message.
        const incoming = chat.bot_ids.includes(message.body.to) && !chat.bot_ids.includes(message.body.from);
        rows.push({
          key: message.id,
          type: "marker",
          text: incoming ? t("Message from") : t("Handed off to"),
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
      case "permission":
        rows.push({ key: message.id, type: "permission", message, body: message.body, bot: message.author.kind === "bot" ? bots.get(message.author.bot_id) : undefined, groupStart });
        previousAuthorKey = null;
        break;
    }
    previous = message;
  }
  if (isWorking) rows.push({ key: "working", type: "working", bots: workingBotIds.map((id) => bots.get(id)).filter((b): b is Bot => !!b) });
  else if (status) rows.push({ key: "status", type: "status", text: status });
  return rows;
}

/// `next` with every row that says what its predecessor of the same key said replaced by that
/// predecessor, and `previous` itself when nothing changed. Rows are rebuilt on every event in
/// the chat, most of which change one row or none (a tool call the transcript does not show).
export function shareRows(previous: Row[], next: Row[]): Row[] {
  if (previous.length === 0) return next;
  const byKey = new Map(previous.map((row) => [row.key, row]));
  let changed = next.length !== previous.length;
  const shared = next.map((row, index) => {
    const old = byKey.get(row.key);
    const kept = old && sameRow(old, row) ? old : row;
    if (kept !== previous[index]) changed = true;
    return kept;
  });
  return changed ? shared : previous;
}

function sameRow(a: Row, b: Row): boolean {
  const x = a as Record<string, unknown>;
  const y = b as Record<string, unknown>;
  for (const key in y) {
    if (x[key] === y[key]) continue;
    const left = x[key];
    const right = y[key];
    // The working row's bots: the same bots in a new list.
    if (Array.isArray(left) && Array.isArray(right) && left.length === right.length && left.every((item, i) => item === right[i])) continue;
    return false;
  }
  return Object.keys(x).length === Object.keys(y).length;
}

export const DayRow = memo(function DayRow({ at }: { at: number }) {
  useLanguage();
  const p = usePalette();
  return (
    <View style={styles.dayRow}>
      <Text style={[styles.caption, { color: p.secondaryLabel }]}>{daySeparator(new Date(at * 1000))}</Text>
    </View>
  );
});

/// Swiping a bubble this far to the right makes the draft a reply to it.
const REPLY_SWIPE = 56;

/// Who wrote a quoted message, as a reply's quote names them.
export function quoteAuthorName(author: Author, bots: Map<string, Bot>): string {
  if (author.kind === "you") return t("You");
  if (author.kind === "bot") return bots.get(author.bot_id)?.name ?? t("Bot");
  return "Lorca";
}

/// How far from the screen's left edge a drag stays the system's back gesture.
const BACK_EDGE = 28;

/// Swipe to reply, as in Messages and Google Messages: the bubble, dragged to the right, takes its
/// row along with a reply arrow fading in behind it; let go past `REPLY_SWIPE` and the draft
/// answers it. Only a drag that starts on the bubble, away from the left edge, is a reply: one
/// anywhere else stays the system's back gesture (iOS 26 takes it from the whole content), as does
/// a leftward one, and vertical ones stay with the transcript. The drag runs on the UI thread, so
/// the bubble follows the finger while JS is busy with a streaming reply.
function useSwipeToReply(onReply?: () => void) {
  const offset = useSharedValue(0);
  const armed = useSharedValue(false);
  const pan = useMemo(() => {
    const reply = () => onReply?.();
    const tick = haptic.threshold;
    return Gesture.Pan()
      .enabled(!!onReply)
      .activeOffsetX(14)
      .failOffsetX(-10)
      .failOffsetY([-10, 10])
      .onTouchesDown((event, manager) => {
        if ((event.allTouches[0]?.absoluteX ?? 0) < BACK_EDGE) manager.fail();
      })
      .onUpdate((event) => {
        offset.value = Math.max(0, Math.min(REPLY_SWIPE * 1.4, event.translationX));
        const past = offset.value >= REPLY_SWIPE;
        if (past !== armed.value) {
          armed.value = past;
          if (past) runOnJS(tick)();
        }
      })
      .onEnd(() => {
        if (armed.value) runOnJS(reply)();
      })
      .onFinalize(() => {
        armed.value = false;
        offset.value = withSpring(0, { damping: 22, stiffness: 260 });
      });
  }, [onReply, offset, armed]);
  const follow = useAnimatedStyle(() => ({ transform: [{ translateX: offset.value }] }));
  const arrow = useAnimatedStyle(() => {
    const progress = Math.min(1, offset.value / REPLY_SWIPE);
    return { opacity: progress, transform: [{ scale: 0.6 + 0.4 * progress }] };
  });
  return { pan, follow, arrow };
}

/// `onReply` makes the draft a reply to this message (a swipe to the left), when it can be quoted;
/// `onQuotePress` brings the message a reply answers into view; `flashing` pulses the bubble once
/// it is there. Rows are built anew on every change to the chat, but their messages keep their
/// identity until they change, so a bubble renders again only when its own message or place
/// in the run does: a streaming reply re-renders its own bubble, not the screenful above it.
export const MessageRow = memo(function MessageRow({
  row,
  bots,
  isGroup,
  onReply,
  onQuotePress,
  flashing,
}: {
  row: Extract<Row, { type: "message" }>;
  bots: Map<string, Bot>;
  isGroup: boolean;
  onReply?: (message: Message) => void;
  onQuotePress?: (messageID: string) => void;
  flashing?: boolean;
}) {
  const held = row.message.queued === true;
  const reply = useMemo(() => (onReply && canBeQuoted(row.message) ? () => onReply(row.message) : undefined), [onReply, row.message]);
  useLanguage();
  const p = usePalette();
  const paneWidth = usePaneWidth();
  const { message, groupStart, groupEnd, showsName } = row;
  const quote = message.body.kind === "text" ? message.body.reply_to : undefined;
  const pulse = useSharedValue(1);
  useEffect(() => {
    if (!flashing) return;
    pulse.value = withSequence(withTiming(0.35, { duration: 260 }), withTiming(1, { duration: 260 }), withTiming(0.35, { duration: 260 }), withTiming(1, { duration: 260 }));
  }, [flashing, pulse]);
  const pulsing = useAnimatedStyle(() => ({ opacity: pulse.value }));
  const isYou = message.author.kind === "you";
  const bot = message.author.kind === "bot" ? bots.get(message.author.bot_id) : undefined;
  const text = message.body.kind === "text" ? message.body.text : "";
  const attachments = message.body.kind === "text" ? (message.body.attachments ?? []) : [];
  const failed = message.state.kind === "failed";
  const showsAvatar = isGroup && !isYou;
  // The bubble column is 80% of the pane, up to a readable line; the bubble pads 13 a side.
  // Attachments and the text both fit that width.
  const columnWidth = Math.min(Math.floor(paneWidth * 0.8), BUBBLE_COLUMN_MAX);
  const attachmentWidth = columnWidth - 26 - (showsAvatar ? AVATAR + GUTTER : 0);
  const quoteName = quote ? quoteAuthorName(quote.author, bots) : "";
  const swipe = useSwipeToReply(reply);
  return (
    <View>
    <Animated.View style={[styles.replyArrow, swipe.arrow]} pointerEvents="none">
      <Symbol name="arrowshape.turn.up.left.fill" size={16} color={p.secondaryLabel} />
    </Animated.View>
    <Animated.View style={swipe.follow}>
    <View style={[styles.messageRow, { paddingTop: groupStart ? 14 : 3 }, isYou ? styles.messageRowYou : styles.messageRowBot]}>
      {showsAvatar && <View style={{ width: AVATAR + GUTTER, alignSelf: "flex-end" }}>{groupEnd && <BotAvatar bot={bot} size={AVATAR} />}</View>}
      <View style={[styles.bubbleColumn, { maxWidth: columnWidth }, isYou && styles.bubbleColumnYou]}>
        {showsName && (
          <Text style={[styles.author, { color: p.secondaryLabel }]} numberOfLines={1}>
            {bot?.name ?? t("Bot")}
          </Text>
        )}
        {quote && (
          <Pressable
            onPress={() => onQuotePress?.(quote.message_id)}
            hitSlop={6}
            style={[styles.quote, isYou && styles.quoteYou]}
            accessibilityRole="button"
            accessibilityLabel={t("In reply to {name}: {text}", { name: quoteName, text: quote.text })}
          >
            <Symbol name="arrowshape.turn.up.left.fill" size={10} color={p.tertiaryLabel} />
            <Text style={[styles.quoteText, { color: p.secondaryLabel }]} numberOfLines={1}>
              <Text style={styles.quoteName}>{quoteName}:</Text> {quote.text}
            </Text>
          </Pressable>
        )}
        <GestureDetector gesture={swipe.pan}>
        <Animated.View
          style={[
            styles.bubble,
            isYou ? { backgroundColor: p.userBubble, borderBottomRightRadius: groupEnd ? 6 : 18 } : { backgroundColor: failed ? "rgba(255,59,48,0.14)" : p.botBubble, borderBottomLeftRadius: groupEnd ? 6 : 18 },
            pulsing,
            held && styles.held,
          ]}
        >
          {attachments.length > 0 && <AttachmentBlock attachments={attachments} onUserBubble={isYou} maxWidth={attachmentWidth} />}
          {text.length > 0 && <Markdown text={text} color={isYou ? p.userBubbleText : p.botBubbleText} maxWidth={attachmentWidth} />}
          {failed && (
            <View style={styles.bubbleFooter}>
              <Symbol name="exclamationmark.triangle.fill" size={11} color={p.red} />
            </View>
          )}
        </Animated.View>
        </GestureDetector>
        {held && (
          <Pressable
            onPress={() => engine.sendNow(message.chat_id, message.id).catch((error) => alert(t("Could not send now"), error instanceof Error ? error.message : String(error)))}
            hitSlop={8}
            style={styles.sendNow}
            accessibilityRole="button"
            accessibilityHint={t("Have the bot read this now. A command it is running moves to the background.")}
          >
            <Text style={[styles.sendNowText, { color: p.tint }]}>{t("Send now")}</Text>
          </Pressable>
        )}
      </View>
    </View>
    </Animated.View>
    </View>
  );
}, (a, b) =>
  a.row.message === b.row.message &&
  a.row.groupStart === b.row.groupStart &&
  a.row.groupEnd === b.row.groupEnd &&
  a.row.showsName === b.row.showsName &&
  a.bots === b.bots &&
  a.isGroup === b.isGroup &&
  a.onReply === b.onReply &&
  a.onQuotePress === b.onQuotePress &&
  a.flashing === b.flashing,
);

/// "Messaged ◉ Name" with the message's first line under it; a tap opens the whole message
/// in a sheet.
export const MarkerRow = memo(function MarkerRow({ row, onPress }: { row: Extract<Row, { type: "marker" }>; onPress?: (row: Extract<Row, { type: "marker" }>) => void }) {
  useLanguage();
  const p = usePalette();
  // Bold and code marks go, as the chat list's preview drops them.
  const preview = row.tooltip ? firstLine(row.tooltip).replace(/\*\*|`/g, "") : "";
  return (
    <Pressable
      style={({ pressed }) => [styles.centered, { paddingTop: row.groupStart ? 14 : 6, opacity: pressed ? 0.5 : 1 }]}
      onPress={preview ? () => onPress?.(row) : undefined}
      disabled={!preview}
      accessibilityRole={preview ? "button" : undefined}
      accessibilityLabel={`${row.text} ${row.bot?.name ?? t("a teammate")}${preview ? `: ${row.tooltip}` : ""}`}
    >
      <View style={styles.markerLine}>
        <Text style={[styles.caption, { color: p.secondaryLabel }]}>{row.text} </Text>
        <BotAvatar bot={row.bot} size={14} />
        <Text style={[styles.caption, { color: p.secondaryLabel, fontWeight: "600" }]}> {row.bot?.name ?? t("a teammate")}</Text>
      </View>
      {preview ? (
        <Text style={[styles.caption, { color: p.tertiaryLabel, marginTop: 3, maxWidth: 300, textAlign: "center" }]} numberOfLines={1}>
          {preview}
        </Text>
      ) : null}
    </Pressable>
  );
});

export const NoticeRow = memo(function NoticeRow({ row }: { row: Extract<Row, { type: "notice" }> }) {
  const p = usePalette();
  return (
    <View style={[styles.centered, { paddingTop: row.groupStart ? 14 : 6 }]}>
      <View style={[styles.notice, { backgroundColor: p.fill }]}>
        <Symbol name="info.circle.fill" size={NOTICE_ICON} color={p.secondaryLabel} style={styles.noticeIcon} />
        <Text style={[styles.noticeText, { color: p.secondaryLabel }]}>{row.text}</Text>
      </View>
    </View>
  );
});

/// A bot asking before a plugin tool runs, a shell command runs, or a plugin is installed. While
/// it waits: the question, the call (a shell command in a code block that opens the whole
/// command on tap), why Auto-review paused it, the answers, and under them the rule Always allow
/// adds. A shell command offers Always allow only with a rule. Once answered, the answer and the
/// call; an Always allow keeps its rule. A bot's Access refusing a call asks for more access: Edit
/// Access… opens the bot's Access, and Dismiss puts the request away; neither runs the call. In a
/// group the card sits in the bubbles' column, the bot's avatar beside its bottom edge.
export const PermissionRow = memo(function PermissionRow({ row, isGroup, onDecide }: { row: Extract<Row, { type: "permission" }>; isGroup: boolean; onDecide: (message: Message, decision: "allow" | "always" | "deny") => void }) {
  useLanguage();
  const p = usePalette();
  const [copied, setCopied] = useState(false);
  const router = useRouter();
  const showsAvatar = isGroup && row.message.author.kind === "bot";
  const pending = row.body.decision === "pending";
  const connect = row.body.tool === "connect";
  const access = row.body.tool === "access";
  const shell = row.body.plugin_id === "computer" && !access;
  const who = row.bot?.name ?? t("The bot");
  const plugin = row.body.plugin_name;
  const title = access
    ? t("{who} needs more access", { who })
    : connect
    ? t("{who} needs a sign-in to {plugin}", { who, plugin })
    : row.body.tool === "install"
      ? t("{who} wants to install {plugin}", { who, plugin })
      : shell
        ? t("{who} wants to run a command on {plugin}", { who, plugin })
        : t("{who} wants to use {plugin}", { who, plugin });
  const command = row.body.command ?? row.body.summary.replace(/^\$ /, "");
  // What an access request names: a plugin's tool as it is, or what the bot wanted to do on its
  // Runner in the CLI's English.
  const local: Record<string, string> = { "Shell commands": t("Shell commands"), "Changing files": t("Changing files"), "Reading files": t("Reading files") };
  const summary = access ? (local[row.body.summary] ?? row.body.summary) : row.body.summary;
  const reason = access ? t("Not allowed in this bot's Access settings.") : row.body.reason;
  const ruleNote = !row.body.rule
    ? undefined
    : pending
      ? t("Always allow adds the rule “{rule}”.", { rule: row.body.rule })
      : row.body.decision === "always"
        ? t("Added the rule “{rule}” to Auto-review.", { rule: row.body.rule })
        : undefined;
  const decided: Record<string, string> = connect
    ? { allowed: t("Signing in"), denied: t("Not now"), dismissed: t("Dismissed"), connected: t("Signed in"), failed: t("Sign-in failed") }
    : { allowed: t("Allowed once"), always: t("Always allowed"), denied: t("Denied"), expired: t("No answer in time"), dismissed: t("Dismissed") };
  const choices: [string, "allow" | "always" | "deny" | "access"][] = access
    ? [[t("Edit Access…"), "access"], [t("Dismiss"), "deny"]]
    : connect
    ? [[t("Sign in"), "allow"], [t("Not now"), "deny"]]
    : row.body.tool === "install"
      ? [[t("Allow"), "allow"], [t("Deny"), "deny"]]
      : shell && !row.body.rule
        ? [[t("Allow once"), "allow"], [t("Deny"), "deny"]]
        : [[t("Allow once"), "allow"], [t("Always allow"), "always"], [t("Deny"), "deny"]];
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);
  return (
    <View style={[styles.messageRow, { paddingTop: row.groupStart ? 14 : 6 }]}>
      {showsAvatar && (
        <View style={{ width: AVATAR + GUTTER, alignSelf: "flex-end" }}>
          <BotAvatar bot={row.bot} size={AVATAR} />
        </View>
      )}
      <View style={[styles.permission, { backgroundColor: p.cell, borderColor: p.separator }]}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <Symbol name={connect ? "person.crop.circle.badge.checkmark" : row.body.tool === "install" ? "puzzlepiece.extension" : "hand.raised"} size={16} color={row.body.decision === "failed" ? p.red : row.body.decision === "connected" ? p.green : p.tint} />
          <Text style={[styles.permissionTitle, { color: p.label }]} numberOfLines={2}>
            {title}
          </Text>
        </View>
        {pending && shell ? (
          <Pressable
            onPress={() => router.push({ pathname: "/command/[id]", params: { id: row.message.id, chat: row.message.chat_id, title } })}
            style={({ pressed }) => [styles.command, { backgroundColor: p.code, opacity: pressed ? 0.6 : 1 }]}
            accessibilityRole="button"
            accessibilityLabel={t("Show the full command")}
          >
            <Text style={[styles.commandText, { color: p.label }]} numberOfLines={2}>
              {command}
            </Text>
          </Pressable>
        ) : (
          <Text style={[styles.caption, { color: p.secondaryLabel }]} numberOfLines={3}>
            {pending ? summary : `${decided[row.body.decision] ?? row.body.decision} · ${summary}`}
          </Text>
        )}
        {pending && reason ? (
          <Text style={[styles.reasonText, { color: p.secondaryLabel }]}>{reason}</Text>
        ) : null}
        {row.body.decision === "allowed" && row.body.code ? (
          <View style={{ flexDirection: "row", alignItems: "center", gap: 10, marginTop: 4 }}>
            <Text selectable style={{ color: p.label, fontSize: 17, fontWeight: "700", fontFamily: "Menlo" }}>{row.body.code}</Text>
            <Pressable
              onPress={async () => {
                if (row.body.code) {
                  await Clipboard.setStringAsync(row.body.code);
                  setCopied(true);
                }
                if (row.body.link) void Linking.openURL(row.body.link);
              }}
              style={({ pressed }) => [styles.permissionButton, { backgroundColor: pressed ? p.separator : p.fill }]}
              accessibilityRole="button"
              accessibilityLabel={copied ? t("Copied") : t("Copy code and open")}
            >
              <View style={{ flexDirection: "row", alignItems: "center", gap: 5 }}>
                {copied ? <Symbol name="checkmark" size={13} color={p.green} weight="semibold" /> : null}
                <Text style={{ color: copied ? p.green : p.tint, fontSize: 13, fontWeight: "600" }}>
                  {copied ? t("Copied") : t("Copy code and open")}
                </Text>
              </View>
            </Pressable>
          </View>
        ) : null}
        {pending ? (
          <View style={{ flexDirection: "row", gap: 8, marginTop: 4 }}>
            {choices.map(([label, decision]) => (
              <Pressable
                key={decision}
                onPress={() =>
                  decision === "access"
                    ? row.message.author.kind === "bot" && router.push({ pathname: "/chat-info/access/[id]", params: { id: row.message.author.bot_id, close: "1" } })
                    : onDecide(row.message, decision)
                }
                style={({ pressed }) => [styles.permissionButton, { backgroundColor: pressed ? p.separator : p.fill }]}
              >
                <Text style={{ color: decision === "deny" ? p.label : p.tint, fontSize: 13, fontWeight: "600" }}>
                  {label}
                </Text>
              </Pressable>
            ))}
          </View>
        ) : null}
        {ruleNote ? <Text style={[styles.ruleNote, { color: p.secondaryLabel }]}>{ruleNote}</Text> : null}
      </View>
    </View>
  );
});

/// A command's card, while the command needs the user (`showsCard`). While Auto-review asks to run
/// it: who wants to, the command on one line in a code block that opens the whole command on tap,
/// why, the answers, and the rule Always allow adds. Once the bot handed the running command over:
/// Stop on the title's line, the command, and its last lines in a code block of their own that
/// scrolls; at a question, Answer, which opens `AnswerSheet`. In a group the card sits in the
/// bubbles' column, the bot's avatar beside its bottom edge.
export const CommandRow = memo(function CommandRow({
  row,
  isGroup,
  onDecide,
  onAnswer,
  onStop,
}: {
  row: Extract<Row, { type: "command" }>;
  isGroup: boolean;
  onDecide: (message: Message, decision: "allow" | "always" | "deny") => void;
  onAnswer: (message: Message) => void;
  onStop: (message: Message) => Promise<void>;
}) {
  useLanguage();
  const p = usePalette();
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const router = useRouter();
  const { run } = row;
  const showsAvatar = isGroup && row.message.author.kind === "bot";
  const who = row.bot?.name ?? t("The bot");
  const command = firstLine(run.command);
  // A new question clears what the last answer or Stop said.
  useEffect(() => setError(null), [run.output, run.state]);
  const title =
    run.state === "asking"
      ? run.device
        ? t("{who} wants to run a command on {plugin}", { who, plugin: run.device })
        : t("{who}'s command", { who })
      : run.state === "running"
        ? t("{who}'s command is running", { who })
        : t("{who}'s command is waiting for input", { who });
  const caption = run.state === "asking" ? run.reason : undefined;
  const output = isLive(run) ? (run.output ?? "").split("\n").filter(Boolean).join("\n") : "";
  const takesInput = isLive(run) && !!run.session_id;
  const choices: [string, "allow" | "always" | "deny"][] = run.rule
    ? [[t("Allow once"), "allow"], [t("Always allow"), "always"], [t("Deny"), "deny"]]
    : [[t("Allow once"), "allow"], [t("Deny"), "deny"]];
  const stop = async () => {
    setStopping(true);
    setError(null);
    try {
      await onStop(row.message);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setStopping(false);
    }
  };
  return (
    <View style={[styles.messageRow, { paddingTop: row.groupStart ? 14 : 6 }]}>
      {showsAvatar && (
        <View style={{ width: AVATAR + GUTTER, alignSelf: "flex-end" }}>
          <BotAvatar bot={row.bot} size={AVATAR} />
        </View>
      )}
      <View style={[styles.permission, { backgroundColor: p.cell, borderColor: p.separator }]}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <Symbol name="terminal" size={16} color={p.tint} />
          <Text style={[styles.permissionTitle, { color: p.label, flex: 1 }]} numberOfLines={2}>
            {title}
          </Text>
          {takesInput ? (
            <Pressable
              disabled={stopping}
              onPress={() => void stop()}
              hitSlop={6}
              style={({ pressed }) => [styles.headerButton, { backgroundColor: pressed ? p.separator : p.fill, opacity: stopping ? 0.5 : 1 }]}
              accessibilityRole="button"
            >
              <Text style={{ color: p.label, fontSize: 13, fontWeight: "600" }}>{t("Stop")}</Text>
            </Pressable>
          ) : null}
        </View>
        <Pressable
          onPress={() => router.push({ pathname: "/command/[id]", params: { id: row.message.id, chat: row.message.chat_id, title: t("{who}'s command", { who }) } })}
          style={({ pressed }) => [styles.command, { backgroundColor: p.code, opacity: pressed ? 0.6 : 1 }]}
          accessibilityRole="button"
          accessibilityLabel={t("Show the full command")}
        >
          <Text style={[styles.commandText, { color: p.label }]} numberOfLines={1}>
            {`$ ${command}`}
          </Text>
        </Pressable>
        {caption ? <Text style={[styles.reasonText, { color: p.secondaryLabel }]}>{caption}</Text> : null}
        {output ? <OutputBlock text={output} /> : null}
        {error ? <Text style={[styles.reasonText, { color: p.red }]}>{error}</Text> : null}
        {run.state === "asking" ? (
          <View style={{ flexDirection: "row", gap: 8, marginTop: 4 }}>
            {choices.map(([label, decision]) => (
              <Pressable key={decision} onPress={() => onDecide(row.message, decision)} style={({ pressed }) => [styles.permissionButton, { backgroundColor: pressed ? p.separator : p.fill }]}>
                <Text style={{ color: decision === "deny" ? p.label : p.tint, fontSize: 13, fontWeight: "600" }}>{label}</Text>
              </Pressable>
            ))}
          </View>
        ) : null}
        {run.state === "asking" && run.rule ? <Text style={[styles.ruleNote, { color: p.secondaryLabel }]}>{t("Always allow adds the rule “{rule}”.", { rule: run.rule })}</Text> : null}
        {run.state === "waiting" && takesInput ? (
          <View style={{ flexDirection: "row", marginTop: 4 }}>
            <Pressable onPress={() => onAnswer(row.message)} style={({ pressed }) => [styles.permissionButton, { backgroundColor: pressed ? p.separator : p.fill }]} accessibilityRole="button">
              <Text style={{ color: p.tint, fontSize: 13, fontWeight: "600" }}>{t("Answer")}</Text>
            </Pressable>
          </View>
        ) : null}
      </View>
    </View>
  );
});

/// A running command's last lines: a code block like the command's that grows to six lines and
/// then scrolls, the newest line in view. An edge with more lines past it fades out: Android
/// draws that itself, iOS gets a gradient in the block's color.
function OutputBlock({ text }: { text: string }) {
  const p = usePalette();
  const scroll = useRef<ScrollView>(null);
  const extent = useRef({ content: 0, frame: 0, offset: 0 });
  const [edges, setEdges] = useState({ above: false, below: false });
  const measured = () => {
    const { content, frame, offset } = extent.current;
    const above = offset > 1;
    const below = offset + frame < content - 1;
    setEdges((shown) => (shown.above === above && shown.below === below ? shown : { above, below }));
  };
  const fade = p.codeFade;
  return (
    <View style={[styles.output, { backgroundColor: p.code }]}>
      <ScrollView
        ref={scroll}
        nestedScrollEnabled
        fadingEdgeLength={16}
        style={styles.outputScroll}
        contentContainerStyle={styles.outputContent}
        scrollEventThrottle={32}
        onLayout={(e) => {
          extent.current.frame = e.nativeEvent.layout.height;
          measured();
        }}
        onContentSizeChange={(_, height) => {
          extent.current.content = height;
          scroll.current?.scrollToEnd({ animated: false });
          measured();
        }}
        onScroll={(e) => {
          extent.current.offset = e.nativeEvent.contentOffset.y;
          measured();
        }}
      >
        <Text style={[styles.commandText, { color: p.label }]}>{text}</Text>
      </ScrollView>
      {fade && edges.above ? <LinearGradient pointerEvents="none" colors={[fade[0], fade[1]]} style={[styles.outputFade, { top: 0 }]} /> : null}
      {fade && edges.below ? <LinearGradient pointerEvents="none" colors={[fade[1], fade[0]]} style={[styles.outputFade, { bottom: 0 }]} /> : null}
    </View>
  );
}

/// The whole command a permission card asks about or a command's card runs, to read or copy.
export const StatusRow = memo(function StatusRow({ text }: { text: string }) {
  const p = usePalette();
  return (
    <View style={[styles.centered, { paddingTop: 10, paddingBottom: 4 }]}>
      <Text style={[styles.caption, { color: p.tertiaryLabel }]}>{text}</Text>
    </View>
  );
});

/// "Working…" in a DM and "Chef is working…" in a group, or what the one bot at work is doing,
/// the words shimmering while a turn runs.
export const WorkingRow = memo(function WorkingRow({ chatId, bots, isGroup }: { chatId: string; bots: Bot[]; isGroup: boolean }) {
  useLanguage();
  const p = usePalette();
  const activity = useStore((s) => workingActivity(s, chatId));
  const names = bots.map((b) => b.name);
  const label =
    names.length > 1
      ? t("{names} and {last} are working…", { names: names.slice(0, -1).join(language === "zh" ? "、" : ", "), last: names[names.length - 1] })
      : (activity ?? (isGroup && names.length === 1 ? t("{name} is working…", { name: names[0] }) : t("Working…")));
  return (
    <View style={[styles.messageRow, styles.messageRowBot, { paddingTop: 14, alignItems: "center" }]}>
      <View style={{ width: AVATAR + GUTTER }}>{bots[0] && <BotAvatar bot={bots[0]} size={AVATAR} />}</View>
      <ShimmerView style={styles.working}>
        <Text style={[styles.caption, { color: p.label }]} numberOfLines={1}>
          {label}
        </Text>
      </ShimmerView>
    </View>
  );
});

const styles = StyleSheet.create({
  dayRow: { alignItems: "center", paddingTop: 18, paddingBottom: 2 },
  caption: { fontSize: Font.caption, fontWeight: "500" },
  messageRow: { flexDirection: "row", paddingHorizontal: INSET },
  messageRowYou: { justifyContent: "flex-end" },
  messageRowBot: { justifyContent: "flex-start" },
  bubbleColumn: { maxWidth: "80%", alignItems: "flex-start" },
  bubbleColumnYou: { alignItems: "flex-end" },
  author: { fontSize: Font.author, fontWeight: "600", marginLeft: 12, marginBottom: 2 },
  quote: { flexDirection: "row", alignItems: "center", gap: 4, maxWidth: "100%", marginHorizontal: 6, marginBottom: 3 },
  quoteYou: { alignSelf: "flex-end" },
  quoteText: { flexShrink: 1, fontSize: 12 },
  quoteName: { fontWeight: "600" },
  replyArrow: { position: "absolute", left: 18, top: 0, bottom: 0, justifyContent: "center" },
  // Held for the bot's next step: the bubble waits, dimmed, over Send now.
  held: { opacity: 0.55 },
  sendNow: { alignSelf: "flex-end", marginTop: 4, marginRight: 6 },
  sendNowText: { fontSize: 13, fontWeight: "600" },
  bubble: { borderRadius: 18, paddingHorizontal: 13, paddingVertical: 9, gap: 2 },
  bubbleFooter: { flexDirection: "row", justifyContent: "flex-end", alignItems: "center" },
  centered: { alignItems: "center", paddingHorizontal: INSET },
  markerLine: { flexDirection: "row", alignItems: "center" },
  notice: { flexDirection: "row", alignItems: "flex-start", gap: 8, borderRadius: 10, paddingHorizontal: 10, paddingVertical: 8, maxWidth: 360 },
  // The card takes what the row leaves beside the avatar column, up to `maxWidth`.
  permission: { flex: 1, borderRadius: 14, borderWidth: StyleSheet.hairlineWidth, paddingHorizontal: 12, paddingVertical: 10, gap: 6, maxWidth: 420 },
  permissionTitle: { fontSize: 14, fontWeight: "600", flexShrink: 1 },
  permissionButton: { paddingHorizontal: 12, paddingVertical: 7, borderRadius: 8 },
  headerButton: { paddingHorizontal: 10, paddingVertical: 4, borderRadius: 7 },
  command: { borderRadius: 8, paddingHorizontal: 10, paddingVertical: 7, marginTop: 2 },
  commandText: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 12.5, lineHeight: 17 },
  output: { borderRadius: 8, marginTop: 2, overflow: "hidden" },
  // Six lines of `commandText`, then the block scrolls.
  outputScroll: { maxHeight: 6 * 17 + 14 },
  outputContent: { paddingHorizontal: 10, paddingVertical: 7 },
  outputFade: { position: "absolute", left: 0, right: 0, height: 16 },
  reasonText: { fontSize: 13, lineHeight: 18 },
  ruleNote: { fontSize: 12, lineHeight: 16 },
  noticeIcon: { marginTop: (NOTICE_LINE - NOTICE_ICON) / 2 },
  noticeText: { fontSize: 12.5, lineHeight: NOTICE_LINE, flexShrink: 1 },
  working: { flexShrink: 1 },
});
