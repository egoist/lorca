import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { MenuView, type MenuComponentRef } from "@expo/ui/community/menu";
import { Platform, ScrollView, StyleSheet, Switch, Text, View } from "react-native";
import { Pressable } from "../../src/ui/Pressable";
import { chatTitle, engine } from "../../src/core/engine";
import { providerKinds, providerLabel, providerModels, PROVIDER_KINDS, taskSymbol, thinkingLabel, thinkingLevels, withCustomModels, type Bot, type Routine } from "../../src/core/model";
import { deviceIsOnline, useBotMap, useChat, useDurableTasks, useOpenReviews, useOutputs, useRoutines, useStore, useWorkingBotIds } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { AvatarCluster, BotAvatar } from "../../src/ui/Avatar";
import { CheckRow, FieldRow, Row, Section, ToggleRow } from "../../src/ui/forms";
import { pluginStateWord } from "../../src/ui/plugins";
import { lastRunSummary, lastSeen, routineDetail } from "../../src/ui/format";
import { Symbol } from "../../src/ui/Symbol";
import { usePalette } from "../../src/ui/theme";
import { deviceSymbol } from "../../src/ui/devices";
import { AndroidIcons, CloseToolbar } from "../../src/ui/navigation";
import { haptic } from "../../src/ui/haptics";
import { alert } from "../../src/ui/alert";
import { OutputRow } from "../../src/ui/outputs";
import { taskStateTitle, useTaskTint } from "../../src/ui/durableTasks";
import { reviewHeadline, reviewStateWord, reviewSymbol } from "../../src/ui/reviews";

export default function ChatInfoScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const chat = useChat(id);
  const bots = useBotMap();
  const allBots = useStore((s) => s.bots);
  const devices = useStore((s) => s.devices);
  const seen = useStore((s) => s.device_seen);
  const providers = useStore((s) => s.providers);
  // The core's catalog, and the custom providers' saved models after it.
  const catalog = withCustomModels(useStore((s) => s.models), providers);
  const working = useWorkingBotIds();
  const members = chat?.bot_ids.map((botID) => bots.get(botID)).filter((bot): bot is Bot => !!bot) ?? [];
  const isGroup = chat?.kind === "group";
  const bot = isGroup ? undefined : members[0];
  const [adding, setAdding] = useState(false);
  const [title, setTitle] = useState(chat?.title ?? "");
  const [botName, setBotName] = useState(bot?.name ?? "");
  const routines = useRoutines(chat?.kind === "dm" ? chat.bot_ids[0] : undefined);
  const outputs = useOutputs(chat?.id);
  const tasks = useDurableTasks(chat?.id);
  const reviews = useOpenReviews(chat?.id);
  const taskTint = useTaskTint();
  const [allTasks, setAllTasks] = useState(false);

  useEffect(() => setBotName(bot?.name ?? ""), [bot?.id, bot?.name]);
  // The chat's outputs, older ones the transcript has not loaded included.
  useEffect(() => {
    if (id) void engine.listOutputs(id);
  }, [id]);

  if (!chat) return null;

  /// A routine's actions, as a sheet: run it now, or delete it. The bot edits it on request.
  function showRoutine(routine: Routine) {
    alert(routine.name, `${routineDetail(routine)}\n${t("Last run: {summary}", { summary: lastRunSummary(routine) })}\n\n${routine.prompt}`, [
      { text: t("Run Now"), onPress: () => engine.runRoutine(routine.id) },
      {
        text: t("Delete"),
        style: "destructive",
        onPress: () =>
          alert(t("Delete “{name}”?", { name: routine.name }), t("This deletes the routine and stops its future runs. This can't be undone."), [
            { text: t("Cancel"), style: "cancel" },
            { text: t("Delete routine"), style: "destructive", onPress: () => engine.deleteRoutine(routine.id) },
          ]),
      },
      { text: t("Done"), style: "cancel" },
    ]);
  }
  const runner = bot ? devices.find((d) => d.id === bot.runner_id) : undefined;
  const offered = bot ? providerModels(catalog, bot.provider) : [];
  const levels = bot ? thinkingLevels(catalog, bot.provider, bot.model) : [];
  // Switching models keeps the thinking level only where the new model takes it.
  const pickModel = (bot: Bot, model: string | undefined) =>
    engine.setBotRuntime(bot.id, bot.provider, model, bot.thinking && thinkingLevels(catalog, bot.provider, model).includes(bot.thinking) ? bot.thinking : undefined);
  const defaultModel = offered[0]?.name;
  const modelValue = bot?.model
    ? offered.find((model) => model.id === bot.model)?.name ?? bot.model
    : defaultModel
      ? t("Default ({model})", { model: defaultModel })
      : t("Default");
  // The built-ins, then the account's custom providers below a divider; decision providers are
  // Auto-review's alone.
  const kinds = providerKinds(providers);
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
      alert(t("Could not update the bot"), error instanceof Error ? error.message : String(error));
    }
  }

  function confirmDelete() {
    alert(t("Delete “{name}”?", { name: chatTitle(chat!) }), t("The chat and its messages are removed from every paired Device."), [
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
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.dismiss()} />
    <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag">
      <View style={styles.hero}>
        {bot ? (
          <Pressable onPress={() => router.push(`/chat-info/look/${bot.id}`)} accessibilityLabel={t("Change {name}'s look", { name: bot.name })} accessibilityRole="button" hitSlop={8} ripple="borderless" rippleRadius={44}>
            <BotAvatar bot={bot} size={72} working={working.has(bot.id)} />
          </Pressable>
        ) : (
          <AvatarCluster bots={members} size={72} working={members.some((m) => working.has(m.id))} />
        )}
        <Text style={[styles.heroTitle, { color: p.label }]}>{chatTitle(chat)}</Text>
        {!bot ? <Text style={[styles.heroSubtitle, { color: p.secondaryLabel }]}>{members.length === 1 ? t("{count} bot", { count: members.length }) : t("{count} bots", { count: members.length })}</Text> : null}
      </View>

      {bot && (
        <Section>
          <FieldRow label={t("Name")} value={botName} onChangeText={setBotName} onBlur={() => void commitBotName()} autoCapitalize="words" returnKeyType="done" submitBehavior="blurAndSubmit" textAlign="right" />
          <Row title={t("Description")} subtitle={bot.description || undefined} subtitleLines={2} chevron onPress={() => router.push(`/chat-info/description/${bot.id}`)} />
        </Section>
      )}

      {isGroup && (
        <Section>
          <FieldRow label={t("Name")} value={title} onChangeText={setTitle} placeholder={members.map((m) => m.name).join(", ")} onBlur={commitTitle} onSubmitEditing={commitTitle} returnKeyType="done" textAlign="right" />
          <Row title={t("Description")} subtitle={chat.description || undefined} subtitleLines={2} chevron onPress={() => router.push(`/chat-info/group-description/${chat.id}`)} />
        </Section>
      )}

      {/* What the chat's bots left for review, oldest first, while any waits or runs: a row opens
          it. How each ended stays in the chat. */}
      {reviews.length > 0 && (
        <Section title={t("Waiting for review")}>
          {reviews.map((item) => {
            const host = devices.find((device) => device.id === item.runner_id);
            return (
              <Row
                key={item.id}
                title={reviewHeadline(item, host)}
                subtitle={item.rationale || undefined}
                subtitleLines={2}
                icon={reviewSymbol(item, host)}
                detail={reviewStateWord(item)}
                chevron
                onPress={() => router.push(`/chat-info/review/${item.id}`)}
              />
            );
          })}
        </Section>
      )}

      {/* The chat's durable tasks, what waits on the user first: a row opens the task. Past five
          rows the rest wait behind Show More; New Task adds one for this chat. */}
      <Section title={t("Tasks")}>
        {(allTasks || tasks.length <= 5 ? tasks : tasks.slice(0, 4)).map((task) => (
          <Row
            key={task.id}
            title={task.goal}
            subtitle={isGroup && bots.get(task.owner_bot_id) ? `${taskStateTitle(task.state)} · ${bots.get(task.owner_bot_id)!.name}` : taskStateTitle(task.state)}
            leading={<Symbol name={taskSymbol(task.state)} size={20} color={taskTint(task.state)} />}
            chevron
            onPress={() => router.push({ pathname: "/chat-info/durable-task/[id]", params: { id: task.id, chat: chat.id } })}
          />
        ))}
        {!allTasks && tasks.length > 5 ? <Row title={t("Show {count} More", { count: tasks.length - 4 })} icon="ellipsis" onPress={() => setAllTasks(true)} /> : null}
        <Row title={t("New Task")} icon="plus" onPress={() => router.push({ pathname: "/chat-info/durable-task/[id]", params: { id: "new", chat: chat.id } })} />
      </Section>

      {/* What the chat's bots published: the latest three, and View All for the rest. */}
      {outputs.length > 0 && (
        <Section title={t("Outputs")}>
          {outputs.slice(0, 3).map((series) => (
            <OutputRow key={series.id} series={series} isGroup={isGroup} onPress={() => router.push({ pathname: "/chat-info/output/[id]", params: { id: series.id, chat: chat.id } })} />
          ))}
          {outputs.length > 3 ? <Row title={t("View All")} detail={String(outputs.length)} chevron onPress={() => router.push(`/chat-info/outputs/${chat.id}`)} /> : null}
        </Section>
      )}

      {bot && (
        <Section title={t("Runs with")}>
          <Row
            title={t("Provider")}
            menu={{
              title: t("Provider"),
              value: providerLabel(bot.provider, providers),
              choices: kinds.map((kind, index) => ({
                title: providerLabel(kind, providers),
                selected: kind === bot.provider,
                onPress: () => {
                  if (kind !== bot.provider) engine.setBotRuntime(bot.id, kind, undefined, undefined);
                },
                dividerAfter: index === PROVIDER_KINDS.length - 1 && kinds.length > PROVIDER_KINDS.length,
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
                  onPress: () => pickModel(bot, undefined),
                  dividerAfter: true,
                },
                ...offered.map((model) => ({
                  title: model.name,
                  selected: bot.model === model.id,
                  onPress: () => pickModel(bot, model.id),
                })),
              ],
            }}
          />
          {/* Only the levels this model takes; a model without any has no choice to make. */}
          {levels.length > 0 && (
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
                  ...levels.map((level) => ({
                    title: thinkingLabel(level),
                    selected: bot.thinking === level,
                    onPress: () => engine.setBotRuntime(bot.id, bot.provider, bot.model, level),
                  })),
                ],
              }}
            />
          )}
        </Section>
      )}

      {bot && runner && (
        <Section title={t("Runs on")}>
          <Row
            title={runner.name}
            subtitle={`${runner.model} · ${deviceIsOnline(runner.id) ? t("Online") : lastSeen(seen[runner.id])}`}
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
              accessory={
                <Switch
                  value={routine.is_enabled}
                  onValueChange={(v) => engine.setRoutineEnabled(routine.id, v)}
                  trackColor={Platform.OS === "android" ? { false: p.fill, true: p.secondaryFill } : undefined}
                  thumbColor={Platform.OS === "android" ? p.tint : undefined}
                />
              }
              onPress={() => showRoutine(routine)}
            />
          ))}
        </Section>
      )}

      {bot && runner && (
        <Section title={t("Plugins")} footer={(runner.plugins ?? []).length === 0 ? t("No plugins on {runner} yet. Add one from the desktop app, or ask {bot} to find one.", { runner: runner.name, bot: bot.name }) : t("Installed on {runner}, for {bot} and every other bot there.", { runner: runner.name, bot: bot.name })}>
          {(runner.plugins ?? []).map((plugin) => (
            // A named account (Gmail · Work) opens its own screen; its name already says which.
            plugin.account_name ? (
              <Row
                key={plugin.id}
                title={plugin.name}
                detail={pluginStateWord(plugin)}
                icon={plugin.icon || "puzzlepiece.extension"}
                chevron
                onPress={() => router.push({ pathname: "/chat-info/account/[id]", params: { id: plugin.id, runner: runner.id } })}
              />
            ) : (
              <Row
                key={plugin.id}
                title={plugin.name}
                subtitle={plugin.state === "ready" ? plugin.description : plugin.detail}
                icon={plugin.icon || "puzzlepiece.extension"}
              />
            )
          ))}
        </Section>
      )}

      {isGroup && (
        <Section title={t("Members")} footer={chat.bot_ids.length >= 6 ? t("A group holds up to six bots.") : t("Bots in a group take turns answering; @mention one to hear from it first.")}>
          {members.map((member) => {
            const removable = chat.owner_bot_id !== member.id && members.length > 1;
            const remove = () => engine.removeBot(chat.id, member.id);
            const label = t("Remove {name}", { name: member.name });
            const row = (openMenu?: () => void) => (
              <Row
                title={member.name}
                subtitle={providerLabel(member.provider, providers)}
                leading={<BotAvatar bot={member} size={36} working={working.has(member.id)} />}
                accessory={
                  chat.owner_bot_id === member.id ? (
                    <Text style={{ color: p.secondaryLabel, fontSize: 13 }}>{t("Owner")}</Text>
                  ) : removable && Platform.OS === "ios" ? (
                    <Pressable hitSlop={14} onPress={remove} accessibilityLabel={label}>
                      <Symbol name="xmark" size={14} color={p.tertiaryLabel} weight="semibold" />
                    </Pressable>
                  ) : null
                }
                onPress={() => engine.setOwner(chat.id, member.id)}
                onLongPress={openMenu}
                action={false}
              />
            );
            // On Android removing a member is the row's long-press menu, as a Material list's is.
            return removable && Platform.OS === "android" ? (
              <RemovableRow key={member.id} label={label} onRemove={remove}>
                {row}
              </RemovableRow>
            ) : (
              <View key={member.id}>{row()}</View>
            );
          })}
          {chat.bot_ids.length < 6 && candidates.length > 0 ? <Row title={adding ? t("Choose a bot") : t("Add Bot")} icon="plus" onPress={() => setAdding((a) => !a)} /> : null}
        </Section>
      )}

      {isGroup && adding && (
        <Section>
          {candidates.map((candidate) => (
            <CheckRow
              key={candidate.id}
              title={candidate.name}
              subtitle={providerLabel(candidate.provider, providers)}
              checked={false}
              indicator={false}
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

/// A member row whose long press offers Remove (Android), as a Material list item's menu does.
function RemovableRow({ label, onRemove, children }: { label: string; onRemove: () => void; children: (openMenu: () => void) => React.ReactNode }) {
  const menu = useRef<MenuComponentRef>(null);
  // The menu's host takes its size from what it is given; Android delivers no touch outside it.
  const [width, setWidth] = useState(0);
  return (
    <View onLayout={(e) => setWidth(e.nativeEvent.layout.width)}>
      <MenuView
        ref={menu}
        style={{ width }}
        actions={[{ id: "remove", title: t("Remove"), image: AndroidIcons.delete, attributes: { destructive: true } }]}
        shouldOpenOnLongPress
        onOpenMenu={haptic.longPress}
        onPressAction={({ nativeEvent }) => nativeEvent.event === "remove" && onRemove()}
      >
        <View style={{ width }} accessibilityActions={[{ name: "remove", label }]} onAccessibilityAction={(event) => event.nativeEvent.actionName === "remove" && onRemove()}>
          {children(() => menu.current?.show())}
        </View>
      </MenuView>
    </View>
  );
}
