// A plugin on its Runner, slid in from its row in Details or in a Device's Plugins. A named account
// (Gmail · Work) has its name, edited in place, and its sign-in, which says how it stands, as the
// desktop apps' Account card does; any other plugin, its state. Then its call limit, which every bot
// on the Runner shares, picked from menus. Adding an account and its setup happen in Lorca on a
// computer.

import { Stack, useLocalSearchParams } from "expo-router";
import { useEffect, useState } from "react";
import { ScrollView, StyleSheet } from "react-native";
import { engine } from "../core/engine";
import type { CallLimits, PluginDetail } from "../core/model";
import { useStore } from "../core/store";
import { t, useLanguage } from "../i18n";
import { alert } from "./alert";
import { FieldRow, Row, Section, type MenuChoice } from "./forms";
import { time } from "./format";
import { callSummary } from "./limits";
import { pluginStateWord } from "./plugins";

export default function AccountScreen() {
  useLanguage();
  const { id, runner: runnerId } = useLocalSearchParams<{ id: string; runner: string }>();
  const runner = useStore((s) => s.devices.find((d) => d.id === runnerId));
  const account = runner?.plugins?.find((plugin) => plugin.id === id);
  const [detail, setDetail] = useState<PluginDetail>();
  const [name, setName] = useState(account?.account_name ?? "");
  const [signingIn, setSigningIn] = useState(false);
  const [callLimits, setCallLimits] = useState<CallLimits>();

  useEffect(() => setName(account?.account_name ?? ""), [account?.account_name]);
  // Whether it is signed in is the Runner's to say; asked again whenever its state moves.
  useEffect(() => {
    let live = true;
    engine.pluginDetail(runnerId, id).then(
      (value) => live && setDetail(value),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [runnerId, id, account?.state, account?.detail]);
  useEffect(() => {
    let live = true;
    engine.callLimits(runnerId, id).then(
      (value) => live && setCallLimits(value),
      () => {},
    );
    return () => {
      live = false;
    };
  }, [runnerId, id]);

  if (!runner || !account) return null;
  const current = account.account_name ?? "";
  const signedIn = detail ? detail.servers.some((server) => server.auth.oauth && server.auth.signed_in) : account.state === "ready";
  const setUp = account.state !== "needs_setup";
  const word = signingIn
    ? t("Signing in…")
    : account.state === "ready"
      ? t("Signed in")
      : account.state === "needs_auth"
        ? t("Not signed in")
        : pluginStateWord(account);
  const footer =
    account.state === "error"
      ? account.detail
      : !setUp
        ? t("Finish its setup in Lorca on a computer, then sign in here.")
        : t("Keys and sign-ins are sent sealed to {runner} and stay there.", { runner: runner.name });

  async function commitName() {
    const value = name.trim();
    if (!value || value === current) return setName(current);
    try {
      await engine.renamePluginAccount(runnerId, id, value);
    } catch (error) {
      setName(current);
      alert(t("Couldn't rename it"), error instanceof Error ? error.message : String(error));
    }
  }

  async function signIn() {
    setSigningIn(true);
    try {
      await engine.connectPlugin(runnerId, id);
    } catch (error) {
      alert(t("Couldn't start the sign-in"), error instanceof Error ? error.message : String(error));
    } finally {
      setSigningIn(false);
      engine.pluginDetail(runnerId, id).then(setDetail, () => {});
    }
  }

  async function changeCallLimits(limits: CallLimits["limits"]) {
    try {
      setCallLimits(await engine.setCallLimits(runnerId, id, limits));
    } catch (error) {
      alert(t("Couldn't change the call limit"), error instanceof Error ? error.message : String(error));
    }
  }

  async function signOut() {
    try {
      await engine.signOutPlugin(runnerId, id);
      setDetail(await engine.pluginDetail(runnerId, id));
    } catch (error) {
      alert(t("Couldn't sign out of {name}", { name: account!.name }), error instanceof Error ? error.message : String(error));
    }
  }

  return (
    <>
      <Stack.Screen options={{ title: account.name }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content} keyboardDismissMode="on-drag" keyboardShouldPersistTaps="handled">
        {account.account_name ? (
          <Section title={t("Account")} footer={footer}>
            <FieldRow label={t("Name")} value={name} onChangeText={setName} onBlur={() => void commitName()} placeholder={t("Work")} autoCapitalize="words" returnKeyType="done" submitBehavior="blurAndSubmit" textAlign="right" />
            <Row title={t("Sign-in")} detail={word} />
            {setUp && !signingIn && (signedIn ? <Row title={t("Sign in again")} onPress={() => void signIn()} /> : <Row title={t("Sign in")} onPress={() => void signIn()} />)}
            {setUp && signedIn && !signingIn && <Row title={t("Sign out")} destructive onPress={() => void signOut()} />}
          </Section>
        ) : (
          <Section footer={account.state === "error" ? account.detail : undefined}>
            <Row title={t("State")} detail={account.state === "ready" ? t("Ready") : pluginStateWord(account)} />
          </Section>
        )}
        {callLimits && <CallLimitSection limits={callLimits} runner={runner.name} name={account.name} onChange={(limits) => void changeCallLimits(limits)} />}
      </ScrollView>
    </>
  );
}

/// How often all bots on the Runner may call the plugin, and how many at once, as menus of the
/// usual values; a limit set on a computer to something else shows as it is.
function CallLimitSection({ limits, runner, name, onChange }: { limits: CallLimits; runner: string; name: string; onChange: (limits: CallLimits["limits"]) => void }) {
  const current = limits.limits;
  const rates: MenuChoice[] = [10, 30, 60, 120, 300].map((calls) => ({
    title: callSummary({ max_calls: calls, window_secs: 60, max_concurrency: current.max_concurrency }),
    selected: current.window_secs === 60 && current.max_calls === calls,
    onPress: () => onChange({ ...current, max_calls: calls, window_secs: 60 }),
  }));
  const atOnce: MenuChoice[] = [1, 2, 4, 8, 16].map((n) => ({
    title: String(n),
    selected: current.max_concurrency === n,
    onPress: () => onChange({ ...current, max_concurrency: n }),
  }));
  const waiting = limits.retry_at && limits.retry_at * 1000 > Date.now() ? t("{name} asked Lorca to slow down. Calls wait until {time}.", { name, time: time(new Date(limits.retry_at * 1000)) }) : "";
  const footer = [t("All bots on {runner} share this limit when they use {name}.", { runner, name }), waiting].filter(Boolean).join(" ");
  return (
    <Section title={t("Call limit")} footer={footer}>
      <Row title={t("Calls")} menu={{ title: t("Calls"), value: callSummary(current), choices: rates }} />
      <Row title={t("At once")} menu={{ title: t("At once"), value: String(current.max_concurrency), choices: atOnce }} />
    </Section>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
});
