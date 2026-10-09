// Everything a bot keeps from the user's feedback, after the desktop apps' feedback sheet: the
// changes it suggests, how often it looks for them, the changes accepted, and the notes. A
// suggestion or a change slides in its own screen; a note's actions are a tap away.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { ScrollView } from "react-native";
import { chatTitle } from "../../../src/core/engine";
import { excludeNote, loadFeedback, setReviewEvery, suggestChanges, useFeedback } from "../../../src/core/feedback";
import { useBotMap, useRoutines, useStore } from "../../../src/core/store";
import { t, tc, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { kindSymbol, kindTitle, noteTitle, targetName, type FeedbackNote } from "../../../src/ui/feedback";
import { Row, Section } from "../../../src/ui/forms";
import { stamp } from "../../../src/ui/format";

const intervals: { seconds?: number; title: () => string }[] = [
  { title: () => t("When asked") },
  { seconds: 86_400, title: () => t("Daily") },
  { seconds: 604_800, title: () => t("Weekly") },
];

export default function FeedbackScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const bot = useBotMap().get(id);
  const routines = useRoutines(id);
  const chats = useStore((s) => s.chats);
  const feedback = useFeedback(id);
  const [looking, setLooking] = useState(false);
  const [foundNothing, setFoundNothing] = useState(false);

  useEffect(() => {
    void loadFeedback(id);
  }, [id]);

  if (!bot || !feedback) return <Stack.Screen options={{ title: t("Feedback") }} />;
  const f = feedback;
  const interval = intervals.find((each) => each.seconds === f.reviewEvery) ?? intervals[2];

  async function act(failure: string, run: () => Promise<unknown>) {
    try {
      await run();
    } catch (error) {
      alert(failure, error instanceof Error ? error.message : String(error));
    }
  }

  async function lookNow() {
    setLooking(true);
    setFoundNothing(false);
    try {
      setFoundNothing((await suggestChanges(id)) === 0);
    } catch (error) {
      alert(t("Couldn't look for changes"), error instanceof Error ? error.message : String(error));
    }
    setLooking(false);
  }

  /// Show in Chat, and the exclusions: the note's words, or everything from its chat.
  function showNote(note: FeedbackNote) {
    const chat = chats.find((each) => each.id === note.chatId);
    alert(noteTitle(f, note, routines), `${kindTitle(note.kind)} · ${stamp(new Date(note.createdAt * 1000))}`, [
      { text: t("Show in Chat"), onPress: () => router.dismissTo(`/chat/${note.chatId}`) },
      { text: t("Exclude"), style: "destructive", onPress: () => void act(t("Couldn't exclude it"), () => excludeNote(id, note, false)) },
      ...(chat ? [{ text: t("Exclude Everything from “{name}”", { name: chatTitle(chat) }), style: "destructive" as const, onPress: () => void act(t("Couldn't exclude it"), () => excludeNote(id, note, true)) }] : []),
      { text: t("Cancel"), style: "cancel" },
    ]);
  }

  return (
    <>
      <Stack.Screen options={{ title: t("Feedback") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <Section
          title={t("Suggestions")}
          footer={foundNothing && f.suggestions.length === 0 ? t("Nothing to change right now.") : t("{name} suggests changes to its routines and skills from your feedback. Nothing changes until you accept one.", { name: bot.name })}
        >
          {f.suggestions.map((suggestion) => (
            <Row
              key={suggestion.id}
              title={targetName(f, suggestion.target, routines)}
              subtitle={t("Suggested change")}
              icon="sparkles"
              chevron
              onPress={() => router.push({ pathname: "/chat-info/feedback-item", params: { bot: id, suggestion: suggestion.id } })}
            />
          ))}
          <Row
            title={t("Look for changes")}
            menu={{
              value: interval.title(),
              title: t("Look for changes"),
              choices: intervals.map((each) => ({ title: each.title(), selected: each === interval, onPress: () => void act(t("Couldn't change it"), () => setReviewEvery(id, each.seconds)) })),
            }}
          />
          <Row title={looking ? t("Looking…") : t("Look Now")} action onPress={looking ? undefined : () => void lookNow()} />
        </Section>

        {f.changes.length > 0 ? (
          <Section title={t("Changes")}>
            {f.changes.map((change) => {
              const when = stamp(new Date(change.createdAt * 1000));
              return (
                <Row
                  key={change.id}
                  title={targetName(f, change.target, routines)}
                  subtitle={change.isUndo ? t("Undone {date}", { date: when }) : t("Changed {date}", { date: when })}
                  icon={change.isUndo ? "arrow.uturn.backward" : "pencil.line"}
                  chevron
                  onPress={() => router.push({ pathname: "/chat-info/feedback-item", params: { bot: id, change: change.id } })}
                />
              );
            })}
          </Section>
        ) : null}

        {f.notes.length > 0 ? (
          <Section title={tc("Notes", "feedback")}>
            {f.notes.map((note) => (
              <Row key={note.id} title={noteTitle(f, note, routines)} subtitle={`${kindTitle(note.kind)} · ${stamp(new Date(note.createdAt * 1000))}`} icon={kindSymbol(note.kind)} action={false} onPress={() => showNote(note)} />
            ))}
          </Section>
        ) : null}
      </ScrollView>
    </>
  );
}
