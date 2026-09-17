import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { Alert, Pressable, ScrollView, StyleSheet, Switch, Text, View } from "react-native";
import { chatTitle, engine } from "../../src/core/engine";
import { isRunner, providerLabel, thinkingLabel, type Bot, type Routine } from "../../src/core/model";
import { deviceIsOnline, useBotMap, useChat, useRoutines, useStore, useWorkingBotIds } from "../../src/core/store";
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
    Alert.alert(routine.name, `${routineDetail(routine)}\nLast run: ${lastRunSummary(routine)}\n\n${routine.prompt}`, [
      { text: "Run Now", onPress: () => engine.runRoutine(routine.id) },
      {
        text: "Delete",
        style: "destructive",
        onPress: () =>
          Alert.alert(`Delete “${routine.name}”?`, "This deletes the routine and stops its future runs. This can't be undone.", [
            { text: "Cancel", style: "cancel" },
            { text: "Delete routine", style: "destructive", onPress: () => engine.deleteRoutine(routine.id) },
          ]),
      },
      { text: "Done", style: "cancel" },
    ]);
  }
  const runner = bot ? devices.find((d) => d.id === bot.runner_id) : undefined;
  const candidates = allBots.filter((b) => !chat.bot_ids.includes(b.id));

  function commitTitle() {
    if ((title.trim() || null) !== (chat!.title?.trim() || null)) engine.renameChat(chat!.id, title);
  }

  function confirmDelete() {
    Alert.alert(`Delete “${chatTitle(chat!)}”?`, "The chat and its messages are removed from every paired Device.", [
      { text: "Cancel", style: "cancel" },
      {
        text: "Delete",
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
      <Stack.Screen options={{ title: isGroup ? "Group Info" : "Details" }} />
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="done" onPress={() => router.dismiss()}>
          Done
        </Stack.Toolbar.Button>
      </Stack.Toolbar>
    <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag">
      <View style={styles.hero}>
        <AvatarCluster bots={members} size={72} working={members.some((m) => working.has(m.id))} />
        <Text style={[styles.heroTitle, { color: p.label }]}>{chatTitle(chat)}</Text>
        {bot ? <Text style={[styles.heroSubtitle, { color: p.secondaryLabel }]}>{bot.label}</Text> : <Text style={[styles.heroSubtitle, { color: p.secondaryLabel }]}>{members.length} {members.length === 1 ? "bot" : "bots"}</Text>}
      </View>

      {isGroup && (
        <Section title="Name">
          <FieldRow value={title} onChangeText={setTitle} placeholder={members.map((m) => m.name).join(", ")} onBlur={commitTitle} onSubmitEditing={commitTitle} returnKeyType="done" />
        </Section>
      )}

      {bot && runner && (
        <Section title="Runs on">
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
          <Row title="Provider" detail={`${providerLabel(bot.provider)}${bot.model ? ` · ${bot.model}` : ""}`} />
          <Row title="Thinking" detail={bot.thinking ? thinkingLabel(bot.thinking) : "Default"} />
        </Section>
      )}

      {bot && (
        <Section title="Routines" footer={routines.length === 0 ? "Routines are recurring tasks this bot runs on a schedule. Ask it in chat to set one up." : `Runs post here. Ask ${bot.name} in chat to change one.`}>
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
        <Section title="Plugins" footer={(runner.plugins ?? []).length === 0 ? `No plugins on ${runner.name} yet. Add one from the Mac app, or ask ${bot.name} to find one.` : `Installed on ${runner.name}; on means ${bot.name} may use it.`}>
          {(runner.plugins ?? []).map((plugin) => {
            const enabled = (bot.plugins ?? []).includes(plugin.id);
            return (
              <Row
                key={plugin.id}
                title={plugin.name}
                subtitle={plugin.state === "ready" ? plugin.description : plugin.detail}
                icon={plugin.icon || "puzzlepiece.extension"}
                accessory={
                  <Switch
                    value={enabled}
                    onValueChange={(v) => engine.setBotPlugins(bot.id, v ? [...(bot.plugins ?? []), plugin.id] : (bot.plugins ?? []).filter((id) => id !== plugin.id))}
                  />
                }
              />
            );
          })}
        </Section>
      )}

      {bot && bot.instructions ? (
        <Section title="Instructions">
          <View style={styles.instructions}>
            <Text style={{ color: p.label, fontSize: 15, lineHeight: 21 }}>{bot.instructions}</Text>
          </View>
        </Section>
      ) : null}

      {isGroup && (
        <Section title="Members" footer={chat.bot_ids.length >= 6 ? "A group holds up to six bots." : "Bots in a group take turns answering; @mention one to hear from it first."}>
          {members.map((member) => (
            <Row
              key={member.id}
              title={member.name}
              subtitle={member.label}
              leading={<BotAvatar bot={member} size={36} working={working.has(member.id)} />}
              accessory={
                chat.owner_bot_id === member.id ? (
                  <Text style={{ color: p.secondaryLabel, fontSize: 13 }}>Owner</Text>
                ) : members.length > 1 ? (
                  <Pressable hitSlop={8} onPress={() => engine.removeBot(chat.id, member.id)} accessibilityLabel={`Remove ${member.name}`}>
                    <Symbol name="xmark" size={14} color={p.tertiaryLabel} weight="semibold" />
                  </Pressable>
                ) : null
              }
              onPress={() => engine.setOwner(chat.id, member.id)}
            />
          ))}
          {chat.bot_ids.length < 6 && candidates.length > 0 ? <Row title={adding ? "Choose a bot" : "Add Bot"} icon="plus" onPress={() => setAdding((a) => !a)} /> : null}
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
        <ToggleRow title="Pinned" icon="pin.fill" value={chat.is_pinned} onValueChange={(v) => engine.pinChat(chat.id, v)} />
      </Section>

      <Section>
        <Row title="Delete Chat" icon="trash" destructive onPress={confirmDelete} />
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
