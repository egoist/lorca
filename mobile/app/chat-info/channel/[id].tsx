// A channel, slid in from its row in Details: whether it listens, and what holds it when a
// message's turn didn't finish or its account can't be read; where it listens and what it takes;
// the chats asking to be allowed; the task the bot does with each message; its intake (what it takes
// on by itself, who can ask directly, its rules, and what it decided lately); and its conversations,
// each a tap away. The bot sets a channel up and changes it when asked in chat; here the user
// pauses it, allows or ignores a chat, settles a held message, sets its intake, or removes it.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { useShallow } from "zustand/react/shallow";
import { chatTitle, engine } from "../../../src/core/engine";
import { deviceName, isWorkChat, type ChannelIntake, type ChatRequest, type IntakeDecision, type IntakePerson } from "../../../src/core/model";
import { channelById, deviceIsOnline, useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert, prompt } from "../../../src/ui/alert";
import {
  channelChats,
  decisionKindTitle,
  decisionPerson,
  decisionTitle,
  INTAKE_CHOICES,
  intakeChoiceFooter,
  intakeChoiceOf,
  intakeChoiceTitle,
  intakeWithChoice,
  intakeWithoutPerson,
  intakeWithPerson,
  intakeWithRule,
  listenSummary,
  personName,
  requestKind,
  serviceName,
  shownDecisions,
} from "../../../src/ui/channels";
import { Row, Section, ToggleRow } from "../../../src/ui/forms";
import { ago, stamp } from "../../../src/ui/format";
import { usePalette } from "../../../src/ui/theme";

/// How many decisions the screen asks for, so what was left alone can't crowd out the ones it lists.
const DECISIONS_ASKED = 100;

/// What the channel's intake decided lately, asked of its Runner when the screen opens, when intake
/// goes on, and when the Runner comes or goes. `retry` asks again.
function useDecisions(channelId: string, on: boolean, online: boolean) {
  const [state, setState] = useState<{ decisions?: IntakeDecision[]; failed?: boolean }>({});
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!on) return;
    let live = true;
    engine.channelDecisions(channelId, DECISIONS_ASKED).then(
      (decisions) => live && setState({ decisions }),
      () => live && setState({ failed: true }),
    );
    return () => {
      live = false;
    };
  }, [channelId, on, online, attempt]);
  return {
    ...state,
    retry: () => {
      setState({});
      setAttempt((each) => each + 1);
    },
  };
}

export default function ChannelScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const runner = useStore((s) => s.devices.find((d) => d.channels?.some((c) => c.id === id)));
  const channel = runner?.channels?.find((c) => c.id === id);
  const bot = useStore((s) => s.bots.find((b) => b.id === channel?.bot_id));
  const account = runner?.plugins?.find((plugin) => plugin.id === channel?.account_id);
  const conversations = useStore(useShallow((s) => s.chats.filter((c) => c.channel?.channel_id === id && !isWorkChat(c))));
  const chatIds = useStore(useShallow((s) => s.chats.map((c) => c.id)));
  const tasks = useStore((s) => s.tasks);
  // Off keeps what the owner wrote, and shows none of it.
  const savedIntake = channel?.intake ?? null;
  const intake = savedIntake?.mode === "off" ? null : savedIntake;
  const online = !!runner && deviceIsOnline(runner.id);
  const decisions = useDecisions(id, !!intake, online);
  if (!channel) return null;
  const kept = [...conversations].sort((a, b) => (b.messages.at(-1)?.created_at ?? b.created_at) - (a.messages.at(-1)?.created_at ?? a.created_at)).slice(0, 6);
  const held = channel.state === "held";
  // A held message can be tried again or skipped; a channel waiting for the user has nothing to settle.
  const settles = held && !!channel.held_delivery;
  const footer = settles
    ? t("A message’s turn didn’t finish, so later messages wait. Read its conversation, then try it again or skip it.")
    : channel.detail || undefined;
  const hint =
    channel.service === "slack"
      ? t("Invite the bot to a channel or send it a message, then allow the chat here.")
      : t("Add the bot to a group or send it a message, then allow the chat here.");

  // Anyone can message the bot or add it to a group; it answers only where the user allows.
  function answer(request: ChatRequest) {
    const name = bot?.name ?? t("The bot");
    alert(t("Let “{chat}” chat with {name}?", { chat: request.title, name }), t("{name} answers there from now on. An ignored chat doesn’t ask again.", { name }), [
      { text: t("Cancel"), style: "cancel" },
      { text: t("Ignore"), style: "destructive", onPress: () => engine.answerChatRequest(channel!.id, request.id, false) },
      { text: t("Allow"), onPress: () => engine.answerChatRequest(channel!.id, request.id, true) },
    ]);
  }

  // For a chat that can't ask itself: one where the bot hears nothing it would answer.
  function askChatId() {
    prompt(t("Add a Chat"), {
      message:
        channel!.service === "slack"
          ? t("Paste the channel’s id, such as C0123456789. Or message the bot there, and the chat asks here.")
          : t("Paste the chat’s id, such as -1001234567890 for a group. Or message the bot there, and the chat asks here."),
      placeholder: t("Chat ID"),
      confirm: t("Add"),
      done: (text) => {
        const chatId = text.trim();
        if (!chatId) return;
        engine.addChat(channel!.id, chatId).catch((error) => alert(t("Couldn’t add the chat"), error instanceof Error ? error.message : String(error)));
      },
    });
  }

  function confirmRemove() {
    alert(t("Remove “{name}”?", { name: channel!.name }), t("{name} stops listening there. Its conversations stay.", { name: bot?.name ?? t("The bot") }), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Remove"),
        style: "destructive",
        onPress: () => {
          engine.removeChannel(channel!.id);
          router.back();
        },
      },
    ]);
  }

  function openChat(chatId: string) {
    router.dismissAll();
    router.push(`/chat/${chatId}`);
  }

  // Each write replaces the whole intake, so it starts from the intake as it stands by then.
  function saveIntake(next: ChannelIntake | null) {
    engine.setChannelIntake(channel!.id, next).catch((error) => alert(t("Couldn’t save the intake"), error instanceof Error ? error.message : String(error)));
  }

  function changeIntake(change: (now: ChannelIntake) => ChannelIntake) {
    const now = channelById(channel!.id)?.channel.intake;
    if (now) saveIntake(change(now));
  }

  function askPerson() {
    prompt(t("Add a Person"), {
      message: channel!.service === "slack" ? t("Paste their Slack member ID, such as U0123456789.") : t("Paste their numeric Telegram user ID."),
      placeholder: t("User ID"),
      confirm: t("Add"),
      done: (text) => {
        const personId = text.trim();
        if (personId) changeIntake((now) => intakeWithPerson(now, { id: personId }));
      },
    });
  }

  function confirmRemovePerson(person: IntakePerson) {
    const name = personName(person);
    alert(t("Remove “{name}”?", { name }), t("{name} can no longer ask directly.", { name }), [
      { text: t("Cancel"), style: "cancel" },
      { text: t("Remove"), style: "destructive", onPress: () => changeIntake((now) => intakeWithoutPerson(now, person.id)) },
    ]);
  }

  function offerPerson(person: IntakePerson) {
    const name = personName(person);
    alert(name, undefined, [
      { text: t("Cancel"), style: "cancel" },
      { text: t("Let {name} Ask Directly", { name }), onPress: () => changeIntake((now) => intakeWithPerson(now, person)) },
    ]);
  }

  /// Where a decision's row leads: the task it came to, which the phone may not have yet, else its
  /// conversation.
  function decisionTarget(decision: IntakeDecision): (() => void) | undefined {
    const taskId = decision.task_id;
    if (taskId && tasks.some((task) => task.id === taskId)) {
      return () => router.push({ pathname: "/chat-info/durable-task/[id]", params: { id: taskId, chat: decision.conversation_id } });
    }
    return chatIds.includes(decision.conversation_id) ? () => openChat(decision.conversation_id) : undefined;
  }

  const choice = intakeChoiceOf(savedIntake);
  const people = intake?.people ?? [];
  const rules = intake?.rules ?? [];

  return (
    <>
      <Stack.Screen options={{ title: channel.name }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content}>
        <Section footer={footer}>
          <ToggleRow title={t("Listening")} value={channel.state !== "paused"} onValueChange={(on) => engine.setChannelPaused(channel.id, !on)} />
          {held && <Row title={t("State")} detail={t("On hold")} />}
          {settles && <Row title={t("Try Again")} action onPress={() => engine.settleHeldMessage(channel.id, true)} />}
          {settles && <Row title={t("Skip")} action onPress={() => engine.settleHeldMessage(channel.id, false)} />}
          {channel.state === "offline" && <Row title={t("State")} detail={t("Can’t connect")} />}
        </Section>
        <Section footer={!channel.chats?.length && !channel.requests?.length ? hint : undefined}>
          <Row title={t("Account")} detail={account?.name ?? serviceName(channel.service)} />
          <Row title={t("Chats")} subtitle={channelChats(channel)} subtitleLines={3} />
          <Row title={t("Add Chat…")} action onPress={askChatId} />
          <Row title={t("Messages")} subtitle={listenSummary(channel.listen)} subtitleLines={2} />
        </Section>
        {!!channel.requests?.length && (
          <Section title={t("Asking to Chat")} footer={t("{name} answers only in chats you allow. An ignored chat doesn’t ask again.", { name: bot?.name ?? t("The bot") })}>
            {channel.requests.map((request) => (
              <Row key={request.id} title={request.title} subtitle={requestKind(request)} onPress={() => answer(request)} />
            ))}
          </Section>
        )}
        <Section title={t("Task")}>
          <View style={styles.task}>
            <Text style={[styles.taskText, { color: p.label }]} selectable>
              {channel.task}
            </Text>
          </View>
        </Section>
        <Section title={t("Intake")} footer={intakeChoiceFooter(choice)}>
          <Row
            title={t("Takes on work")}
            menu={{
              title: t("Takes on work"),
              value: intakeChoiceTitle(choice),
              choices: INTAKE_CHOICES.map((each) => ({
                title: intakeChoiceTitle(each),
                selected: each === choice,
                onPress: () => {
                  if (each !== choice) saveIntake(intakeWithChoice(channelById(channel.id)?.channel.intake, each));
                },
              })),
            }}
          />
        </Section>
        {intake && (
          <Section title={t("Can Ask Directly")}>
            {people.map((person) => (
              <Row key={person.id} title={personName(person)} action={false} onPress={() => confirmRemovePerson(person)} />
            ))}
            <Row title={t("Add Person…")} action onPress={askPerson} />
          </Section>
        )}
        {intake && (
          <Section title={t("Rules")} footer={rules.length ? undefined : t("No rules yet: it only takes on what someone you list asks for.")}>
            {rules.map((rule) => (
              <ToggleRow
                key={rule.id || rule.name}
                title={rule.name}
                value={rule.enabled}
                onValueChange={(enabled) => changeIntake((now) => intakeWithRule(now, { ...rule, enabled }))}
                onPress={() => router.push({ pathname: "/chat-info/channel/rule", params: { channel: channel.id, rule: rule.id } })}
              />
            ))}
            <Row title={t("Add Rule…")} action onPress={() => router.push({ pathname: "/chat-info/channel/rule", params: { channel: channel.id } })} />
          </Section>
        )}
        {intake && (decisions.failed || decisions.decisions) && (
          <Section title={t("Decisions")}>
            {decisions.failed ? (
              <Row
                title={runner && !online ? t("{name} is offline", { name: deviceName(runner) }) : t("Couldn’t load the decisions")}
                subtitle={t("Tap to try again")}
                action={false}
                onPress={decisions.retry}
              />
            ) : shownDecisions(decisions.decisions ?? []).length === 0 ? (
              <Row title={t("None yet")} />
            ) : (
              shownDecisions(decisions.decisions ?? []).map((decision) => {
                const person = decisionPerson(decision, intake);
                const target = decisionTarget(decision);
                return (
                  <Row
                    key={decision.id}
                    title={decisionTitle(decision)}
                    titleLines={2}
                    subtitle={[decisionKindTitle(decision.kind), decision.sender || decision.sender_id, ago(decision.at)].filter(Boolean).join(" · ")}
                    subtitleLines={2}
                    action={false}
                    chevron={!!target}
                    onPress={target}
                    onLongPress={person ? () => offerPerson(person) : undefined}
                  />
                );
              })
            )}
          </Section>
        )}
        {kept.length > 0 && (
          <Section title={t("Conversations")}>
            {kept.map((chat) => (
              <Row
                key={chat.id}
                title={chatTitle(chat)}
                detail={stamp(new Date((chat.messages.at(-1)?.created_at ?? chat.created_at) * 1000))}
                chevron
                onPress={() => openChat(chat.id)}
              />
            ))}
          </Section>
        )}
        <Section>
          <Row title={t("Remove Channel…")} destructive onPress={confirmRemove} />
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 32 },
  task: { paddingHorizontal: 16, paddingVertical: 12 },
  taskText: { fontSize: 15, lineHeight: 21 },
});
