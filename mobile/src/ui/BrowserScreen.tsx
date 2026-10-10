// A bot's browser profiles on its Runner, slid in from the Browser row in Details, as the Mac's
// Browser sheet lists them: each profile and how it stands, and on a tap its menu. A profile opens
// in a window on the Runner only, so that is where the user signs in; from here they take the
// browser over, hand it back, take a screenshot into the chat, close it, add, or delete one.

import { Host, OutlinedTextField, AlertDialog, Text as ComposeText, TextButton } from "@expo/ui/jetpack-compose";
import { fillMaxWidth } from "@expo/ui/jetpack-compose/modifiers";
import { Stack, useLocalSearchParams } from "expo-router";
import { useCallback, useEffect, useRef, useState } from "react";
import { Alert, Platform, PlatformColor, ScrollView, StyleSheet, Text, View, type ColorValue } from "react-native";
import { engine } from "../core/engine";
import type { BrowserProfile } from "../core/model";
import { useStore } from "../core/store";
import { t, useLanguage } from "../i18n";
import { alert } from "./alert";
import { profileActions, profileExplanation, profileStateWord, type ProfileAction } from "./browserProfiles";
import { Row, Section } from "./forms";
import { RowMenu } from "./RowMenu";
import { accentColor, Font, usePalette } from "./theme";

/// Through the relay the Runner is asked again this often while the screen is up, since another
/// Device can change a profile too.
const REFRESH_MS = 5000;

export default function BrowserScreen() {
  useLanguage();
  const { id: botId, chat: chatId } = useLocalSearchParams<{ id: string; chat?: string }>();
  const bot = useStore((s) => s.bots.find((b) => b.id === botId));
  const runner = useStore((s) => s.devices.find((d) => d.id === bot?.runner_id));
  const p = usePalette();
  const [profiles, setProfiles] = useState<BrowserProfile[]>();
  const [loadError, setLoadError] = useState<string>();
  // What a row says while its action runs on the Runner ("Closing…").
  const [pending, setPending] = useState<Record<string, string>>({});
  const [naming, setNaming] = useState(false);
  const [name, setName] = useState("");
  // The newest read: an older answer, or one begun before an action, never covers it.
  const loads = useRef(0);

  const load = useCallback(async () => {
    if (!botId) return;
    const load = ++loads.current;
    try {
      const list = await engine.browserProfiles(botId);
      if (load === loads.current) {
        setProfiles(list);
        setLoadError(undefined);
      }
    } catch (error) {
      if (load === loads.current) setLoadError(error instanceof Error ? error.message : String(error));
    }
  }, [botId]);

  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), REFRESH_MS);
    return () => clearInterval(timer);
  }, [load]);

  if (!bot || !runner) return null;
  const runnerName = runner.name;
  const orange: ColorValue = Platform.OS === "ios" ? PlatformColor("systemOrange") : accentColor("orange", p.dark);

  async function perform(profile: BrowserProfile, method: string, label: string | undefined, failure: string, params: Record<string, string | number> = {}) {
    if (label) setPending((all) => ({ ...all, [profile.id]: label }));
    loads.current++;
    try {
      await engine.browserAction(method, botId, { ...params, session_id: profile.id });
    } catch (error) {
      alert(failure, error instanceof Error ? error.message : String(error));
    } finally {
      setPending(({ [profile.id]: _, ...rest }) => rest);
      void load();
    }
  }

  function choose(profile: BrowserProfile, action: ProfileAction["id"]) {
    switch (action) {
      case "takeover":
        return void perform(profile, "browser.takeover", t("Taking over…"), t("Couldn't take over the browser"));
      case "resume":
        return void perform(profile, "browser.resume", t("Returning…"), t("Couldn't hand the browser back"), { revision: profile.revision });
      case "screenshot":
        return chatId && void perform(profile, "browser.screenshot", undefined, t("Couldn't take a screenshot"), { chat_id: chatId });
      case "stop":
        return void perform(profile, "browser.stop", t("Closing…"), t("Couldn't close the browser"));
      case "delete":
        return alert(t("Delete the “{name}” profile?", { name: profile.name }), t("Its browser closes, and the sites it signed in to are signed out for {bot}.", { bot: bot!.name }), [
          { text: t("Cancel"), style: "cancel" },
          { text: t("Delete"), style: "destructive", onPress: () => void perform(profile, "browser.delete", t("Deleting…"), t("Couldn't delete the profile")) },
        ]);
    }
  }

  async function add(text: string | undefined) {
    const value = (text ?? "").trim();
    if (!value) return;
    try {
      await engine.browserAction("browser.create", botId, { name: value });
    } catch (error) {
      alert(t("Couldn't add the profile"), error instanceof Error ? error.message : String(error));
    }
    void load();
  }

  function startAdding() {
    if (Platform.OS === "android") {
      setName("");
      setNaming(true);
      return;
    }
    Alert.prompt(t("New Profile"), t("{bot} keeps the sign-ins you make in it.", { bot: bot!.name }), [
      { text: t("Cancel"), style: "cancel" },
      { text: t("Add"), isPreferred: true, onPress: (text?: string) => void add(text) },
    ]);
  }

  const footer =
    loadError ??
    (profiles?.length === 0
      ? t("A profile keeps sign-ins for {bot}'s browser. Add one, then open it on {runner} to sign in.", { bot: bot.name, runner: runnerName })
      : t("Profiles open in a window on {runner}, where you sign in.", { runner: runnerName }));

  return (
    <>
      <Stack.Screen options={{ title: t("Browser") }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={styles.content}>
        <Section title={t("Profiles")} footer={profiles || loadError ? footer : undefined}>
          {(profiles ?? []).map((profile) => {
            const busy = pending[profile.id];
            const actions = profileActions(profile, runnerName, !!chatId, !!busy);
            return (
              <RowMenu
                key={profile.id}
                actions={actions}
                onChoose={(action) => choose(profile, action)}
                label={`${profile.name}, ${busy ?? profileStateWord(profile)}`}
                hint={profileExplanation(profile, bot.name, runnerName)}
              >
                <Row
                  title={profile.name}
                  icon="person.crop.circle"
                  accessory={
                    <Text style={[styles.state, { color: !busy && profile.state === "human" ? orange : p.secondaryLabel }]} numberOfLines={1}>
                      {busy ?? profileStateWord(profile)}
                    </Text>
                  }
                />
              </RowMenu>
            );
          })}
          {profiles && <Row title={t("Add Profile")} icon="plus" onPress={startAdding} />}
        </Section>
      </ScrollView>
      {Platform.OS === "android" && naming ? (
        <Host style={StyleSheet.absoluteFill} pointerEvents="box-none">
          <AlertDialog onDismissRequest={() => setNaming(false)}>
            <AlertDialog.Title>
              <ComposeText style={{ typography: "headlineSmall" }}>{t("New Profile")}</ComposeText>
            </AlertDialog.Title>
            <AlertDialog.Text>
              <OutlinedTextField autoFocus singleLine onValueChange={setName} modifiers={[fillMaxWidth()]}>
                <OutlinedTextField.Placeholder>
                  <ComposeText>{t("Work")}</ComposeText>
                </OutlinedTextField.Placeholder>
              </OutlinedTextField>
            </AlertDialog.Text>
            <AlertDialog.ConfirmButton>
              <TextButton
                enabled={!!name.trim()}
                onClick={() => {
                  setNaming(false);
                  void add(name);
                }}
              >
                <ComposeText>{t("Add")}</ComposeText>
              </TextButton>
            </AlertDialog.ConfirmButton>
            <AlertDialog.DismissButton>
              <TextButton onClick={() => setNaming(false)}>
                <ComposeText>{t("Cancel")}</ComposeText>
              </TextButton>
            </AlertDialog.DismissButton>
          </AlertDialog>
        </Host>
      ) : null}
    </>
  );
}

/// A profile's row whose tap opens its menu: SwiftUI's on iOS, labelled for VoiceOver with the
/// row's words, which its hosted row does not lend it; Material's dropdown on Android. Delete is a
/// group of its own.
const styles = StyleSheet.create({
  content: { paddingBottom: 40 },
  state: { fontSize: Font.body, maxWidth: 180 },
});
