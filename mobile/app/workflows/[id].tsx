// A workflow's setup on a Runner, slid in from its row, after the Mac's workflow page: its questions,
// the Runner and the bot, the accounts it needs, then Run Sample, the sample's replies, and its
// schedule, which stays off until the user turns it on. The core keeps the setup in the account's
// roster, so leaving the screen keeps it too.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useMemo, useRef, useState } from "react";
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from "react-native";
import { isRunner } from "../../src/core/model";
import { deviceIsOnline, useStore } from "../../src/core/store";
import { workflow } from "../../src/core/workflows";
import { t, tc, useLanguage } from "../../src/i18n";
import { alert } from "../../src/ui/alert";
import { listenSummary, serviceName } from "../../src/ui/channels";
import { FieldRow, Row, Section, type MenuChoice } from "../../src/ui/forms";
import { scheduleText } from "../../src/ui/format";
import { usePaneWidth } from "../../src/ui/layout";
import { Markdown } from "../../src/ui/Markdown";
import { PackTile } from "../../src/ui/PackTile";
import { Font, usePalette } from "../../src/ui/theme";
import { accountOf, accountRow, configureParams, draftOf, nextStep, sampleText, type WorkflowDraft, type WorkflowProgress } from "../../src/ui/workflows";

export default function WorkflowScreen() {
  useLanguage();
  const { id: packId } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const width = usePaneWidth();
  const devices = useStore((s) => s.devices);
  const runners = useMemo(() => devices.filter(isRunner), [devices]);
  const [runnerId, setRunnerId] = useState(() => runners.find((r) => deviceIsOnline(r.id))?.id ?? runners[0]?.id ?? "");
  const runner = runners.find((r) => r.id === runnerId);
  const [progress, setProgress] = useState<WorkflowProgress>();
  const [draft, setDraft] = useState<WorkflowDraft>({ answers: {}, bots: {} });
  const [busy, setBusy] = useState(false);
  const [attempt, setAttempt] = useState(0);
  // What the roster and turns carry: a change may move the setup, so it is read again.
  const bots = useStore((s) => s.bots);
  const routines = useStore((s) => s.routines);
  const running = useStore((s) => s.running);
  const busyRef = useRef(false);

  /// Shows the setup as the core answered it, keeping what the user typed and picked unless `fresh`.
  function apply(next: WorkflowProgress, fresh = false) {
    setProgress(next);
    setDraft((current) => {
      const saved = draftOf(next);
      return fresh ? saved : { answers: { ...saved.answers, ...current.answers }, bots: { ...saved.bots, ...current.bots } };
    });
  }

  /// One request, or a few in a row, while the screen's actions wait.
  async function perform(work: () => Promise<WorkflowProgress>, then?: (done: WorkflowProgress) => void, fresh = false) {
    if (busyRef.current) return;
    busyRef.current = true;
    setBusy(true);
    try {
      const done = await work();
      apply(done, fresh);
      then?.(done);
    } catch (error) {
      alert(t("Couldn't change the workflow"), error instanceof Error ? error.message : String(error));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  // Each Runner has its own setup of the pack; picking another opens that one.
  useEffect(() => {
    if (!runnerId) return;
    setDraft((current) => ({ ...current, bots: {} }));
    void perform(() => workflow("start", { pack_id: packId, runner_id: runnerId }), undefined);
  }, [packId, runnerId, attempt]);

  useEffect(() => {
    if (!progress || busyRef.current) return;
    let live = true;
    workflow("get", { id: progress.setup.id }).then(
      (next) => live && !busyRef.current && next.setup.id === progress.setup.id && apply(next),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [bots, routines, running, devices]);

  const pack = progress?.setup.pack;
  const step = progress ? nextStep(progress, draft) : undefined;
  const setup = progress?.setup;
  const locked = !progress || setup?.phase === "enabled" || progress.is_running || busy;
  const sampleBot = progress?.specialists.flatMap((s) => s.choices).find((b) => b.id === setup?.sample?.bot_id)?.name ?? "";
  const chatId = setup?.sample?.chat_id;

  const runSample = () =>
    progress &&
    void perform(
      async () => {
        await workflow("configure", configureParams(progress, draft));
        return workflow("sample", { id: progress.setup.id });
      },
      undefined,
      // Saved now: a new bot is a bot of its own, and the menus show what setup holds.
      true,
    );
  const turnOn = () =>
    progress?.setup.sample &&
    void perform(
      async () => {
        const { id, sample } = progress.setup;
        if (sample && sample.state !== "reviewed") await workflow("review", { id, job_id: sample.job_id });
        return workflow("enable", { id });
      },
      (done) => (done.setup.sample ? router.dismissTo(`/chat/${done.setup.sample.chat_id}`) : router.dismiss()),
    );
  const cancel = () => progress && void perform(() => workflow("cancel", { id: progress.setup.id }), () => router.back());
  const choose = (serviceId: string, accountId: string) =>
    progress && void perform(() => workflow("connection", { id: progress.setup.id, service_id: serviceId, plugin_id: accountId }));

  const runnerChoices: MenuChoice[] = runners.map((r) => ({ title: r.name, selected: r.id === runnerId, onPress: () => setRunnerId(r.id) }));

  return (
    <>
      <Stack.Screen options={{ title: pack?.name ?? "" }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        {pack && (
          <View style={styles.hero}>
            <PackTile symbol={pack.symbol_name} size={64} />
            <Text style={[styles.outcome, { color: p.secondaryLabel }]}>{pack.outcome}</Text>
          </View>
        )}
        {!progress && (
          <Section footer={runners.length ? undefined : t("Pair a computer running macOS, Linux, or Windows first. Phones never run bots.")}>
            <Row title={busy ? t("Loading…") : t("Try Again")} action={!busy} onPress={busy || !runnerId ? undefined : () => setAttempt((n) => n + 1)} />
          </Section>
        )}
        {progress && setup && (
          <>
            <Section title={tc("Setup", "workflow questions")}>
              {runners.length > 1 ? (
                <Row title={t("Runs on")} menu={{ value: runner?.name ?? "", title: t("Runs on"), choices: runnerChoices }} />
              ) : (
                <Row title={t("Runs on")} detail={runner?.name} />
              )}
              {setup.pack.questions.map((q) => (
                <FieldRow
                  key={q.id}
                  label={q.label}
                  value={draft.answers[q.id] ?? ""}
                  onChangeText={(text) => setDraft((d) => ({ ...d, answers: { ...d.answers, [q.id]: text } }))}
                  placeholder={q.placeholder}
                  editable={!locked}
                  autoCapitalize="none"
                  autoCorrect={false}
                />
              ))}
              {progress.specialists.map((specialist) => {
                const picked = draft.bots[specialist.id] ?? "";
                // The new bot is a choice only while setup would add one; once it has, it is a bot.
                const choices: MenuChoice[] = [
                  ...(specialist.selected_id ? [] : [{ title: t("{name} (new)", { name: specialist.name }), selected: picked === "", onPress: () => setDraft((d) => ({ ...d, bots: { ...d.bots, [specialist.id]: "" } })), dividerAfter: specialist.choices.length > 0 }]),
                  ...specialist.choices.map((bot) => ({ title: bot.name, selected: picked === bot.id, onPress: () => setDraft((d) => ({ ...d, bots: { ...d.bots, [specialist.id]: bot.id } })) })),
                ];
                const value = picked ? (specialist.choices.find((b) => b.id === picked)?.name ?? "") : t("{name} (new)", { name: specialist.name });
                const title = progress.specialists.length === 1 ? t("Bot") : specialist.name;
                return locked ? <Row key={specialist.id} title={title} detail={value} /> : <Row key={specialist.id} title={title} menu={{ value, title, choices }} />;
              })}
              {/* An account of several is the user's to pick; one is simply used. */}
              {progress.connections
                .filter((connection) => connection.choices.length > 1)
                .map((connection) => {
                  const account = accountOf(connection);
                  const value = account ? (account.account_name ?? account.name) : t("Choose…");
                  const choices = connection.choices.map((choice) => ({ title: choice.account_name ?? choice.name, selected: choice.id === account?.id, onPress: () => choose(connection.service_id, choice.id) }));
                  return locked ? (
                    <Row key={connection.service_id} title={connection.name} detail={value} />
                  ) : (
                    <Row key={connection.service_id} title={connection.name} menu={{ value, title: connection.name, choices }} />
                  );
                })}
            </Section>
            {progress.connections.length > 0 && (
              <Section title={t("Accounts")} footer={progress.connections.some((c) => accountRow(c).needsComputer) ? t("Add the accounts it needs in Lorca on a computer.") : undefined}>
                {progress.connections.map((connection) => {
                  const account = accountOf(connection);
                  const row = accountRow(connection);
                  return (
                    <Row
                      key={connection.service_id}
                      title={connection.name}
                      subtitle={account?.account_name}
                      icon={account?.icon ?? "puzzlepiece.extension"}
                      detail={row.detail}
                      chevron={row.opens}
                      onPress={
                        row.opens && account
                          ? () => router.push({ pathname: "/workflows/account/[id]", params: { id: account.id, runner: runnerId } })
                          : !connection.available && !account
                            ? () => alert(t("Not available"), t("{name} isn't in the marketplace yet. This setup waits for it.", { name: connection.name }))
                            : undefined
                      }
                    />
                  );
                })}
              </Section>
            )}
            {step?.kind === "run" && (
              <Section footer={step.disabled || undefined}>
                <Row title={t("Run Sample")} action disabled={step.disabled !== undefined || busy} onPress={runSample} />
              </Section>
            )}
            {setup.sample && (
              <Section title={t("Sample")}>
                {progress.is_running ? (
                  <Row title={t("{name} is working on it…", { name: sampleBot })} leading={<ActivityIndicator />} />
                ) : (setup.sample.state === "ready" || setup.sample.state === "reviewed") && progress.sample_messages.length > 0 ? (
                  <View style={styles.sample}>
                    <Markdown text={sampleText(progress)} color={p.label} maxWidth={Math.min(width, 620) - 64} size={Font.body} />
                  </View>
                ) : (
                  <View style={styles.sample}>
                    <Text style={[styles.note, { color: p.secondaryLabel }]}>{t("The sample didn't finish. {name}'s chat says what happened.", { name: sampleBot })}</Text>
                  </View>
                )}
              </Section>
            )}
            {(progress.routines.length > 0 || (progress.channels ?? []).length > 0) && (
              <Section title={t("Schedule")}>
                {progress.routines.map((routine) => (
                  <Row key={routine.id} title={routine.name} subtitle={scheduleText(routine.schedule_text)} detail={routine.is_enabled ? t("On") : t("Off")} />
                ))}
                {(progress.channels ?? []).map((channel) => (
                  <Row
                    key={channel.id}
                    title={channel.name}
                    subtitle={`${serviceName(channel.service_id)} · ${listenSummary(channel.listen)}`}
                    detail={channel.channel && channel.channel.state !== "paused" ? t("On") : t("Off")}
                  />
                ))}
                {step?.kind === "decide" && <Row title={t("Turn On Schedule")} action disabled={busy} onPress={turnOn} />}
                {step?.kind === "decide" && chatId && <Row title={t("Not Now")} action disabled={busy} onPress={() => router.dismissTo(`/chat/${chatId}`)} />}
              </Section>
            )}
            {/* Apart from the rest: Cancel Setup, or Turn Off once it is on. */}
            {(Object.keys(setup.bot_ids).length > 0 || setup.sample) && (
              <Section>
                <Row title={setup.phase === "enabled" ? t("Turn Off Workflow") : t("Cancel Setup")} destructive disabled={busy} onPress={cancel} />
              </Section>
            )}
          </>
        )}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  hero: { alignItems: "center", gap: 12, paddingTop: 8, paddingHorizontal: 32 },
  outcome: { fontSize: Font.body, textAlign: "center", lineHeight: 21 },
  sample: { paddingHorizontal: 16, paddingVertical: 12 },
  note: { fontSize: Font.small, lineHeight: 18 },
});
