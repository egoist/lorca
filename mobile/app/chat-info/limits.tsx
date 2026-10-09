// What a DM's turns, a task's runs, or a routine's runs may use on the bot's Runner, pushed from
// the Limits row of Details, a task, or a routine, after the Mac's Limits sheet: work that stopped
// at a limit first, with Resume; the limits, empty for none; then what the work used of them.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { Platform, PlatformColor, ScrollView, StyleSheet, Text } from "react-native";
import { engine } from "../../src/core/engine";
import type { BudgetLimits, BudgetState } from "../../src/core/model";
import { useBotMap, useBudget, useStoppedTurn, useStore } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { alert } from "../../src/ui/alert";
import { FieldRow, Row, Section } from "../../src/ui/forms";
import { count, fieldsOf, fits, isStopped, limitsOf, minutes, spend, stoppedDetail, stoppedTitle, type LimitFields } from "../../src/ui/limits";
import { SaveToolbar } from "../../src/ui/navigation";
import { Symbol } from "../../src/ui/Symbol";
import { accentColor, Font, usePalette } from "../../src/ui/theme";

export default function LimitsScreen() {
  useLanguage();
  const { kind, id, bot: botId } = useLocalSearchParams<{ kind: "chat" | "task" | "routine"; id: string; bot: string }>();
  const router = useRouter();
  const p = usePalette();
  // Work that stopped is the user's to act on.
  const orange = Platform.OS === "ios" ? PlatformColor("systemOrange") : accentColor("orange", p.dark);
  const bot = useBotMap().get(botId);
  const routine = useStore((s) => (kind === "routine" ? s.routines.find((r) => r.id === id) : undefined));
  const task = useStore((s) => (kind === "task" ? s.tasks.find((each) => each.id === id) : undefined));
  const configured = useBudget(kind, id, bot?.runner_id);
  const turn = useStoppedTurn(kind === "chat" ? id : undefined, bot?.runner_id);
  const stopped = kind === "chat" ? turn : isStopped(configured) ? configured : undefined;
  const usage = kind === "chat" ? stopped?.usage : configured?.usage;
  const [fields, setFields] = useState<LimitFields>(() => fieldsOf(configured?.limits));
  const [busy, setBusy] = useState(false);
  const dm = useStore((s) => s.chats.find((c) => c.kind === "dm" && c.bot_ids[0] === botId)?.id);

  if (!bot) return null;
  // The DM a turn runs in, the task's first chat, or the routine's DM.
  const chatId = kind === "chat" ? id : kind === "task" ? (task?.chat_ids[0] ?? "") : (dm ?? "");
  const footer =
    kind === "routine"
      ? t("All runs of {name} count toward these. When it reaches one, it waits until you resume it.", { name: routine?.name ?? "" })
      : kind === "task"
        ? t("All runs of this task count toward these. When it reaches one, the task waits until you resume it.")
        : t("Each turn with {name} stops when it reaches one of these.", { name: bot.name });

  function read(): BudgetLimits | undefined {
    const result = limitsOf(fields);
    if ("limits" in result) return result.limits;
    alert(result.message);
    return undefined;
  }

  async function run(change: () => Promise<void>) {
    setBusy(true);
    try {
      await change();
      router.back();
    } catch (error) {
      setBusy(false);
      alert(t("Couldn't change the limits"), error instanceof Error ? error.message : String(error));
    }
  }

  const save = (limits: BudgetLimits) => engine.setBudget(kind, id, limits, bot, chatId);

  /// Goes on where it stopped. The stopped turn takes the form's limits too, so raising the one it
  /// reached lets it go on; otherwise resuming grants the limits again in full, which asks first.
  function resume(stopped: BudgetState) {
    const limits = read();
    if (!limits || !bot) return;
    const go = (fresh: boolean) =>
      void run(async () => {
        await save(limits);
        if (stopped.kind === "job") await engine.setBudget("job", stopped.id, limits, bot, chatId);
        await engine.resumeBudget(stopped.kind, stopped.id, bot.runner_id, fresh);
      });
    if (fits(stopped, limits)) return go(false);
    const message = kind === "routine" ? t("Resume this routine with fresh limits?") : kind === "task" ? t("Resume this task with fresh limits?") : t("Resume this turn with fresh limits?");
    alert(message, t("It already used its limits. Resuming lets it use them again in full."), [
      { text: t("Cancel"), style: "cancel" },
      { text: t("Resume"), onPress: () => go(true) },
    ]);
  }

  const used: [string, string, boolean][] = usage
    ? ([
        [t("Spending"), usage.api_cost_usd > 0 || usage.subscription_estimate_usd > 0 ? spend(usage.api_cost_usd, usage.subscription_estimate_usd) : usage.unknown_price_calls > 0 ? t("Price unknown") : "", stopped?.reached === "usd"],
        [t("Tokens"), usage.tokens > 0 ? count(usage.tokens) : "", stopped?.reached === "tokens"],
        [t("Run time"), usage.runtime_secs >= 1 ? minutes(usage.runtime_secs) : "", stopped?.reached === "runtime"],
        [t("Retries"), usage.retries > 0 ? count(usage.retries) : "", stopped?.reached === "retries"],
        [t("Plugin calls"), usage.connector_calls > 0 ? count(usage.connector_calls) : "", stopped?.reached === "connector_calls"],
      ] as [string, string, boolean][]).filter(([, value]) => value)
    : [];
  const field = (key: keyof LimitFields, label: string, keyboard: "decimal-pad" | "number-pad") => (
    <FieldRow
      label={label}
      value={fields[key]}
      onChangeText={(text) => setFields((f) => ({ ...f, [key]: text }))}
      placeholder={t("No limit")}
      keyboardType={keyboard}
      textAlign="right"
      editable={!busy}
    />
  );

  return (
    <>
      <Stack.Screen options={{ title: t("Limits") }} />
      <SaveToolbar
        label={t("Save")}
        disabled={busy}
        onSave={() => {
          const limits = read();
          if (limits) void run(() => save(limits));
        }}
      />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        {stopped && (
          <Section>
            <Row title={stoppedTitle(stopped)} subtitle={stoppedDetail(stopped)} subtitleLines={3} leading={<Symbol name="exclamationmark.circle.fill" size={22} color={orange} />} />
            <Row title={t("Resume")} action onPress={busy ? undefined : () => resume(stopped)} />
          </Section>
        )}
        <Section title={t("Limits")} footer={`${footer} ${t("Spending is in US dollars and run time in minutes. Leave a limit empty for none.")}`}>
          {field("usd", t("Spending"), "decimal-pad")}
          {field("tokens", t("Tokens"), "number-pad")}
          {field("minutes", t("Run time"), "decimal-pad")}
          {field("retries", t("Retries"), "number-pad")}
          {field("calls", t("Plugin calls"), "number-pad")}
        </Section>
        {used.length > 0 && (
          <Section title={t("Used")}>
            {used.map(([title, value, reached]) => (
              <Row key={title} title={title} accessory={<Text style={[styles.value, { color: reached ? orange : p.secondaryLabel }]}>{value}</Text>} />
            ))}
          </Section>
        )}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  value: { fontSize: Font.body },
});
