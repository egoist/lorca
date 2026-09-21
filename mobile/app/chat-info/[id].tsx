import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Alert, Pressable, ScrollView, StyleSheet, Switch, Text, View } from "react-native";
import { chatTitle, engine } from "../../src/core/engine";
import { providerLabel, PROVIDER_KINDS, PROVIDER_MODELS, THINKING_LEVELS, thinkingLabel, type Bot, type Routine } from "../../src/core/model";
import { deviceIsOnline, useBotMap, useChat, useRoutines, useStore, useWorkingBotIds } from "../../src/core/store";
import { t } from "../../src/i18n";
import { AvatarCluster, BotAvatar } from "../../src/ui/Avatar";
import { CheckRow, FieldRow, Row, Section, ToggleRow } from "../../src/ui/forms";
import { lastRunSummary, lastSeen, routineDetail } from "../../src/ui/format";
import { Symbol } from "../../src/ui/Symbol";
import { usePalette } from "../../src/ui/theme";
import { deviceSymbol } from "../../src/ui/devices";

export default function ChatInfoScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const chat = useChat(id);
  const bots = useBotMap();
  const allBots = useStore((s) => s.bots);
  const devices = useStore((s) => s.devices);
  const seen = useStore((s) => s.device_seen);
  const working = useWorkingBotIds();
  const members = chat?.bot_ids.map((botID) => bots.get(botID)).filter((bot): bot is Bot => !!bot) ?? [];
  const isGroup = chat?.kind === "group";
  const bot = isGroup ? undefined : members[0];
  const [adding, setAdding] = useState(false);
  const [title, setTitle] = useState(chat?.title ?? "");
  const [botName, setBotName] = useState(bot?.name ?? "");
  const [botLabel, setBotLabel] = useState(bot?.label ?? "");
  const routines = useRoutines(chat?.kind === "dm" ? chat.bot_ids[0] : undefined);

  useEffect(() => setBotName(bot?.name ?? ""), [bot?.id, bot?.name]);
  useEffect(() => setBotLabel(bot?.label ?? ""), [bot?.id, bot?.label]);

  if (!chat) return null;

  /// A routine's actions, as a sheet: run it now, or delete it. The bot edits it on request.
  function showRoutine(routine: Routine) {
    Alert.alert(routine.name, `${routineDetail(routine)}\n${t("Last run: {summary}", { summary: lastRunSummary(routine) })}\n\n${routine.prompt}`, [
      { text: t("Run Now"), onPress: () => engine.runRoutine(routine.id) },
      {
        text: t("Delete"),
        style: "destructive",
        onPress: () =>
          Alert.alert(t("Delete “{name}”?", { name: routine.name }), t("This deletes the routine and stops its future runs. This can't be undone."), [
            { text: t("Cancel"), style: "cancel" },
            { text: t("Delete routine"), style: "destructive", onPress: () => engine.deleteRoutine(routine.id) },
          ]),
      },
      { text: t("Done"), style: "cancel" },
    ]);
  }
  const runner = bot ? devices.find((d) => d.id === bot.runner_id) : undefined;
  const providerModels = bot ? (PROVIDER_MODELS[bot.provider] ?? []) : [];
  const thinkingLevels = bot ? (THINKING_LEVELS[bot.provider] ?? []) : [];
  const defaultModel = providerModels[0]?.label;
  const modelValue = bot?.model
    ? providerModels.find((model) => model.id === bot.model)?.label ?? bot.model
    : defaultModel
      ? t("Default ({model})", { model: defaultModel })
      : t("Default");
  const candidates = allBots.filter((b) => !chat.bot_ids.includes(b.id));

  function commitTitle() {
    if ((title.trim() || null) !== (chat!.title?.trim() || null)) engine.renameGroup(chat!.id, title);
  }

  async function commitBotName() {
    if (!bot) return;
    const name = botName.trim() || bot.name;
    setBotName(name);
    if (name === bot.name) return;
    try {
      const updated = await engine.updateBot(bot.id, { name });
      setBotName(updated.name);
    } catch (error) {
      setBotName(bot.name);
      Alert.alert(t("Could not update the bot"), error instanceof Error ? error.message : String(error));
    }
  }

  async function commitBotLabel() {
    if (!bot) return;
    const label = botLabel.trim() || bot.label;
    setBotLabel(label);
    if (label === bot.label) return;
    try {
      const updated = await engine.updateBot(bot.id, { label });
      setBotLabel(updated.label);
    } catch (error) {
      setBotLabel(bot.label);
      Alert.alert(t("Could not update the bot"), error instanceof Error ? error.message : String(error));
    }
  }

  function confirmDelete() {
    Alert.alert(t("Delete “{name}”?", { name: chatTitle(chat!) }), t("The chat and its messages are removed from every paired Device."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Delete"),
        style: "destructive",
        onPress: () => {
          engine.deleteChat(chat!.id);
          router.dismissAll();
        },
      },
    ]);
  }

  return (
    <>
      <Stack.Screen options={{ title: isGroup ? t("Group Info") : t("Details") }} />
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="done" onPress={() => router.dismiss()}>
          {t("Done")}
        </Stack.Toolbar.Button>
      </Stack.Toolbar>
    <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag">
      <View style={styles.hero}>
        {bot ? (
          <Pressable onPress={() => router.push(`/chat-info/look/${bot.id}`)} accessibilityLabel={t("Change {name}'s look", { name: bot.name })} accessibilityRole="button" hitSlop={8}>
            <BotAvatar bot={bot} size={72} working={working.has(bot.id)} />
          </Pressable>
        ) : (
          <AvatarCluster bots={members} size={72} working={members.some((m) => working.has(m.id))} />
        )}
        <Text style={[styles.heroTitle, { color: p.label }]}>{chatTitle(chat)}</Text>
        {bot ? <Text style={[styles.heroSubtitle, { color: p.secondaryLabel }]}>{bot.label}</Text> : <Text style={[styles.heroSubtitle, { color: p.secondaryLabel }]}>{members.length === 1 ? t("{count} bot", { count: members.length }) : t("{count} bots", { count: members.length })}</Text>}
      </View>

      {bot && (
        <Section>
          <FieldRow label={t("Name")} value={botName} onChangeText={setBotName} onBlur={() => void commitBotName()} autoCapitalize="words" returnKeyType="done" submitBehavior="blurAndSubmit" textAlign="right" />
          <FieldRow label={t("Label")} value={botLabel} onChangeText={setBotLabel} onBlur={() => void commitBotLabel()} autoCapitalize="sentences" returnKeyType="done" submitBehavior="blurAndSubmit" textAlign="right" />
          <Row title={t("Description")} subtitle={bot.description || undefined} subtitleLines={2} chevron onPress={() => router.push(`/chat-info/description/${bot.id}`)} />
        </Section>
      )}

      {isGroup && (
        <Section title={t("Name")}>
          <FieldRow value={title} onChangeText={setTitle} placeholder={members.map((m) => m.name).join(", ")} onBlur={commitTitle} onSubmitEditing={commitTitle} returnKeyType="done" />
        </Section>
      )}

      {bot && (
        <Section title={t("Runs with")}>
          <Row
            title={t("Provider")}
            menu={{
              title: t("Provider"),
              value: providerLabel(bot.provider),
              choices: PROVIDER_KINDS.map((kind) => ({
                title: providerLabel(kind),
                selected: kind === bot.provider,
                onPress: () => {
                  if (kind !== bot.provider) engine.setBotRuntime(bot.id, kind, undefined, undefined);
                },
              })),
            }}
          />
          <Row
            title={t("Model")}
            menu={{
              title: t("Model"),
              value: modelValue,
              choices: [
                {
                  title: defaultModel ? t("Default ({model})", { model: defaultModel }) : t("Default"),
                  selected: !bot.model,
                  onPress: () => engine.setBotRuntime(bot.id, bot.provider, undefined, bot.thinking),
                  dividerAfter: true,
                },
                ...providerModels.map((model) => ({
                  title: model.label,
                  selected: bot.model === model.id,
                  onPress: () => engine.setBotRuntime(bot.id, bot.provider, model.id, bot.thinking),
                })),
              ],
            }}
          />
          <Row
            title={t("Thinking")}
            menu={{
              title: t("Thinking"),
              value: bot.thinking ? thinkingLabel(bot.thinking) : t("Default"),
              choices: [
                {
                  title: t("Default"),
                  selected: !bot.thinking,
                  onPress: () => engine.setBotRuntime(bot.id, bot.provider, bot.model, undefined),
                  dividerAfter: true,
                },
                ...thinkingLevels.map((level) => ({
                  title: thinkingLabel(level),
                  selected: bot.thinking === level,
                  onPress: () => engine.setBotRuntime(bot.id, bot.provider, bot.model, level),
                })),
              ],
            }}
          />
        </Section>
      )}

      {bot && runner && (
        <Section title={t("Runs on")}>
          <Row
            title={runner.name}
            subtitle={`${runner.model} · ${lastSeen(seen[runner.id])}`}
            leading={
              <View style={styles.deviceIcon}>
                <Symbol name={deviceSymbol(runner.os, runner.model)} size={22} color={p.label} />
                <View style={[styles.deviceDot, { backgroundColor: deviceIsOnline(runner.id) ? p.green : p.tertiaryLabel, borderColor: p.cell }]} />
              </View>
            }
          />
        </Section>
      )}

      {bot && (
        <Section title={t("Routines")} footer={routines.length === 0 ? t("Routines are recurring tasks this bot runs on a schedule. Ask it in chat to set one up.") : t("Runs post here. Ask {name} in chat to change one.", { name: bot.name })}>
          {routines.map((routine) => (
            <Row
              key={routine.id}
              title={routine.name}
              subtitle={routineDetail(routine)}
              icon={routine.is_running ? "arrow.triangle.2.circlepath" : routine.is_enabled ? "clock" : "pause.circle"}
              accessory={<Switch value={routine.is_enabled} onValueChange={(v) => engine.setRoutineEnabled(routine.id, v)} />}
              onPress={() => showRoutine(routine)}
            />
          ))}
        </Section>
      )}

      {bot && runner && (
        <Section title={t("Plugins")} footer={(runner.plugins ?? []).length === 0 ? t("No plugins on {runner} yet. Add one from the Mac app, or ask {bot} to find one.", { runner: runner.name, bot: bot.name }) : t("Installed on {runner}, for {bot} and every other bot there.", { runner: runner.name, bot: bot.name })}>
          {(runner.plugins ?? []).map((plugin) => (
            <Row
              key={plugin.id}
              title={plugin.name}
              subtitle={plugin.state === "ready" ? plugin.description : plugin.detail}
              icon={plugin.icon || "puzzlepiece.extension"}
            />
          ))}
        </Section>
      )}

      {isGroup && (
        <Section title={t("Members")} footer={chat.bot_ids.length >= 6 ? t("A group holds up to six bots.") : t("Bots in a group take turns answering; @mention one to hear from it first.")}>
          {members.map((member) => (
            <Row
              key={member.id}
              title={member.name}
              subtitle={member.label}
              leading={<BotAvatar bot={member} size={36} working={working.has(member.id)} />}
              accessory={
                chat.owner_bot_id === member.id ? (
                  <Text style={{ color: p.secondaryLabel, fontSize: 13 }}>{t("Owner")}</Text>
                ) : members.length > 1 ? (
                  <Pressable hitSlop={8} onPress={() => engine.removeBot(chat.id, member.id)} accessibilityLabel={t("Remove {name}", { name: member.name })}>
                    <Symbol name="xmark" size={14} color={p.tertiaryLabel} weight="semibold" />
                  </Pressable>
                ) : null
              }
              onPress={() => engine.setOwner(chat.id, member.id)}
            />
          ))}
          {chat.bot_ids.length < 6 && candidates.length > 0 ? <Row title={adding ? t("Choose a bot") : t("Add Bot")} icon="plus" onPress={() => setAdding((a) => !a)} /> : null}
        </Section>
      )}

      {isGroup && adding && (
        <Section>
          {candidates.map((candidate) => (
            <CheckRow
              key={candidate.id}
              title={candidate.name}
              subtitle={candidate.label}
              checked={false}
              leading={<BotAvatar bot={candidate} size={36} />}
              onPress={() => {
                engine.addBot(chat.id, candidate.id);
                setAdding(false);
              }}
            />
          ))}
        </Section>
      )}

      <Section>
        <ToggleRow title={t("Pinned")} icon="pin.fill" value={chat.is_pinned} onValueChange={(v) => engine.pinChat(chat.id, v)} />
      </Section>

      <Section>
        <Row title={t("Delete Chat")} icon="trash" destructive onPress={confirmDelete} />
      </Section>
    </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  hero: { alignItems: "center", paddingTop: 16, gap: 6 },
  heroTitle: { fontSize: 22, fontWeight: "700", marginTop: 6 },
  heroSubtitle: { fontSize: 15, textAlign: "center", paddingHorizontal: 32 },
  deviceIcon: { width: 32, alignItems: "center" },
  deviceDot: { position: "absolute", right: 0, bottom: -2, width: 10, height: 10, borderRadius: 5, borderWidth: 2 },
});
