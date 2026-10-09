// A named account on its Runner (Gmail · Work), slid in from its row in Details or in a Device's
// Plugins: its name, edited in place, and its sign-in, which says how it stands, as the desktop
// apps' Account card does. Adding an account and its setup happen in Lorca on a computer.

import { Stack, useLocalSearchParams } from "expo-router";
import { useEffect, useState } from "react";
import { ScrollView, StyleSheet } from "react-native";
import { engine } from "../core/engine";
import type { PluginDetail } from "../core/model";
import { useStore } from "../core/store";
import { t, useLanguage } from "../i18n";
import { alert } from "./alert";
import { FieldRow, Row, Section } from "./forms";
import { pluginStateWord } from "./plugins";

export default function AccountScreen() {
  useLanguage();
  const { id, runner: runnerId } = useLocalSearchParams<{ id: string; runner: string }>();
  const runner = useStore((s) => s.devices.find((d) => d.id === runnerId));
  const account = runner?.plugins?.find((plugin) => plugin.id === id);
  const [detail, setDetail] = useState<PluginDetail>();
  const [name, setName] = useState(account?.account_name ?? "");
  const [signingIn, setSigningIn] = useState(false);

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

  if (!runner || !account?.account_name) return null;
  const current = account.account_name;
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
        <Section title={t("Account")} footer={footer}>
          <FieldRow label={t("Name")} value={name} onChangeText={setName} onBlur={() => void commitName()} placeholder={t("Work")} autoCapitalize="words" returnKeyType="done" submitBehavior="blurAndSubmit" textAlign="right" />
          <Row title={t("Sign-in")} detail={word} />
          {setUp && !signingIn && (signedIn ? <Row title={t("Sign in again")} onPress={() => void signIn()} /> : <Row title={t("Sign in")} onPress={() => void signIn()} />)}
          {setUp && signedIn && !signingIn && <Row title={t("Sign out")} destructive onPress={() => void signOut()} />}
        </Section>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
});
