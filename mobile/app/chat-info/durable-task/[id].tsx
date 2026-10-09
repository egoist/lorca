// One durable task, slid in from the chat's details, after the desktop apps' task sheet: where it
// stands, why when it is blocked or cancelled, its result and what supports it, the one step the
// state allows (Start, Resume, Mark Complete, Reopen), and its Limits; then the goal, owner, next
// step, and what done looks like, which Save writes back, and what it waits for and links to.
// `new` as the id is the same form for a new task in the chat; once created, the screen shows it.
//
// Edits start from the version the screen showed. A change from elsewhere replaces an untouched
// form. When a save meets a newer revision, the screen takes it under the user's edits and says
// so; the next Save writes them. (Not the Running tasks of `app/tasks`: those are commands.)

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useRef, useState } from "react";
import { Platform, PlatformColor, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../../src/core/engine";
import { CANCELLED_BY_USER, taskCanStart, taskIsFinished, taskSymbol, type DurableTask, type TaskEvidence } from "../../../src/core/model";
import { useBotMap, useBudget, useDurableTask, useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { taskStateTitle, useTaskTint } from "../../../src/ui/durableTasks";
import { daySeparator } from "../../../src/ui/format";
import { FieldRow, Row, Section } from "../../../src/ui/forms";
import { SaveToolbar } from "../../../src/ui/navigation";
import { openLink } from "../../../src/ui/outputs";
import { Symbol } from "../../../src/ui/Symbol";
import { accentColor, Font, usePalette } from "../../../src/ui/theme";
import { isStopped, limitsSummary, stoppedLabel } from "../../../src/ui/limits";

interface Form {
  goal: string;
  next: string;
  criteria: string;
  owner: string;
}

const lines = (text: string) =>
  text
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);

const formOf = (task: DurableTask): Form => ({ goal: task.goal, next: task.next_action, criteria: task.acceptance_criteria.join("\n"), owner: task.owner_bot_id });

/// The fields of `form` that differ from `task`, as `tasks.update` takes them.
function changes(form: Form, task: DurableTask): Record<string, unknown> {
  const params: Record<string, unknown> = {};
  if (form.goal.trim() !== task.goal.trim()) params.goal = form.goal.trim();
  if (form.next.trim() !== task.next_action.trim()) params.next_action = form.next.trim();
  const criteria = lines(form.criteria);
  if (criteria.join("\n") !== task.acceptance_criteria.join("\n")) params.acceptance_criteria = criteria;
  if (form.owner && form.owner !== task.owner_bot_id) params.owner_bot_id = form.owner;
  return params;
}

export default function DurableTaskScreen() {
  useLanguage();
  const { id, chat: chatId } = useLocalSearchParams<{ id: string; chat: string }>();
  const router = useRouter();
  const p = usePalette();
  const tint = useTaskTint();
  const bots = useBotMap();
  const chats = useStore((s) => s.chats);
  const tasks = useStore((s) => s.tasks);
  const [createdId, setCreatedId] = useState<string>();
  const latest = useDurableTask(createdId ?? (id === "new" ? undefined : id));
  // The version the form's edits start from.
  const [base, setBase] = useState<DurableTask | undefined>(latest);
  const chat = chats.find((each) => each.id === chatId);
  const [form, setForm] = useState<Form>(() =>
    latest ? formOf(latest) : { goal: "", next: "", criteria: "", owner: chat?.owner_bot_id ?? chat?.bot_ids[0] ?? "" },
  );
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string>();
  // What the task's runs may use; a task its limits stopped resumes there.
  const limits = useBudget("task", latest?.id, latest?.runner_id);
  const stopped = isStopped(limits) ? limits : undefined;
  const orange = Platform.OS === "ios" ? PlatformColor("systemOrange") : accentColor("orange", p.dark);
  // A request and its id: a retry after an unclear failure sends the same id, so the core answers it once.
  const sent = useRef<{ body: string; id: string } | undefined>(undefined);

  const edited = !!base && Object.keys(changes(form, base)).length > 0;
  useEffect(() => {
    if (latest && base && latest.revision > base.revision && !edited && !busy) {
      setBase(latest);
      setForm(formOf(latest));
    }
  }, [latest?.revision]);

  const ownerIds = new Set((base?.chat_ids ?? [chatId]).flatMap((each) => chats.find((c) => c.id === each)?.bot_ids ?? []));
  const owners = [...bots.values()].filter((bot) => ownerIds.has(bot.id));
  const canSave = !busy && !!form.owner && !!form.goal.trim() && !!form.next.trim() && lines(form.criteria).length > 0;
  const task = latest ?? base;
  const running = !!task?.active_run;

  async function send(method: string, params: Record<string, unknown>): Promise<DurableTask | undefined> {
    const body = JSON.stringify({ method, params });
    if (sent.current?.body !== body) sent.current = { body, id: Math.random().toString(36).slice(2) + Date.now().toString(36) };
    setBusy(true);
    setNote(undefined);
    try {
      const answer = await engine.taskRequest(method, { ...params, request_id: sent.current.id });
      sent.current = undefined;
      return answer;
    } catch (error) {
      const text = error instanceof Error ? error.message : String(error);
      if (text.startsWith("Task revision conflict") && base) {
        // Take the newer version under the user's edits: a field they changed keeps their words.
        const newer = await engine.taskRequest("tasks.get", { id: base.id, refresh: true }).catch(() => undefined);
        if (newer) {
          const before = formOf(base);
          setForm((now) => ({
            goal: now.goal === before.goal ? newer.goal : now.goal,
            next: now.next === before.next ? newer.next_action : now.next,
            criteria: now.criteria === before.criteria ? newer.acceptance_criteria.join("\n") : now.criteria,
            owner: now.owner === before.owner ? newer.owner_bot_id : now.owner,
          }));
          setBase(newer);
        }
        setNote(t("This task changed since you opened it. Your edits are still here; save again to keep them."));
      } else {
        alert(t("Could not save the task"), text);
      }
      return undefined;
    } finally {
      setBusy(false);
    }
  }

  /// Writes the form's edits, and `params` with them, over the version they start from.
  const update = (params: Record<string, unknown>) =>
    base ? send("tasks.update", { ...changes(form, base), ...params, id: base.id, expected_revision: base.revision }) : Promise.resolve(undefined);

  async function save() {
    if (!canSave) return;
    if (!base) {
      const created = await send("tasks.create", {
        owner_bot_id: form.owner,
        goal: form.goal.trim(),
        next_action: form.next.trim(),
        acceptance_criteria: lines(form.criteria),
        chat_ids: [chatId],
      });
      if (created) {
        setCreatedId(created.id);
        setBase(created);
        setForm(formOf(created));
      }
      return;
    }
    if (!edited) return router.back();
    if (await update({})) router.back();
  }

  /// Saves what the user changed, then starts a run in this chat when the owner is in it, or in
  /// the first of the task's chats it is in, and goes back to the chat's details.
  function openLimits(task: DurableTask) {
    router.push({ pathname: "/chat-info/limits", params: { kind: "task", id: task.id, bot: task.owner_bot_id } });
  }

  async function start() {
    if (!base) return;
    const saved = edited ? await update({}) : base;
    if (!saved) return;
    const inChat = chat?.bot_ids.includes(saved.owner_bot_id);
    if (await send("tasks.run", { id: saved.id, expected_revision: saved.revision, ...(inChat ? { chat_id: chatId } : {}) })) router.back();
  }

  async function step(state: "completed" | "queued") {
    const answer = await update({ state });
    if (!answer) return;
    if (state === "completed") router.back();
    else {
      setBase(answer);
      setForm(formOf(answer));
    }
  }

  function confirmCancel() {
    const owner = task && bots.get(task.owner_bot_id);
    alert(t("Cancel this task?"), running && owner ? t("{name} stops working on it. You can reopen it later.", { name: owner.name }) : t("You can reopen it later."), [
      { text: t("Keep Task"), style: "cancel" },
      {
        text: t("Cancel Task"),
        style: "destructive",
        onPress: async () => {
          if (await update({ state: "cancelled", reason: CANCELLED_BY_USER })) router.back();
        },
      },
    ]);
  }

  // Where the task stands, and the one step its state allows.
  let detail = "";
  let action: { title: string; icon: string; run: () => void } | undefined;
  if (task) {
    switch (task.state) {
      case "queued":
        action = { title: t("Start"), icon: "play", run: () => void start() };
        break;
      case "blocked":
        detail = task.reason ?? "";
        // A run would only be refused again while the task's limits stopped it.
        action = { title: t("Resume"), icon: "play", run: stopped ? () => openLimits(task) : () => void start() };
        break;
      case "working": {
        const owner = bots.get(task.owner_bot_id);
        if (task.active_run && owner) detail = t("{name} is working on it.", { name: owner.name });
        break;
      }
      case "awaiting_review":
        if (task.result && task.evidence.length > 0) action = { title: t("Mark Complete"), icon: "checkmark", run: () => void step("completed") };
        break;
      case "completed":
        action = { title: t("Reopen"), icon: "arrow.counterclockwise", run: () => void step("queued") };
        break;
      case "cancelled":
        detail = task.reason === CANCELLED_BY_USER ? "" : task.reason ?? "";
        action = { title: t("Reopen"), icon: "arrow.counterclockwise", run: () => void step("queued") };
        break;
    }
    if (taskCanStart(task) && task.dependencies.some((each) => tasks.find((other) => other.id === each)?.state !== "completed")) {
      detail = t("It starts once the tasks it waits for are completed.");
      action = undefined;
    }
  }

  return (
    <>
      <Stack.Screen options={{ title: task ? t("Task") : t("New Task") }} />
      <SaveToolbar label={task ? t("Save") : t("Create")} disabled={!canSave} onSave={() => void save()} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        {task && (
          <Section>
            <Row title={taskStateTitle(task.state)} subtitle={detail || undefined} subtitleLines={6} leading={<Symbol name={taskSymbol(task.state)} size={20} color={tint(task.state)} />} />
            {task.result ? (
              <View style={styles.result}>
                <Text style={[styles.resultText, { color: p.label }]} selectable numberOfLines={12}>
                  {task.result}
                </Text>
              </View>
            ) : null}
            {task.evidence.map((item, index) => (
              <EvidenceRow key={index} item={item} />
            ))}
            {action && !busy ? <Row title={action.title} icon={action.icon} onPress={action.run} /> : null}
            <Row
              title={t("Limits")}
              detail={stopped ? undefined : limitsSummary(limits?.limits)}
              accessory={stopped ? <Text style={{ color: orange, fontSize: Font.body }}>{stoppedLabel(stopped)}</Text> : undefined}
              chevron
              onPress={() => openLimits(task)}
            />
          </Section>
        )}

        <Section footer={note ?? (task ? undefined : t("A task stays with its bot across turns, until it’s done."))}>
          <FieldRow label={t("Goal")} value={form.goal} onChangeText={(goal) => setForm((f) => ({ ...f, goal }))} placeholder={t("What should get done")} autoCapitalize="sentences" />
          {owners.length > 1 ? (
            running ? (
              <Row title={t("Owner")} detail={bots.get(form.owner)?.name} />
            ) : (
              <Row
                title={t("Owner")}
                menu={{
                  title: t("Owner"),
                  value: bots.get(form.owner)?.name ?? "",
                  choices: owners.map((bot) => ({ title: bot.name, selected: bot.id === form.owner, onPress: () => setForm((f) => ({ ...f, owner: bot.id })) })),
                }}
              />
            )
          ) : null}
          <FieldRow label={t("Next step")} value={form.next} onChangeText={(next) => setForm((f) => ({ ...f, next }))} placeholder={t("What the bot does first")} autoCapitalize="sentences" />
          <FieldRow label={t("Done when")} value={form.criteria} onChangeText={(criteria) => setForm((f) => ({ ...f, criteria }))} placeholder={t("One check per line")} multiline editable={!running} autoCapitalize="sentences" />
        </Section>

        {task && task.dependencies.length > 0 && (
          <Section title={t("Waits For")}>
            {task.dependencies.map((each) => {
              const dependency = tasks.find((other) => other.id === each);
              return (
                <Row
                  key={each}
                  title={dependency?.goal ?? t("A task on another Device")}
                  subtitle={dependency ? taskStateTitle(dependency.state) : undefined}
                  leading={<Symbol name={dependency ? taskSymbol(dependency.state) : "circle"} size={20} color={dependency ? tint(dependency.state) : p.tertiaryLabel} />}
                />
              );
            })}
          </Section>
        )}

        {task && task.links.length > 0 && (
          <Section title={t("Links")}>
            {task.links.map((link) => (
              <Row key={link.url} title={link.label} subtitle={link.label === link.url ? undefined : hostOf(link.url)} icon="link" chevron onPress={() => openLink(link.url)} />
            ))}
          </Section>
        )}

        {task && !taskIsFinished(task.state) && (
          <Section>
            <Row title={t("Cancel Task")} icon="xmark.circle" destructive onPress={confirmCancel} />
          </Section>
        )}
      </ScrollView>
    </>
  );
}

const hostOf = (url: string) => {
  try {
    return new URL(url).host;
  } catch {
    return undefined;
  }
};

/// A message by who wrote it and when; an output by its name and version, which opens that
/// version; a link by where it goes; the core's label otherwise.
function EvidenceRow({ item }: { item: TaskEvidence }) {
  const router = useRouter();
  const p = usePalette();
  const bots = useBotMap();
  const message = useStore((s) => (item.chat_id ? s.chats.find((c) => c.id === item.chat_id)?.messages.find((m) => m.id === item.message_id) : undefined));
  const icon = <Symbol name={{ output: "doc.richtext", url: "link", file: "doc.text", review: "checkmark.circle", message: "bubble.left" }[item.kind]} size={20} color={p.secondaryLabel} />;
  switch (item.kind) {
    case "output":
      return (
        <Row
          title={item.label}
          subtitle={item.version ? t("Version {number}", { number: item.version }) : undefined}
          leading={icon}
          chevron
          onPress={() => {
            if (item.output_id && item.chat_id) router.push({ pathname: "/chat-info/output/[id]", params: { id: item.output_id, chat: item.chat_id, version: item.message_id ?? "" } });
          }}
        />
      );
    case "url":
      return <Row title={item.label} subtitle={item.url ? hostOf(item.url) : undefined} leading={icon} chevron onPress={() => item.url && openLink(item.url)} />;
    case "message": {
      const author = message?.author;
      const title = !message
        ? t("Message")
        : author?.kind === "you"
          ? t("Your message")
          : author?.kind === "bot"
            ? t("Message from {name}", { name: bots.get(author.bot_id)?.name ?? t("A bot") })
            : t("Message");
      return <Row title={title} subtitle={message ? daySeparator(new Date(message.created_at * 1000)) : undefined} leading={icon} />;
    }
    default:
      return <Row title={item.label} leading={icon} />;
  }
}

const styles = StyleSheet.create({
  // On the titles' column: the row's 16 inset, the 20 symbol, and the 12 gap after it.
  result: { paddingVertical: 11, paddingLeft: 48, paddingRight: 16 },
  resultText: { fontSize: 15, lineHeight: 20 },
});
