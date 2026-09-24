// Transcript rows, after the Mac app: bubbles (a group shows the bot's name above and its
// avatar beside the bubble's bottom edge; a DM shows neither), "Today 4:13 AM" separators after
// fifteen minutes of silence, the "is working" row, "Chef stopped without replying", and the
// centered "Message from ◉ Name" / "Messaged ◉ Name" markers. Tool calls never render.

import { useEffect, useState } from "react";
import { Linking, Modal, Platform, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { ShimmerView } from "../../modules/lorca-core/ShimmerView";
import { isSentMessage, type Body, type Bot, type Chat, type Message } from "../core/model";
import { useStore } from "../core/store";
import { language, t } from "../i18n";
import { AttachmentBlock } from "./attachments";
import { BotAvatar } from "./Avatar";
import { daySeparator, firstLine, workingActivity } from "./format";
import { Markdown } from "./Markdown";
import { Symbol } from "./Symbol";
import { usePaneWidth } from "./layout";
import { Font, usePalette } from "./theme";

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
        rows.push({ key: message.id, type: "marker", text: t("Messaged"), bot: bots.get(message.body.target_bot_id ?? ""), tooltip: message.body.detail, groupStart });
        previousAuthorKey = null;
        break;
      case "handoff": {
        const incoming = chat.kind !== "group" && chat.bot_ids.includes(message.body.to);
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

export function DayRow({ at }: { at: number }) {
  const p = usePalette();
  return (
    <View style={styles.dayRow}>
      <Text style={[styles.caption, { color: p.secondaryLabel }]}>{daySeparator(new Date(at * 1000))}</Text>
    </View>
  );
}

export function MessageRow({ row, bots, isGroup }: { row: Extract<Row, { type: "message" }>; bots: Map<string, Bot>; isGroup: boolean }) {
  const p = usePalette();
  const paneWidth = usePaneWidth();
  const { message, groupStart, groupEnd, showsName } = row;
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
  return (
    <View style={[styles.messageRow, { paddingTop: groupStart ? 14 : 3 }, isYou ? styles.messageRowYou : styles.messageRowBot]}>
      {showsAvatar && <View style={{ width: AVATAR + GUTTER, alignSelf: "flex-end" }}>{groupEnd && <BotAvatar bot={bot} size={AVATAR} />}</View>}
      <View style={[styles.bubbleColumn, { maxWidth: columnWidth }, isYou && styles.bubbleColumnYou]}>
        {showsName && (
          <Text style={[styles.author, { color: p.secondaryLabel }]} numberOfLines={1}>
            {bot?.name ?? t("Bot")}
          </Text>
        )}
        <View
          style={[
            styles.bubble,
            isYou ? { backgroundColor: p.userBubble, borderBottomRightRadius: groupEnd ? 6 : 18 } : { backgroundColor: failed ? "rgba(255,59,48,0.14)" : p.botBubble, borderBottomLeftRadius: groupEnd ? 6 : 18 },
          ]}
        >
          {attachments.length > 0 && <AttachmentBlock attachments={attachments} onUserBubble={isYou} maxWidth={attachmentWidth} />}
          {text.length > 0 && <Markdown text={text} color={isYou ? p.userBubbleText : p.botBubbleText} maxWidth={attachmentWidth} />}
          {failed && (
            <View style={styles.bubbleFooter}>
              <Symbol name="exclamationmark.triangle.fill" size={11} color={p.red} />
            </View>
          )}
        </View>
      </View>
    </View>
  );
}

/// "Messaged ◉ Name" with the message's first line under it; a tap opens the whole message
/// in a sheet.
export function MarkerRow({ row, onPress }: { row: Extract<Row, { type: "marker" }>; onPress?: (row: Extract<Row, { type: "marker" }>) => void }) {
  const p = usePalette();
  const preview = row.tooltip ? firstLine(row.tooltip) : "";
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
}

export function NoticeRow({ row }: { row: Extract<Row, { type: "notice" }> }) {
  const p = usePalette();
  return (
    <View style={[styles.centered, { paddingTop: row.groupStart ? 14 : 6 }]}>
      <View style={[styles.notice, { backgroundColor: p.fill }]}>
        <Symbol name="info.circle.fill" size={NOTICE_ICON} color={p.secondaryLabel} style={styles.noticeIcon} />
        <Text style={[styles.noticeText, { color: p.secondaryLabel }]}>{row.text}</Text>
      </View>
    </View>
  );
}

/// A bot asking before a plugin tool runs, a shell command runs, or a plugin is installed. While
/// it waits: the question, the call (a shell command in a code block that opens the whole
/// command on tap), why Auto-review paused it, the answers, and under them the rule Always allow
/// adds. A shell command offers Always allow only with a rule. Once answered, the answer and the
/// call; an Always allow keeps its rule.
export function PermissionRow({ row, onDecide }: { row: Extract<Row, { type: "permission" }>; onDecide: (decision: "allow" | "always" | "deny") => void }) {
  const p = usePalette();
  const [copied, setCopied] = useState(false);
  const [showCommand, setShowCommand] = useState(false);
  const pending = row.body.decision === "pending";
  const connect = row.body.tool === "connect";
  const shell = row.body.plugin_id === "computer";
  const who = row.bot?.name ?? t("The bot");
  const plugin = row.body.plugin_name;
  const title = connect
    ? t("{who} needs a sign-in to {plugin}", { who, plugin })
    : row.body.tool === "install"
      ? t("{who} wants to install {plugin}", { who, plugin })
      : shell
        ? t("{who} wants to run a command on {plugin}", { who, plugin })
        : t("{who} wants to use {plugin}", { who, plugin });
  const command = row.body.command ?? row.body.summary.replace(/^\$ /, "");
  const ruleNote = !row.body.rule
    ? undefined
    : pending
      ? t("Always allow adds the rule “{rule}”.", { rule: row.body.rule })
      : row.body.decision === "always"
        ? t("Added the rule “{rule}” to Auto-review.", { rule: row.body.rule })
        : undefined;
  const decided: Record<string, string> = connect
    ? { allowed: t("Signing in"), denied: t("Not now"), connected: t("Signed in"), failed: t("Sign-in failed") }
    : { allowed: t("Allowed once"), always: t("Always allowed"), denied: t("Denied"), expired: t("No answer in time") };
  const choices: [string, "allow" | "always" | "deny"][] = connect
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
    <View style={{ paddingTop: row.groupStart ? 14 : 6, paddingHorizontal: INSET }}>
      <View style={[styles.permission, { backgroundColor: p.cell, borderColor: p.separator }]}>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <Symbol name={connect ? "person.crop.circle.badge.checkmark" : row.body.tool === "install" ? "puzzlepiece.extension" : "hand.raised"} size={16} color={row.body.decision === "failed" ? p.red : row.body.decision === "connected" ? p.green : p.tint} />
          <Text style={[styles.permissionTitle, { color: p.label }]} numberOfLines={2}>
            {title}
          </Text>
        </View>
        {pending && shell ? (
          <Pressable
            onPress={() => setShowCommand(true)}
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
            {pending ? row.body.summary : `${decided[row.body.decision] ?? row.body.decision} · ${row.body.summary}`}
          </Text>
        )}
        {pending && row.body.reason ? (
          <Text style={[styles.reasonText, { color: p.secondaryLabel }]}>{row.body.reason}</Text>
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
              <Pressable key={decision} onPress={() => onDecide(decision)} style={({ pressed }) => [styles.permissionButton, { backgroundColor: pressed ? p.separator : p.fill }]}>
                <Text style={{ color: decision === "deny" ? p.label : p.tint, fontSize: 13, fontWeight: "600" }}>
                  {label}
                </Text>
              </Pressable>
            ))}
          </View>
        ) : null}
        {ruleNote ? <Text style={[styles.ruleNote, { color: p.secondaryLabel }]}>{ruleNote}</Text> : null}
      </View>
      {shell ? <CommandSheet visible={showCommand} title={title} command={command} onClose={() => setShowCommand(false)} /> : null}
    </View>
  );
}

/// The whole command a permission card asks about, to read or copy before answering.
function CommandSheet({ visible, title, command, onClose }: { visible: boolean; title: string; command: string; onClose: () => void }) {
  const p = usePalette();
  const [copied, setCopied] = useState(false);
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), 1500);
    return () => clearTimeout(timer);
  }, [copied]);
  return (
    <Modal visible={visible} animationType="slide" presentationStyle="pageSheet" onRequestClose={onClose}>
      <View style={[styles.sheet, { backgroundColor: p.groupedBackground }]}>
        <Text style={[styles.sheetTitle, { color: p.label }]}>{title}</Text>
        <ScrollView style={{ flex: 1 }} contentContainerStyle={[styles.sheetCommand, { backgroundColor: p.code }]}>
          <Text selectable style={[styles.commandText, { color: p.label }]}>{command}</Text>
        </ScrollView>
        <View style={styles.sheetButtons}>
          <Pressable
            onPress={async () => {
              await Clipboard.setStringAsync(command);
              setCopied(true);
            }}
            style={({ pressed }) => [styles.permissionButton, { backgroundColor: pressed ? p.separator : p.fill }]}
          >
            <Text style={{ color: copied ? p.green : p.tint, fontSize: 15, fontWeight: "600" }}>{copied ? t("Copied") : t("Copy")}</Text>
          </Pressable>
          <Pressable onPress={onClose} style={({ pressed }) => [styles.permissionButton, { backgroundColor: pressed ? p.separator : p.fill }]}>
            <Text style={{ color: p.tint, fontSize: 15, fontWeight: "600" }}>{t("Done")}</Text>
          </Pressable>
        </View>
      </View>
    </Modal>
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

/// "Working…" in a DM and "Chef is working…" in a group, or what the one bot at work is doing,
/// the words shimmering while a turn runs.
export function WorkingRow({ chatId, bots, isGroup }: { chatId: string; bots: Bot[]; isGroup: boolean }) {
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
  bubbleFooter: { flexDirection: "row", justifyContent: "flex-end", alignItems: "center" },
  centered: { alignItems: "center", paddingHorizontal: INSET },
  markerLine: { flexDirection: "row", alignItems: "center" },
  notice: { flexDirection: "row", alignItems: "flex-start", gap: 8, borderRadius: 10, paddingHorizontal: 10, paddingVertical: 8, maxWidth: 360 },
  permission: { borderRadius: 14, borderWidth: StyleSheet.hairlineWidth, paddingHorizontal: 12, paddingVertical: 10, gap: 6, maxWidth: 420 },
  permissionTitle: { fontSize: 14, fontWeight: "600", flexShrink: 1 },
  permissionButton: { paddingHorizontal: 12, paddingVertical: 7, borderRadius: 8 },
  command: { borderRadius: 8, paddingHorizontal: 10, paddingVertical: 7, marginTop: 2 },
  commandText: { fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace", fontSize: 12.5, lineHeight: 17 },
  reasonText: { fontSize: 13, lineHeight: 18 },
  ruleNote: { fontSize: 12, lineHeight: 16 },
  sheet: { flex: 1, paddingHorizontal: 20, paddingTop: 20, gap: 14 },
  sheetTitle: { fontSize: 17, fontWeight: "600" },
  sheetCommand: { borderRadius: 10, padding: 12 },
  sheetButtons: { flexDirection: "row", justifyContent: "space-between", paddingBottom: 12 },
  noticeIcon: { marginTop: (NOTICE_LINE - NOTICE_ICON) / 2 },
  noticeText: { fontSize: 12.5, lineHeight: NOTICE_LINE, flexShrink: 1 },
  working: { flexShrink: 1 },
});
