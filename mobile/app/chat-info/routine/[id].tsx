// One routine, slid in from its row in the chat's Details, after the Mac app's routine sheet: how
// it stands, with what happened and how to fix it when something went wrong, Run Now, Pause or
// Resume, and its Limits; the schedule with its timezone when the phone keeps other hours, the
// next run, what happens to runs its Runner missed, and its last check and run; the task, the
// check, and Delete. A routine on events shows what it listens to and its latest event instead
// of a next run; one with a webhook has its URL, key, and header to copy, and a new key. The bot
// owns the routine: the user asks it in chat to change one.

import * as Clipboard from "expo-clipboard";
import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useEffect, useState } from "react";
import { Platform, PlatformColor, Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../../src/core/engine";
import { deviceName, type RoutineEvents } from "../../../src/core/model";
import { useBotMap, useBudget, useRoutines, useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { alert } from "../../../src/ui/alert";
import { daySeparator } from "../../../src/ui/format";
import { Row, Section } from "../../../src/ui/forms";
import { isStopped, limitsSummary, stoppedDetail, stoppedLabel } from "../../../src/ui/limits";
import { openLink } from "../../../src/ui/outputs";
import { haptic } from "../../../src/ui/haptics";
import { checkIsFailing, isWebhook, lastCheck, lastEvent, lastRun, looksFirst, missedRuns, nextRunText, problemExplanation, problemNeedsUser, problemWord, routineProblem, routineSchedule, routineSymbol, timezoneLabel } from "../../../src/ui/routines";
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
  // The webhook key shows only after a tap on its row; a row's Copy says Copied for a moment.
  const [keyShown, setKeyShown] = useState(false);
  const [copied, setCopied] = useState<string>();
  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(undefined), 1500);
    return () => clearTimeout(timer);
  }, [copied]);

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
          ? { title: t("On"), note: undefined, symbol: routineSymbol(routine), color: p.green }
          : { title: routine.paused_reason === "away" ? t("Paused while you were away") : t("Paused"), note: undefined, symbol: "pause.circle", color: p.secondaryLabel };
  // A run needs its Runner online, and a routine paused by failed sign-ins needs Resume.
  const canRun = !routine.is_running && routine.state !== "waiting_for_runner" && routine.paused_reason !== "authentication";
  const missed = missedRuns(routine, runnerName);
  const checked = lastCheck(routine);
  const ran = lastRun(routine);
  const lastSuccess = routine.health?.last_success_at;

  const events = routine.events;

  async function copy(row: string, text: string) {
    await Clipboard.setStringAsync(text);
    haptic.success();
    setCopied(row);
  }

  async function openSetup(receiver: string, subject?: string) {
    try {
      openLink(await engine.receiverSetupURL(receiver, subject));
    } catch (error) {
      alert(t("Request failed"), error instanceof Error ? error.message : String(error));
    }
  }

  function confirmRegenerate() {
    alert(t("Regenerate the webhook key?"), t("Services that send the current key stop reaching this routine until you give them the new one."), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Regenerate Key"),
        style: "destructive",
        onPress: async () => {
          try {
            await engine.regenerateRoutineKey(routine!.id);
            setKeyShown(true);
          } catch (error) {
            alert(t("Request failed"), error instanceof Error ? error.message : String(error));
          }
        },
      },
    ]);
  }

  /// A row's Copy, which says Copied for a moment after a tap.
  const copyAccessory = (row: string, text: string) => (
    <Pressable onPress={() => void copy(row, text)} hitSlop={10} accessibilityRole="button">
      <Text style={[styles.value, { color: copied === row ? p.green : p.tint }]}>{copied === row ? t("Copied") : t("Copy")}</Text>
    </Pressable>
  );

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

        {/* A one-time routine runs once its Runner is back, whatever the policy; a routine on
            events has no next run or missed runs: its events start it, held on the relay while
            its Runner is off. */}
        <Section title={t("Schedule")} footer={routine.once_at || events ? undefined : missed.note}>
          <Row title={routineSchedule(routine)} titleLines={2} detail={timezoneLabel(routine)} />
          {events ? <EventRows events={events} bot={bot.name} onSetup={() => void openSetup(events.receiver, events.subject)} /> : null}
          {/* The calendar names its account. */}
          {routine.calendar?.account ? <Row title={t("Calendar")} detail={routine.calendar.account} /> : null}
          {events ? null : <Row title={looksFirst(routine) ? t("Next check") : t("Next run")} subtitle={routine.calendar?.next_event?.title} detail={nextRunText(routine) ?? "—"} />}
          {routine.once_at || events ? null : <Row title={t("Missed runs")} detail={missed.value} />}
          {/* The time on the right and how it went under the title, so neither is cut short. */}
          {checked ? <Row title={t("Last check")} subtitle={checked.outcome} detail={checked.when} /> : null}
          {/* Only a failing check has a success to tell apart from it. */}
          {checked && checkIsFailing(routine) ? <Row title={t("Last successful check")} detail={lastSuccess ? daySeparator(new Date(lastSuccess * 1000)) : t("Never")} /> : null}
          <Row title={t("Last run")} subtitle={ran.outcome} detail={ran.when} />
        </Section>

        {/* A routine's webhook: its URL, its key (hidden until a tap shows it), and the header a
            sender pastes, each with Copy; Regenerate Key makes a new key. */}
        {events?.endpoint ? (
          <Section
            title={t("Webhook")}
            footer={t("Each request to this URL runs the routine once, with what it sent. Senders include the key in an Authorization: Bearer header; share it only with the service that calls this routine.")}
          >
            <Row title={t("Webhook URL")} subtitle={events.endpoint} subtitleLines={2} accessory={copyAccessory("url", events.endpoint)} onPress={() => void copy("url", events.endpoint!)} />
            <Row
              title={t("Webhook key")}
              subtitle={keyShown ? events.key : HIDDEN}
              subtitleLines={2}
              accessory={copyAccessory("key", events.key ?? "")}
              onPress={() => setKeyShown(!keyShown)}
            />
            <Row
              title={t("Authorization header")}
              subtitle={`Bearer ${keyShown ? events.key : HIDDEN}`}
              subtitleLines={2}
              accessory={copyAccessory("header", `Authorization: Bearer ${events.key ?? ""}`)}
              onPress={() => void copy("header", `Authorization: Bearer ${events.key ?? ""}`)}
            />
            <Row title={t("Regenerate Key…")} icon="arrow.clockwise" onPress={confirmRegenerate} />
          </Section>
        ) : null}

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

const HIDDEN = "••••••••••••";

/// What a routine on events listens to: its pull request, which opens on GitHub; how it hears the
/// service, where an App that isn't installed yet installs from; and the latest event.
function EventRows({ events, bot, onSetup }: { events: RoutineEvents; bot: string; onSetup: () => void }) {
  const p = usePalette();
  const orange = Platform.OS === "ios" ? PlatformColor("systemOrange") : accentColor("orange", p.dark);
  const gitHub = events.receiver === "github";
  const service = gitHub ? "GitHub" : events.source_name || events.receiver;
  const url = events.url;
  const last = lastEvent(events);
  let status: { detail?: string; accessory?: React.ReactNode; note?: string } | undefined;
  if (events.status === "needs_setup") status = { accessory: <Text style={[styles.value, { color: orange }]}>{gitHub ? t("Install App…") : t("Set Up…")}</Text> };
  else if (events.status === "gateway") status = { detail: t("Your gateway"), note: t("This relay doesn’t take {service}’s events itself, so they come through a gateway you run. {bot} said how to set it up in the chat.", { service, bot }) };
  else if (events.status === "pending" && (!isWebhook(events) || !events.endpoint)) status = { detail: t("Connecting…") };
  else if (!isWebhook(events)) status = { detail: t("Connected") };
  return (
    <>
      {events.subject ? (
        <Row title={gitHub ? t("Pull request") : (events.source_name ?? "")} detail={events.title || events.subject} action={false} onPress={url ? () => openLink(url) : undefined} />
      ) : null}
      {status ? (
        <Row
          title={service}
          subtitle={status.note}
          subtitleLines={4}
          detail={status.detail}
          accessory={status.accessory}
          onPress={events.status === "needs_setup" ? onSetup : undefined}
        />
      ) : null}
      {/* The time on the right and what came under the title, as the last run reads. */}
      <Row title={t("Last event")} subtitle={last.summary} detail={last.when} />
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
