// One routine, slid in from its row in the chat's Details, after the Mac app's routine sheet: how
// it stands, with what happened and how to fix it when something went wrong, Run Now, Pause or
// Resume, and its Limits; the schedule with its timezone when the phone keeps other hours, the
// next run, what happens to runs its Runner missed, and its last check and run; the task, the
// check, and Delete. The bot owns the routine: the user asks it in chat to change one.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { Platform, PlatformColor, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../../src/core/engine";
import { deviceName } from "../../../src/core/model";
import { useBotMap, useBudget, useRoutines, useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { daySeparator, scheduleText } from "../../../src/ui/format";
import { Row, Section } from "../../../src/ui/forms";
import { isStopped, limitsSummary, stoppedDetail, stoppedLabel } from "../../../src/ui/limits";
import { checkIsFailing, lastCheck, lastRun, missedRuns, nextRunText, problemExplanation, problemNeedsUser, problemWord, routineProblem, timezoneLabel } from "../../../src/ui/routines";
import { Symbol } from "../../../src/ui/Symbol";
import { accentColor, Font, usePalette } from "../../../src/ui/theme";

export default function RoutineScreen() {
  useLanguage();
  const { id, bot: botId } = useLocalSearchParams<{ id: string; bot: string }>();
  const router = useRouter();
  const p = usePalette();
  const bot = useBotMap().get(botId);
  // The bot's list carries the running state the core's turns add.
  const routine = useRoutines(botId).find((r) => r.id === id);
  const runner = useStore((s) => s.devices.find((d) => d.id === bot?.runner_id));
  const limits = useBudget("routine", routine?.id, bot?.runner_id);
  const orange = Platform.OS === "ios" ? PlatformColor("systemOrange") : accentColor("orange", p.dark);

  if (!routine || !bot) return null;
  const runnerName = runner ? deviceName(runner) : t("its Runner");
  const problem = routineProblem(routine);
  // A routine stopped at its limits runs again only once the user resumes it in Limits, which
  // comes before anything else that's wrong with it.
  const stopped = isStopped(limits) ? limits : undefined;
  const state = stopped
    ? { title: stoppedLabel(stopped), note: stoppedDetail(stopped), symbol: "exclamationmark.circle.fill", color: orange }
    : routine.is_running
      ? { title: t("Running…"), note: undefined, symbol: "arrow.triangle.2.circlepath", color: p.tint }
      : problem
        ? { title: problemWord(problem), note: problemExplanation(problem, bot.name, runnerName), symbol: problemNeedsUser(problem) ? "exclamationmark.circle.fill" : "clock", color: problemNeedsUser(problem) ? orange : p.secondaryLabel }
        : routine.is_enabled
          ? { title: t("On"), note: undefined, symbol: "clock", color: p.green }
          : { title: routine.paused_reason === "away" ? t("Paused while you were away") : t("Paused"), note: undefined, symbol: "pause.circle", color: p.secondaryLabel };
  // A run needs its Runner online, and a routine paused by failed sign-ins needs Resume.
  const canRun = !routine.is_running && routine.state !== "waiting_for_runner" && routine.paused_reason !== "authentication";
  const missed = missedRuns(routine, runnerName);
  const checked = lastCheck(routine);
  const ran = lastRun(routine);
  const lastSuccess = routine.health?.last_success_at;

  function confirmDelete() {
    alert(t("Delete “{name}”?", { name: routine!.name }), t("This deletes the routine and stops its future runs. This can't be undone."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Delete routine"),
        style: "destructive",
        onPress: () => {
          engine.deleteRoutine(routine!.id);
          router.back();
        },
      },
    ]);
  }

  return (
    <>
      <Stack.Screen options={{ title: routine.name }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content}>
        <Section>
          <Row title={state.title} subtitle={state.note} subtitleLines={6} leading={<Symbol name={state.symbol} size={20} color={state.color} />} />
          {!stopped && <Row title={t("Run Now")} icon="play" disabled={!canRun} onPress={() => engine.runRoutine(routine.id)} />}
          <Row title={routine.is_enabled ? t("Pause") : t("Resume")} icon={routine.is_enabled ? "pause" : "arrow.clockwise"} onPress={() => engine.setRoutineEnabled(routine.id, !routine.is_enabled)} />
          <Row
            title={t("Limits")}
            detail={stopped ? undefined : limitsSummary(limits?.limits)}
            accessory={stopped ? <Text style={[styles.value, { color: orange }]}>{stoppedLabel(stopped)}</Text> : undefined}
            chevron
            onPress={() => router.push({ pathname: "/chat-info/limits", params: { kind: "routine", id: routine.id, bot: bot.id } })}
          />
        </Section>

        <Section title={t("Schedule")} footer={missed.note}>
          <Row title={scheduleText(routine.schedule_text)} detail={timezoneLabel(routine)} />
          <Row title={routine.check ? t("Next check") : t("Next run")} detail={nextRunText(routine) ?? "—"} />
          <Row title={t("Missed runs")} detail={missed.value} />
          {/* The time on the right and how it went under the title, so neither is cut short. */}
          {checked ? <Row title={t("Last check")} subtitle={checked.outcome} detail={checked.when} /> : null}
          {/* Only a failing check has a success to tell apart from it. */}
          {checked && checkIsFailing(routine) ? <Row title={t("Last successful check")} detail={lastSuccess ? daySeparator(new Date(lastSuccess * 1000)) : t("Never")} /> : null}
          <Row title={t("Last run")} subtitle={ran.outcome} detail={ran.when} />
        </Section>

        <Section title={t("Task")} footer={t("Ask {name} in chat to change this routine.", { name: bot.name })}>
          <View style={styles.text}>
            <Text style={[styles.prompt, { color: p.label }]} selectable>
              {routine.prompt}
            </Text>
          </View>
        </Section>

        {routine.check ? (
          <Section title={t("Check")}>
            <View style={styles.text}>
              <Text style={[styles.code, { color: p.label }]} selectable>
                {routine.check}
              </Text>
            </View>
          </Section>
        ) : null}

        <Section>
          <Row title={t("Delete Routine")} icon="trash" destructive onPress={confirmDelete} />
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  value: { fontSize: Font.body },
  text: { paddingHorizontal: 16, paddingVertical: 11 },
  prompt: { fontSize: Font.body, lineHeight: 22 },
  code: { fontSize: 13, lineHeight: 18, fontFamily: Platform.OS === "ios" ? "Menlo" : "monospace" },
});
