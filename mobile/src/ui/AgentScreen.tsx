// A coding agent's page, after Cursor's agent transcript and view-only like Grok Bot's exchange
// view: its mark and title, its name, how it stands, where it works and on which Runner, its pull
// request and Stop; what it asks, as in the chat; then its steps in order (the prompt, Lorca's own
// lines, thoughts, what it said, the tools it used with reads and searches in a row folded into one
// line, its plan, and errors), and what its work changed. It follows the agent while it is open,
// asking the Runner again every five seconds, and keeps to the end unless the user scrolled up to
// read; one that asks opens at its question instead. Follow-ups go through the bot, so there is no composer.

import { Stack, useFocusEffect, useRouter } from "expo-router";
import { useCallback, useRef, useState, type ReactNode } from "react";
import { ActivityIndicator, Platform, ScrollView, StyleSheet, Text, useWindowDimensions, View, type ColorValue } from "react-native";
import { engine } from "../core/engine";
import { agentIsOpen, agentIsRunning, agentName, agentTitle, pullRequestLabel, type AgentItem, type AgentRun, type AgentToolStep } from "../core/model";
import { useBotMap, useChat } from "../core/store";
import { t, useLanguage } from "../i18n";
import { AgentLogo } from "./AgentLogo";
import { alert } from "./alert";
import { agentStatusWord, changedFiles, exploredLine, thoughtLine, toolLine, transcriptBlocks, type TranscriptBlock } from "./codingAgents";
import { Markdown } from "./Markdown";
import { CloseToolbar } from "./navigation";
import { openLink } from "./outputs";
import { Pressable } from "./Pressable";
import { Symbol } from "./Symbol";
import { Font, usePalette } from "./theme";
import { AgentAskCard } from "./transcript";

/// How often the transcript is asked for again while the agent is open and on screen.
const REFRESH_MS = 5000;
/// The first prompt folds past this many lines.
const PROMPT_LINES = 6;

/// `messageId` is the row that started the agent. `presented` gives a sheet its Done; inside
/// Details the stack's back button leaves.
export function AgentScreen({ chatId, messageId, presented }: { chatId: string; messageId: string; presented: boolean }) {
  useLanguage();
  const router = useRouter();
  const p = usePalette();
  const chat = useChat(chatId);
  const bots = useBotMap();
  const { width } = useWindowDimensions();
  const [paneWidth, setPaneWidth] = useState(width);
  const [items, setItems] = useState<AgentItem[] | null>(null);
  const [fetched, setFetched] = useState<AgentRun | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [stopping, setStopping] = useState(false);
  const scroll = useRef<ScrollView>(null);
  const row = chat?.messages.find((m) => m.id === messageId);
  // The row follows the Runner's events, so its card is the freshest; the transcript's card stands
  // in when the row is not loaded.
  const card = (row?.body.kind === "tool" ? row.body.agent : undefined) ?? fetched;
  const open = card ? agentIsOpen(card) : true;
  const bot = row?.author.kind === "bot" ? bots.get(row.author.bot_id) : undefined;
  // It opens at its latest step, unless it asks: then at its question, on top.
  const atEnd = useRef(card?.state !== "asking");

  useFocusEffect(
    useCallback(() => {
      let live = true;
      let timer: ReturnType<typeof setTimeout> | undefined;
      const load = async () => {
        let again = true;
        try {
          const answer = await engine.agentTranscript(chatId, messageId);
          if (!live) return;
          setItems(answer.items);
          setFetched(answer.card);
          setError(null);
          again = agentIsOpen(answer.card);
        } catch (e) {
          // What was shown stays; the next try may reach the Runner.
          if (live) setError(e instanceof Error ? e.message : String(e));
        }
        if (live && again) timer = setTimeout(load, REFRESH_MS);
      };
      void load();
      return () => {
        live = false;
        if (timer) clearTimeout(timer);
      };
      // An agent that ended and was opened again (a follow-up) is asked for again.
    }, [chatId, messageId, open]),
  );

  const stop = async () => {
    setStopping(true);
    try {
      await engine.stopAgent(chatId, messageId);
    } catch (e) {
      alert(t("Couldn't stop the agent"), e instanceof Error ? e.message : String(e));
    } finally {
      setStopping(false);
    }
  };

  const decide = useCallback((message: NonNullable<typeof row>, decision: "allow" | "always" | "deny") => {
    engine.answerPermission(message.chat_id, message.id, decision).catch((e) => alert(t("Could not answer"), e instanceof Error ? e.message : String(e)));
  }, []);

  const where = card ? [agentName(card.kind), agentStatusWord(card), card.branch, card.device].filter(Boolean).join(" · ") : "";
  const blocks = items ? transcriptBlocks(items) : [];
  const textWidth = paneWidth - 32;

  return (
    <View style={styles.screen} onLayout={(e) => setPaneWidth(e.nativeEvent.layout.width)}>
      <Stack.Screen options={{ title: "" }} />
      {presented ? <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.dismiss()} /> : null}
      <ScrollView
        ref={scroll}
        contentContainerStyle={styles.content}
        contentInsetAdjustmentBehavior="automatic"
        scrollEventThrottle={64}
        onScroll={(e) => {
          const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
          atEnd.current = contentOffset.y + layoutMeasurement.height >= contentSize.height - 24;
        }}
        onContentSizeChange={() => {
          if (atEnd.current && items?.length) scroll.current?.scrollToEnd({ animated: false });
        }}
      >
        {card ? (
          <View style={styles.header}>
            <View style={styles.titleLine}>
              <View style={styles.logo}>
                <AgentLogo kind={card.kind} size={20} color={p.label} />
              </View>
              <Text style={[styles.title, { color: p.label }]} numberOfLines={3} selectable>
                {agentTitle(card)}
              </Text>
            </View>
            <Text style={[styles.where, { color: p.secondaryLabel }]} numberOfLines={2}>
              {where}
            </Text>
            {card.pull_request || agentIsRunning(card) ? (
              <View style={styles.actions}>
                {card.pull_request ? (
                  <Pressable
                    onPress={() => openLink(card.pull_request!)}
                    style={({ pressed }) => [styles.action, { backgroundColor: pressed ? p.separator : p.fill }]}
                    accessibilityRole="link"
                    accessibilityLabel={t("Pull request {number}", { number: pullRequestLabel(card.pull_request) })}
                  >
                    <Symbol name="arrow.triangle.pull" size={13} color={p.tint} />
                    <Text style={[styles.actionText, { color: p.tint }]}>{pullRequestLabel(card.pull_request)}</Text>
                  </Pressable>
                ) : null}
                {agentIsRunning(card) ? (
                  <Pressable
                    disabled={stopping}
                    onPress={() => void stop()}
                    style={({ pressed }) => [styles.action, { backgroundColor: pressed ? p.separator : p.fill, opacity: stopping ? 0.5 : 1 }]}
                    accessibilityRole="button"
                  >
                    <Text style={[styles.actionText, { color: p.label }]}>{t("Stop")}</Text>
                  </Pressable>
                ) : null}
              </View>
            ) : null}
          </View>
        ) : null}

        {row && card?.state === "asking" ? (
          <View style={styles.ask}>
            <AgentAskCard message={row} agent={card} bot={bot} isGroup={false} onDecide={decide} />
          </View>
        ) : null}

        {blocks.map((block) => (
          <Block key={block.kind === "item" ? block.item.id : `explored-${block.id}`} block={block} textWidth={textWidth} />
        ))}

        {items === null || items.length === 0 ? (
          <Text style={[styles.status, { color: p.secondaryLabel }]}>{error ?? (items === null ? t("Loading…") : t("Nothing yet"))}</Text>
        ) : null}

        {card?.changes ? (
          <View style={[styles.footer, { borderTopColor: p.separator }]}>
            <Text style={[styles.caption, { color: p.secondaryLabel }]}>
              <Text style={{ color: p.green }}>+{card.changes.added}</Text> <Text style={{ color: p.red }}>−{card.changes.removed}</Text>
              {changedFiles(card.changes.files)}
              {card.branch ? ` · ${card.branch}` : ""}
            </Text>
          </View>
        ) : null}
      </ScrollView>
    </View>
  );
}

function Block({ block, textWidth }: { block: TranscriptBlock; textWidth: number }) {
  if (block.kind === "explored") return <Explored steps={block.steps} />;
  const item = block.item;
  switch (item.kind) {
    case "prompt":
      return <Prompt text={item.text ?? ""} />;
    case "status":
      return <Status text={item.text ?? ""} />;
    case "thought":
      return <Thought text={item.text ?? ""} seconds={item.seconds} />;
    case "text":
      return <Said text={item.text ?? ""} width={textWidth} />;
    case "tool":
      return item.tool ? <Tool step={item.tool} /> : null;
    case "plan":
      return <Plan entries={item.plan ?? []} />;
    case "error":
      return <Failure text={item.text ?? ""} />;
  }
}

/// What the agent was sent, in a card under a small "Prompt" label: the first brief folds past six
/// lines behind Show more.
function Prompt({ text }: { text: string }) {
  const p = usePalette();
  const [whole, setWhole] = useState(false);
  const long = text.split("\n").length > PROMPT_LINES || text.length > 420;
  return (
    <View style={[styles.prompt, { backgroundColor: p.cell, borderColor: p.separator }]}>
      <Text style={[styles.promptLabel, { color: p.secondaryLabel }]}>{t("Prompt")}</Text>
      <Text style={[styles.body, { color: p.label }]} numberOfLines={whole || !long ? undefined : PROMPT_LINES} selectable>
        {text}
      </Text>
      {long ? (
        <Pressable onPress={() => setWhole(!whole)} hitSlop={8} accessibilityRole="button">
          <Text style={[styles.more, { color: p.tint }]}>{whole ? t("Show less") : t("Show more")}</Text>
        </Pressable>
      ) : null}
    </View>
  );
}

function Status({ text }: { text: string }) {
  const p = usePalette();
  return <Text style={[styles.status, { color: p.secondaryLabel }]}>{text}</Text>;
}

/// A line that opens to show more under it: a thought, a run of reads and searches, a command's
/// output.
function Disclosure({ line, tint, busy, children }: { line: ReactNode; tint?: ColorValue; busy?: boolean; children?: ReactNode }) {
  const p = usePalette();
  const [opened, setOpened] = useState(false);
  const opens = !!children;
  return (
    <View style={styles.step}>
      <Pressable onPress={opens ? () => setOpened(!opened) : undefined} disabled={!opens} style={styles.stepLine} accessibilityRole={opens ? "button" : undefined} accessibilityState={opens ? { expanded: opened } : undefined}>
        {busy ? <ActivityIndicator size="small" style={styles.spinner} /> : null}
        <Text style={[styles.stepText, { color: tint ?? p.secondaryLabel }]} numberOfLines={opened ? undefined : 2}>
          {line}
        </Text>
        {opens ? <Symbol name={opened ? "chevron.down" : "chevron.right"} size={10} color={p.tertiaryLabel} weight="semibold" /> : null}
      </Pressable>
      {opened ? children : null}
    </View>
  );
}

function Thought({ text, seconds }: { text: string; seconds?: number }) {
  const p = usePalette();
  return (
    <Disclosure line={thoughtLine(seconds)}>
      {text ? (
        <Text style={[styles.thought, { color: p.secondaryLabel, borderLeftColor: p.separator }]} selectable>
          {text}
        </Text>
      ) : undefined}
    </Disclosure>
  );
}

/// The agent's own words, as a bot's message reads.
function Said({ text, width }: { text: string; width: number }) {
  const p = usePalette();
  return (
    <View style={styles.said}>
      <Markdown text={text} color={p.label} maxWidth={width} />
    </View>
  );
}

function Explored({ steps }: { steps: AgentToolStep[] }) {
  const p = usePalette();
  const running = steps.some((step) => step.status === "running");
  return (
    <Disclosure line={exploredLine(steps, running)} busy={running}>
      <View style={styles.list}>
        {steps.map((step, index) => (
          <Text key={index} style={[styles.listText, { color: step.status === "failed" ? p.red : p.secondaryLabel }]} numberOfLines={1}>
            {toolLine(step)}
          </Text>
        ))}
      </View>
    </Disclosure>
  );
}

/// One tool step on its line: what it did, an edit's counts in green and red, a running step with a
/// spinner, a failed one in red. A command with output opens to show it.
function Tool({ step }: { step: AgentToolStep }) {
  const p = usePalette();
  const tint = step.status === "failed" || step.status === "denied" ? p.red : undefined;
  const counts =
    step.kind === "edit" && step.status !== "denied" && (step.added || step.removed) ? (
      <>
        {" "}
        <Text style={{ color: p.green }}>+{step.added ?? 0}</Text> <Text style={{ color: p.red }}>−{step.removed ?? 0}</Text>
      </>
    ) : null;
  const output = step.kind === "execute" ? step.output?.trimEnd() : undefined;
  return (
    <Disclosure
      line={
        <>
          {toolLine(step)}
          {counts}
        </>
      }
      tint={tint}
      busy={step.status === "running"}
    >
      {output ? (
        <View style={[styles.output, { backgroundColor: p.code }]}>
          <Text style={[styles.mono, { color: p.label }]} selectable>
            {output}
          </Text>
        </View>
      ) : undefined}
    </Disclosure>
  );
}

function Plan({ entries }: { entries: NonNullable<AgentItem["plan"]> }) {
  const p = usePalette();
  return (
    <View style={[styles.plan, { backgroundColor: p.cell, borderColor: p.separator }]}>
      {entries.map((entry, index) => (
        <View key={index} style={styles.planEntry}>
          <Symbol
            name={entry.status === "completed" ? "checkmark.circle.fill" : entry.status === "in_progress" ? "circle.dotted.circle" : "circle"}
            size={15}
            color={entry.status === "completed" ? p.green : entry.status === "in_progress" ? p.tint : p.tertiaryLabel}
          />
          <Text style={[styles.planText, { color: entry.status === "completed" ? p.secondaryLabel : p.label }]}>{entry.text}</Text>
        </View>
      ))}
    </View>
  );
}

function Failure({ text }: { text: string }) {
  const p = usePalette();
  return (
    <Text style={[styles.body, styles.failure, { color: p.red }]} selectable>
      {text}
    </Text>
  );
}

const MONO = Platform.OS === "ios" ? "Menlo" : "monospace";

const styles = StyleSheet.create({
  screen: { flex: 1 },
  content: { paddingHorizontal: 16, paddingTop: 8, paddingBottom: 40, gap: 6 },
  header: { gap: 6, paddingBottom: 10 },
  titleLine: { flexDirection: "row", alignItems: "flex-start", gap: 10 },
  logo: { paddingTop: 2 },
  title: { flex: 1, fontSize: 20, fontWeight: "700", lineHeight: 25 },
  where: { fontSize: Font.small },
  actions: { flexDirection: "row", gap: 8, marginTop: 4 },
  action: { flexDirection: "row", alignItems: "center", gap: 5, paddingHorizontal: 12, paddingVertical: 6, borderRadius: 8 },
  actionText: { fontSize: 13, fontWeight: "600" },
  ask: { marginHorizontal: -12, paddingBottom: 6 },
  prompt: { borderRadius: 12, borderWidth: StyleSheet.hairlineWidth, padding: 12, gap: 6, marginVertical: 4 },
  promptLabel: { fontSize: 12, fontWeight: "600" },
  body: { fontSize: 15, lineHeight: 21 },
  more: { fontSize: 13, fontWeight: "600" },
  status: { fontSize: 13, lineHeight: 18, paddingVertical: 4 },
  step: { paddingVertical: 3 },
  stepLine: { flexDirection: "row", alignItems: "center", gap: 6, minHeight: 24 },
  stepText: { flexShrink: 1, fontSize: 14, lineHeight: 19 },
  spinner: { transform: [{ scale: 0.7 }], width: 16, height: 16 },
  thought: { fontSize: 13.5, lineHeight: 19, marginTop: 4, paddingLeft: 10, borderLeftWidth: 2 },
  said: { paddingVertical: 4 },
  list: { gap: 3, marginTop: 4, paddingLeft: 10 },
  listText: { fontSize: 13, lineHeight: 18 },
  output: { borderRadius: 8, padding: 10, marginTop: 4 },
  mono: { fontFamily: MONO, fontSize: 12, lineHeight: 17 },
  plan: { borderRadius: 12, borderWidth: StyleSheet.hairlineWidth, padding: 12, gap: 8, marginVertical: 4 },
  planEntry: { flexDirection: "row", alignItems: "flex-start", gap: 8 },
  planText: { flex: 1, fontSize: 14, lineHeight: 19 },
  failure: { paddingVertical: 4 },
  footer: { borderTopWidth: StyleSheet.hairlineWidth, marginTop: 10, paddingTop: 10 },
  caption: { fontSize: 13 },
});
