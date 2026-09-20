import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { Alert, Pressable, ScrollView, StyleSheet, Switch, Text, View } from "react-native";
import { chatTitle, engine } from "../../src/core/engine";
import { isRunner, providerLabel, thinkingLabel, type Bot, type Routine } from "../../src/core/model";
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
  const [adding, setAdding] = useState(false);
  const [title, setTitle] = useState(chat?.title ?? "");
  const routines = useRoutines(chat?.kind === "dm" ? chat.bot_ids[0] : undefined);

  if (!chat) return null;
  const members = chat.bot_ids.map((b) => bots.get(b)).filter((b): b is Bot => !!b);
  const isGroup = chat.kind === "group";
  const bot = isGroup ? undefined : members[0];

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
  const candidates = allBots.filter((b) => !chat.bot_ids.includes(b.id));

  function commitTitle() {
    if ((title.trim() || null) !== (chat!.title?.trim() || null)) engine.renameGroup(chat!.id, title);
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

      {isGroup && (
        <Section title={t("Name")}>
          <FieldRow value={title} onChangeText={setTitle} placeholder={members.map((m) => m.name).join(", ")} onBlur={commitTitle} onSubmitEditing={commitTitle} returnKeyType="done" />
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
          <Row title={t("Provider")} detail={`${providerLabel(bot.provider)}${bot.model ? ` · ${bot.model}` : ""}`} />
          <Row title={t("Thinking")} detail={bot.thinking ? thinkingLabel(bot.thinking) : t("Default")} />
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

      {bot && bot.instructions ? (
        <Section title={t("Instructions")}>
          <View style={styles.instructions}>
            <Text style={{ color: p.label, fontSize: 15, lineHeight: 21 }}>{bot.instructions}</Text>
          </View>
        </Section>
      ) : null}

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
  instructions: { paddingHorizontal: 16, paddingVertical: 12 },
});
