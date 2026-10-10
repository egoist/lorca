// A bot's email or Slack message, opened from its card in the chat as Mail's compose sheet is: To,
// Cc, Bcc, and Subject as fields, the text, the attachments with a button to leave one out, and
// Send in the bar, which sends what the fields show (an edit first, as the next version). Cancel
// leaves it waiting as the bot wrote it. A Slack message also offers Always Send, which has the bot
// send its next messages directly; Discard sends nothing. Once sent or discarded, the sheet shows
// the message as it went.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { Platform, ScrollView, StyleSheet } from "react-native";
import { engine } from "../../src/core/engine";
import type { MessageDraft } from "../../src/core/model";
import { useChat, useStore } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { alert } from "../../src/ui/alert";
import { draftStateWord, draftTitle, fileSize, recipients, sendable } from "../../src/ui/drafts";
import { FieldRow, Row, Section } from "../../src/ui/forms";
import { CloseToolbar, FormToolbar } from "../../src/ui/navigation";
import { Pressable } from "../../src/ui/Pressable";
import { Symbol } from "../../src/ui/Symbol";
import { usePalette } from "../../src/ui/theme";

export default function DraftScreen() {
  useLanguage();
  const { id, chat: chatId } = useLocalSearchParams<{ id: string; chat: string }>();
  const router = useRouter();
  const p = usePalette();
  const message = useChat(chatId)?.messages.find((m) => m.id === id);
  const bot = useStore((s) => s.bots.find((each) => message?.author.kind === "bot" && each.id === message.author.bot_id));
  const card = message?.body.kind === "draft" ? message.body : undefined;
  const [to, setTo] = useState((card?.draft.to ?? []).join(", "));
  const [cc, setCc] = useState((card?.draft.cc ?? []).join(", "));
  const [bcc, setBcc] = useState((card?.draft.bcc ?? []).join(", "));
  const [subject, setSubject] = useState(card?.draft.subject ?? "");
  const [body, setBody] = useState(card?.draft.body ?? "");
  const [files, setFiles] = useState(card?.draft.attachments ?? []);
  const [busy, setBusy] = useState(false);
  if (!message || !card) return null;
  const pending = card.state === "pending";
  const email = card.draft.kind === "email";
  const editable = pending && !busy;
  const shown: MessageDraft = {
    ...card.draft,
    to: recipients(to),
    ...(email ? { cc: recipients(cc), bcc: recipients(bcc), subject } : {}),
    body,
    attachments: files,
  };

  async function act(failure: string, request: () => Promise<unknown>) {
    setBusy(true);
    try {
      await request();
      router.back();
    } catch (error) {
      setBusy(false);
      alert(failure, error instanceof Error ? error.message : String(error));
    }
  }
  const send = (always: boolean) => void act(t("Couldn't send this draft"), () => engine.sendDraft(chatId, id, shown, always));
  const state = draftStateWord(card.state);

  return (
    <>
      <Stack.Screen options={{ title: draftTitle(card.draft, bot?.name ?? t("The bot")) }} />
      {pending ? (
        <FormToolbar cancelLabel={t("Cancel")} saveLabel={t("Send")} saveDisabled={busy || !sendable(shown)} onCancel={() => router.back()} onSave={() => send(false)} />
      ) : (
        <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.back()} />
      )}
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        <Section title={card.account} footer={card.note ?? (state && !pending ? state : undefined)}>
          <FieldRow label={t("To")} value={to} onChangeText={setTo} editable={editable} autoCapitalize="none" autoCorrect={false} keyboardType={email ? "email-address" : "default"} />
          {email && (card.draft.cc?.length ?? 0) > 0 ? (
            <FieldRow label={t("Cc")} value={cc} onChangeText={setCc} editable={editable} autoCapitalize="none" autoCorrect={false} keyboardType="email-address" />
          ) : null}
          {email && (card.draft.bcc?.length ?? 0) > 0 ? (
            <FieldRow label={t("Bcc")} value={bcc} onChangeText={setBcc} editable={editable} autoCapitalize="none" autoCorrect={false} keyboardType="email-address" />
          ) : null}
          {email ? <FieldRow label={t("Subject")} value={subject} onChangeText={setSubject} editable={editable} /> : null}
          <FieldRow value={body} onChangeText={setBody} editable={editable} multiline accessibilityLabel={t("Message")} style={styles.body} />
        </Section>

        {files.length > 0 ? (
          <Section>
            {files.map((file, index) => (
              <Row
                key={`${file.name}-${index}`}
                title={file.name}
                detail={file.size > 0 ? fileSize(file.size) : undefined}
                icon="paperclip"
                accessory={
                  pending ? (
                    <Pressable
                      onPress={() => setFiles(files.filter((_, at) => at !== index))}
                      disabled={busy}
                      hitSlop={8}
                      accessibilityRole="button"
                      accessibilityLabel={t("Remove {name}", { name: file.name })}
                    >
                      <Symbol name="xmark.circle.fill" size={18} color={p.tertiaryLabel} />
                    </Pressable>
                  ) : undefined
                }
              />
            ))}
          </Section>
        ) : null}

        {pending && card.direct ? (
          <Section footer={t("Always Send also sends this bot's next Slack messages directly.")}>
            <Row title={t("Always Send")} disabled={busy || !sendable(shown)} onPress={() => send(true)} />
          </Section>
        ) : null}

        {pending ? (
          <Section>
            <Row title={t("Discard")} destructive onPress={busy ? undefined : () => void act(t("Couldn't discard this draft"), () => engine.discardDraft(chatId, id))} />
          </Section>
        ) : null}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  body: { minHeight: 200 },
});
