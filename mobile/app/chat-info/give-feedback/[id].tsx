// Feedback on one of the bot's replies, after the desktop apps' Give Feedback…, which hangs off a
// message's menu there: on the phone a message's long press selects its text, so Details' Give
// Feedback opens this with the newest reply picked. Whether it was right, the user's own version,
// or a note for next time, and the routine or skill it applies to, go to the bot's Runner.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useMemo, useState } from "react";
import { ScrollView, StyleSheet } from "react-native";
import { loadFeedback, recordFeedback, useFeedback } from "../../../src/core/feedback";
import { useBotMap, useChat } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { feedbackSources, responses, type FeedbackKind } from "../../../src/ui/feedback";
import { CheckRow, FieldRow, Row, Section } from "../../../src/ui/forms";
import { firstLine, stamp } from "../../../src/ui/format";
import { SaveToolbar } from "../../../src/ui/navigation";

export default function GiveFeedbackScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const chat = useChat(id);
  const bots = useBotMap();
  const sources = useMemo(() => (chat ? feedbackSources(chat) : []), [chat?.messages]);
  const [picked, setPicked] = useState(sources[0]?.id);
  const message = sources.find((m) => m.id === picked);
  const botId = message?.author.kind === "bot" ? message.author.bot_id : undefined;
  const feedback = useFeedback(botId);
  const original = message?.body.kind === "text" ? message.body.text : "";
  const [kind, setKind] = useState<FeedbackKind>("accepted");
  const [version, setVersion] = useState(original);
  const [note, setNote] = useState("");
  const [target, setTarget] = useState(-1);
  const [busy, setBusy] = useState(false);

  // The routines and skills the note can be about, as the Runner names them.
  useEffect(() => {
    if (botId) void loadFeedback(botId);
  }, [botId]);
  // Another reply picked starts its own version.
  useEffect(() => setVersion(original), [picked]);

  if (!chat) return null;
  const targets = feedback?.targets ?? [];
  const response = responses.find((r) => r.kind === kind)!;
  const canSave = !!message && !!botId && !busy && (kind === "explicit" ? note.trim().length > 0 : kind === "edited" ? version !== original : true);

  async function save() {
    if (!canSave || !message || !botId) return;
    setBusy(true);
    try {
      await recordFeedback(botId, { kind, chatId: chat!.id, messageId: message.id, note: note.trim(), before: original, after: version, target: targets[target]?.target });
      router.back();
    } catch (error) {
      setBusy(false);
      alert(t("Couldn't save your feedback"), error instanceof Error ? error.message : String(error));
    }
  }

  const name = botId ? bots.get(botId)?.name ?? "" : "";
  return (
    <>
      <Stack.Screen options={{ title: t("Feedback") }} />
      <SaveToolbar label={t("Save")} disabled={!canSave} onSave={() => void save()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <Section title={t("Reply")} footer={sources.length === 0 ? t("Nothing here to give feedback on yet.") : name ? t("{name} uses it to suggest changes to its routines and skills.", { name }) : undefined}>
          {sources.map((m) => (
            <CheckRow key={m.id} title={firstLine(m.body.kind === "text" ? m.body.text : "")} subtitle={stamp(new Date(m.created_at * 1000))} checked={m.id === picked} onPress={() => setPicked(m.id)} />
          ))}
        </Section>
        <Section>
          <Row title={t("Response")} menu={{ value: response.title(), title: t("Response"), choices: responses.map((r) => ({ title: r.title(), selected: r.kind === kind, onPress: () => setKind(r.kind) })) }} />
        </Section>
        {kind === "edited" ? (
          <Section title={t("Your version")}>
            <FieldRow value={version} onChangeText={setVersion} multiline editable={!busy} accessibilityLabel={t("Your version")} style={styles.version} />
          </Section>
        ) : null}
        <Section>
          <FieldRow value={note} onChangeText={setNote} placeholder={response.placeholder()} multiline editable={!busy} accessibilityLabel={t("Note")} style={styles.note} />
        </Section>
        <Section>
          <Row
            title={t("Applies to")}
            menu={{
              value: targets[target]?.name ?? t("Any"),
              title: t("Applies to"),
              choices: [{ title: t("Any"), selected: target < 0, onPress: () => setTarget(-1), dividerAfter: targets.length > 0 }, ...targets.map((each, index) => ({ title: each.name, selected: index === target, onPress: () => setTarget(index) }))],
            }}
          />
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  version: { minHeight: 160 },
  note: { minHeight: 64 },
});
