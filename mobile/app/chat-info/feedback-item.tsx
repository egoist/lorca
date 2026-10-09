// One suggestion or one change, after the desktop apps' sheet for either: why, the change as the
// lines it takes out and puts in, and for a suggestion the notes it is based on. Accept, the
// toolbar's button, applies a suggestion on the bot's Runner; Reject sits apart at the bottom. A
// change can be undone while the routine or skill still reads as the change left it.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { decideSuggestion, undoChange, useFeedback } from "../../src/core/feedback";
import { useRoutines } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { alert } from "../../src/ui/alert";
import { diffLines, feedbackProblem, kindSymbol, kindTitle, noteTitle, targetName, type FeedbackTarget } from "../../src/ui/feedback";
import { Row, Section } from "../../src/ui/forms";
import { daySeparator, stamp } from "../../src/ui/format";
import { SaveToolbar } from "../../src/ui/navigation";
import { usePalette } from "../../src/ui/theme";

const kindWord = (target: FeedbackTarget) => (target.kind === "playbook" ? t("Skill") : t("Task"));

/// The lines a change takes out on red and puts in on green, as tracked changes read.
function Diff({ diff }: { diff: string }) {
  const p = usePalette();
  return (
    <View>
      {diffLines(diff).map((line, index) => (
        <View key={index} style={[styles.line, { backgroundColor: line.removed ? "rgba(255, 59, 48, 0.14)" : "rgba(52, 199, 89, 0.16)" }]}>
          <Text style={[styles.mark, { color: line.removed ? p.red : p.green }]}>{line.removed ? "−" : "+"}</Text>
          <Text style={[styles.text, { color: p.label }]}>{line.text || " "}</Text>
        </View>
      ))}
    </View>
  );
}

export default function FeedbackItemScreen() {
  useLanguage();
  const { bot, suggestion: suggestionId, change: changeId } = useLocalSearchParams<{ bot: string; suggestion?: string; change?: string }>();
  const router = useRouter();
  const p = usePalette();
  const routines = useRoutines(bot);
  const feedback = useFeedback(bot);
  // What the screen opened on, kept while a decision is on its way and the list changes.
  const [suggestion] = useState(() => feedback?.suggestions.find((s) => s.id === suggestionId));
  const [change] = useState(() => feedback?.changes.find((c) => c.id === changeId));
  const [busy, setBusy] = useState(false);
  if (!feedback || !(suggestion || change)) return null;
  const f = feedback;
  const target = (suggestion ?? change)!.target;
  const name = targetName(f, target, routines);

  async function run(failure: string, request: () => Promise<unknown>) {
    setBusy(true);
    try {
      await request();
      router.back();
    } catch (error) {
      setBusy(false);
      alert(failure, feedbackProblem(error instanceof Error ? error.message : String(error), name));
    }
  }

  if (suggestion) {
    const notes = suggestion.evidence.map((id) => f.notes.find((note) => note.id === id)).filter((note) => !!note);
    return (
      <>
        <Stack.Screen options={{ title: name }} />
        <SaveToolbar label={t("Accept")} disabled={busy} onSave={() => void run(t("Couldn't make the change"), () => decideSuggestion(bot, suggestion, true))} />
        <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
          <View style={styles.header}>
            <Text style={[styles.kicker, { color: p.secondaryLabel }]}>{t("Suggested change")}</Text>
            <Text style={[styles.explanation, { color: p.label }]}>{suggestion.explanation}</Text>
          </View>
          <Section title={kindWord(target)}>
            <Diff diff={suggestion.diff} />
          </Section>
          {notes.length > 0 ? (
            <Section title={t("Based on")}>
              {notes.map((note) => (
                <Row key={note.id} title={noteTitle(f, note, routines)} subtitle={`${kindTitle(note.kind)} · ${stamp(new Date(note.createdAt * 1000))}`} icon={kindSymbol(note.kind)} />
              ))}
            </Section>
          ) : null}
          <Section>
            <Row title={t("Reject")} icon="xmark.circle" destructive onPress={busy ? undefined : () => void run(t("Couldn't reject it"), () => decideSuggestion(bot, suggestion, false))} />
          </Section>
        </ScrollView>
      </>
    );
  }

  const when = daySeparator(new Date(change!.createdAt * 1000));
  return (
    <>
      <Stack.Screen options={{ title: name }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <Section
          title={kindWord(target)}
          footer={`${change!.isUndo ? t("Undone {date}", { date: when }) : t("Changed {date}", { date: when })}${change!.canUndo ? "" : `\n${t("“{name}” has changed since, so this can't be undone.", { name })}`}`}
        >
          <Diff diff={change!.diff} />
        </Section>
        {change!.canUndo ? (
          <Section>
            <Row title={t("Undo Change")} icon="arrow.uturn.backward" onPress={busy ? undefined : () => void run(t("Couldn't undo the change"), () => undoChange(bot, change!))} />
          </Section>
        ) : null}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  header: { paddingHorizontal: 20, paddingTop: 8, gap: 4 },
  kicker: { fontSize: 15 },
  explanation: { fontSize: 17, lineHeight: 22 },
  line: { flexDirection: "row", gap: 10, paddingVertical: 8, paddingHorizontal: 16 },
  mark: { width: 12, fontSize: 15, lineHeight: 21, fontWeight: "600" },
  text: { flex: 1, fontSize: 15, lineHeight: 21 },
});
