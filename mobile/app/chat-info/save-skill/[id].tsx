// Save a skill from this chat, after the desktop apps' Save as Skill and Save as Standing
// Instruction, which hang off a message's menu there: on the phone a message's long press selects
// its text, so Details' Add Skill opens this instead. A bot's reply picked makes a workflow from
// how it was done; two or more of the user's own messages make a standing instruction from the
// corrections. In a group, Used by picks the group or the bot. Only the picked messages go to the
// bot's Runner, whose provider writes a draft; the draft opens for review and is used once saved.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useMemo, useState } from "react";
import { ActivityIndicator, ScrollView } from "react-native";
import { chatTitle, engine } from "../../../src/core/engine";
import type { Message, PlaybookScope } from "../../../src/core/model";
import { useBotMap, useChat } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { CheckRow, Row, Section } from "../../../src/ui/forms";
import { SaveToolbar } from "../../../src/ui/navigation";
import { captureKind, captureSources } from "../../../src/ui/skills";
import { usePalette } from "../../../src/ui/theme";

export default function SaveSkillScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const chat = useChat(id);
  const bots = useBotMap();
  const sources = useMemo(() => (chat ? captureSources(chat) : []), [chat?.messages]);
  // The newest reply and the request before it start picked.
  const [picked, setPicked] = useState<Set<string>>(() => {
    const reply = [...sources].reverse().find((m) => m.author.kind === "bot");
    const request = reply ? [...sources].reverse().find((m) => m.author.kind === "you" && m.created_at <= reply.created_at) : undefined;
    return new Set([reply?.id, request?.id].filter((each): each is string => !!each));
  });
  const [forBot, setForBot] = useState(false);
  const [busy, setBusy] = useState(false);
  if (!chat) return null;

  const pickedMessages = sources.filter((m) => picked.has(m.id));
  const made = captureKind(chat, pickedMessages);
  const isGroup = chat.kind === "group";
  const botId = made?.botId ?? chat.owner_bot_id ?? chat.bot_ids[0];
  const botName = bots.get(botId)?.name ?? t("The bot");
  const scope: PlaybookScope = isGroup && !forBot ? { kind: "project", id: chat.id } : { kind: "bot", id: botId };

  /// A workflow is one bot's: picking another bot's reply leaves the first bot's out.
  function toggle(message: Message) {
    setPicked((current) => {
      const next = new Set(current);
      if (next.has(message.id)) next.delete(message.id);
      else {
        next.add(message.id);
        if (message.author.kind === "bot") {
          const author = message.author.bot_id;
          for (const other of sources) if (other.author.kind === "bot" && other.author.bot_id !== author) next.delete(other.id);
        }
      }
      return next;
    });
  }

  async function draft() {
    if (!made || busy) return;
    setBusy(true);
    try {
      const record = await engine.draftPlaybook(scope, made.botId, chat!.id, made.kind, pickedMessages.map((m) => m.id));
      router.replace({ pathname: "/chat-info/skill/[id]", params: { id: record.id, kind: record.scope.kind, scope: record.scope.id } });
    } catch (error) {
      alert(t("Couldn't write the draft"), error instanceof Error ? error.message : String(error));
    } finally {
      setBusy(false);
    }
  }

  const author = (message: Message) => (message.author.kind === "you" ? t("You") : (message.author.kind === "bot" && bots.get(message.author.bot_id)?.name) || t("Bot"));
  const preview = (message: Message) => (message.body.kind === "text" ? message.body.text.replace(/\s+/g, " ").trim() : "");

  return (
    <>
      <Stack.Screen options={{ title: made?.kind === "corrections" ? t("Save as Standing Instruction") : t("Save as Skill") }} />
      <SaveToolbar label={t("Continue")} disabled={!made || busy} onSave={() => void draft()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        {isGroup && (
          <Section>
            <Row
              title={t("Used by")}
              menu={{
                title: t("Used by"),
                value: forBot ? t("{bot}, in every chat", { bot: botName }) : t("The bots in {group}", { group: chatTitle(chat) }),
                choices: [
                  { title: t("The bots in {group}", { group: chatTitle(chat) }), selected: !forBot, onPress: () => setForBot(false) },
                  { title: t("{bot}, in every chat", { bot: botName }), selected: forBot, onPress: () => setForBot(true) },
                ],
              }}
            />
          </Section>
        )}
        {busy && (
          <Section>
            <Row title={t("Writing a draft…")} leading={<ActivityIndicator size="small" color={p.secondaryLabel as any} />} />
          </Section>
        )}
        <Section title={t("Messages")} footer={t("Pick a reply to save how it was done, or two or more of your corrections to save them as a standing instruction. You review the draft next.")}>
          {sources.map((message) => (
            <CheckRow key={message.id} title={author(message)} subtitle={preview(message)} checked={picked.has(message.id)} multiple onPress={() => !busy && toggle(message)} />
          ))}
        </Section>
      </ScrollView>
    </>
  );
}
