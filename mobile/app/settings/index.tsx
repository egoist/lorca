import * as Application from "expo-application";
import { AlertDialog, Column, Host, OutlinedTextField, RadioButton, Row as ComposeRow, Text as ComposeText, TextButton } from "@expo/ui/jetpack-compose";
import { clickable, fillMaxWidth, padding } from "@expo/ui/jetpack-compose/modifiers";
import { Stack, useRouter } from "expo-router";
import { useState } from "react";
import { Alert, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../src/core/engine";
import { isRunner, providerLabel } from "../../src/core/model";
import { deviceIsOnline, useStore } from "../../src/core/store";
import { deviceLanguage, languageNames, languages, setAppLanguage, t, useLanguage } from "../../src/i18n";
import { FieldRow, Row, Section, ToggleRow } from "../../src/ui/forms";
import { lastSeen } from "../../src/ui/format";
import { Symbol } from "../../src/ui/Symbol";
import { usePalette } from "../../src/ui/theme";
import { deviceSymbol } from "../../src/ui/devices";
import { CloseToolbar } from "../../src/ui/navigation";
import {
  automaticLanguage,
  languageName,
  setDictationLanguage,
  useDictationLanguage,
  useSupportedLanguages,
} from "../../src/ui/dictation";

export default function SettingsScreen() {
  const router = useRouter();
  const p = usePalette();
  const devices = useStore((s) => s.devices);
  const seen = useStore((s) => s.device_seen);
  const relayConnected = useStore((s) => s.relayConnected);
  const relayUpdateRequired = useStore((s) => s.relayUpdateRequired);
  const relayError = useStore((s) => s.relayError);
  const relayUrl = useStore((s) => s.relayUrl);
  const identity = useStore((s) => s.identityId);
  const autoReview = useStore((s) => s.auto_review);
  const providers = useStore((s) => s.providers);
  const thisDevice = devices.find((d) => d.is_this_device);
  const [name, setName] = useState(thisDevice?.name ?? "");
  const [addingRule, setAddingRule] = useState(false);
  const [ruleText, setRuleText] = useState("");
  const [ruleBehavior, setRuleBehavior] = useState<"allow" | "ask">("allow");
  const dictation = useDictationLanguage();
  const appLanguage = useLanguage();

  // The Mac app's pop-up: Automatic with the language it resolves to, a separator, then every
  // language the recognizer knows, by name.
  const dictationLanguages = useSupportedLanguages();
  const automaticDictation = t("Automatic ({language})", { language: languageName(automaticLanguage()) });
  const dictationChoices = [
    { title: automaticDictation, selected: !dictation.setting, onPress: () => setDictationLanguage(undefined), dividerAfter: true },
    ...dictationLanguages.map((tag) => ({ title: languageName(tag), selected: dictation.setting === tag, onPress: () => setDictationLanguage(tag) })),
  ];
  const systemLanguage = t("System ({language})", { language: languageNames[deviceLanguage()] });
  const appLanguageChoices = [
    { title: systemLanguage, selected: !appLanguage.chosen, onPress: () => setAppLanguage(undefined) },
    ...languages.map((code) => ({ title: languageNames[code], selected: appLanguage.chosen === code, onPress: () => setAppLanguage(code) })),
  ];
  const thisId = engine.deviceId;
  const sorted = [...devices].sort((a, b) =>
    a.id === thisId ? -1 : b.id === thisId ? 1 : a.name.localeCompare(b.name),
  );

  function commitName() {
    if (thisDevice && name.trim() && name.trim() !== thisDevice.name)
      void engine.renameDevice(name);
  }

  function addRule() {
    if (Platform.OS === "android") {
      setRuleText("");
      setRuleBehavior("allow");
      setAddingRule(true);
      return;
    }
    Alert.prompt(t("New rule"), t("When a bot wants to…"), [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Allow automatically"),
        onPress: (text?: string) => saveRule(text, "allow"),
      },
      { text: t("Ask first"), onPress: (text?: string) => saveRule(text, "ask") },
    ]);
  }

  function saveRule(text: string | undefined, behavior: "allow" | "ask") {
    const trimmed = (text ?? "").trim();
    if (!trimmed) return;
    engine.setAutoReview({
      ...autoReview,
      rules: [...autoReview.rules, { id: "", text: trimmed, behavior }],
    });
  }

  function editRule(id: string) {
    const rule = autoReview.rules.find((r) => r.id === id);
    if (!rule) return;
    const flipped: "allow" | "ask" =
      rule.behavior === "allow" ? "ask" : "allow";
    Alert.alert(
      rule.text,
      rule.behavior === "allow" ? t("Allow automatically") : t("Ask first"),
      [
        { text: t("Cancel"), style: "cancel" },
        {
          text: flipped === "allow" ? t("Allow automatically") : t("Ask first"),
          onPress: () =>
            engine.setAutoReview({
              ...autoReview,
              rules: autoReview.rules.map((r) =>
                r.id === id ? { ...r, behavior: flipped } : r,
              ),
            }),
        },
        {
          text: t("Delete"),
          style: "destructive",
          onPress: () =>
            engine.setAutoReview({
              ...autoReview,
              rules: autoReview.rules.filter((r) => r.id !== id),
            }),
        },
      ],
    );
  }

  function confirmUnpair() {
    Alert.alert(
      t("Unpair this phone?"),
      t("Its keys and the synced chats are removed from this phone. Your other paired Devices keep everything, and you can pair again any time."),
      [
        { text: t("Cancel"), style: "cancel" },
        {
          text: t("Unpair"),
          style: "destructive",
          onPress: () => {
            // Forgetting the identity flips `paired`, and the guarded stack swaps to the pair
            // screen on its own; dismissing here would find nothing to pop.
            void engine.unpair();
          },
        },
      ],
    );
  }

  return (
    <>
      <Stack.Screen options={{ title: t("Settings") }} />
      <CloseToolbar label={Platform.OS === "android" ? t("Close") : t("Done")} onClose={() => router.dismiss()} />
      <ScrollView
        contentInsetAdjustmentBehavior="automatic"
        contentContainerStyle={{ paddingBottom: 40 }}
        keyboardDismissMode="on-drag"
      >
        <Section
          title={t("This phone")}
          footer={t("How this phone shows up in the Device list on your other Devices.")}
        >
          <FieldRow
            label={t("Name")}
            value={name}
            onChangeText={setName}
            onBlur={commitName}
            onSubmitEditing={commitName}
            returnKeyType="done"
            autoCapitalize="words"
            // A value among values: on the trailing edge in their color, as Role and App Language are.
            style={{ textAlign: "right", color: p.secondaryLabel }}
          />
          <Row title={t("Role")} detail={t("Device")} />
          <Row
            title={t("App Language")}
            menu={{ title: t("App Language"), value: appLanguage.chosen ? languageNames[appLanguage.chosen] : systemLanguage, choices: appLanguageChoices }}
          />
        </Section>

        <Section title={t("Dictation")}>
          <Row title={t("Language")} menu={{ title: t("Dictation Language"), value: dictation.setting ? languageName(dictation.setting) : automaticDictation, choices: dictationChoices }} />
        </Section>

        <Section title={t("Account")}>
          <Row title={t("Identity")} detail={identity ?? "—"} />
          <Row
            title={t("Relay")}
            detail={relayUrl?.replace(/^https?:\/\//, "") ?? "—"}
            // Why the last try to connect failed, as the core has it, until one goes through.
            subtitle={relayUpdateRequired ? t("Update Lorca to sync") : relayConnected ? t("Connected") : relayError ? relayError.message : t("Connecting…")}
            subtitleLines={3}
          />
        </Section>

        <Section
          title={t("Auto-review")}
          footer={
            autoReview.is_enabled
              ? t('Lorca checks effectful plugin actions and every shell command before they run. Safe commands normally run automatically; risky commands ask you first. Add rules to customize what bots can do automatically; "Ask first" wins if rules conflict.')
              : t("Off: every shell command and effectful plugin action asks you first.")
          }
        >
          <ToggleRow
            title={t("Check actions before they run")}
            value={autoReview.is_enabled}
            onValueChange={(v) =>
              engine.setAutoReview({ ...autoReview, is_enabled: v })
            }
          />
          {autoReview.rules.map((rule) => (
            <Row
              key={rule.id}
              title={rule.text}
              detail={
                rule.behavior === "allow" ? t("Allow automatically") : t("Ask first")
              }
              onPress={() => editRule(rule.id)}
            />
          ))}
          <Row title={t("Add rule…")} onPress={addRule} />
        </Section>

        <Section
          title={t("Providers")}
          footer={t("Credentials belong to your account and reach every paired Device encrypted.")}
        >
          {providers.map((provider) => (
            <Row
              key={provider.kind}
              title={providerLabel(provider.kind)}
              subtitle={
                provider.is_connected ? provider.detail || undefined : undefined
              }
              detail={provider.is_connected ? t("Connected") : t("Not connected")}
              onPress={() => router.push(`/settings/provider/${provider.kind}`)}
              chevron
            />
          ))}
        </Section>

        <Section
          title={t("Devices")}
          footer={t("Desktop Devices are Runners: they run bots with your account's provider credentials. Phones and tablets read and write chats.")}
        >
          {sorted.map((device) => {
            const online = device.id === thisId || deviceIsOnline(device.id);
            return (
              <Row
                key={device.id}
                title={
                  device.id === thisId
                    ? t("{name} (this phone)", { name: device.name })
                    : device.name
                }
                onPress={() => router.push(`/settings/device/${device.id}`)}
                chevron
                subtitle={[
                  device.model,
                  isRunner(device) ? t("Runner") : t("Device"),
                  online ? t("Online") : lastSeen(seen[device.id]),
                ]
                  .filter(Boolean)
                  .join(" · ")}
                leading={
                  <View style={styles.deviceIcon}>
                    <Symbol
                      name={deviceSymbol(device.os, device.model)}
                      size={22}
                      color={p.label}
                    />
                    <View
                      style={[
                        styles.deviceDot,
                        {
                          backgroundColor: online ? p.green : p.tertiaryLabel,
                          borderColor: p.cell,
                        },
                      ]}
                    />
                  </View>
                }
              />
            );
          })}
        </Section>

        <Section>
          <Row
            title={t("Unpair This Phone")}
            icon="xmark"
            destructive
            onPress={confirmUnpair}
          />
        </Section>

        <Text style={[styles.version, { color: p.tertiaryLabel }]}>
          {Application.applicationName ?? "Lorca"} {Application.nativeApplicationVersion ?? ""} (
          {Application.nativeBuildVersion ?? ""})
        </Text>
      </ScrollView>
      {Platform.OS === "android" && addingRule ? (
        <Host style={styles.dialogHost} pointerEvents="box-none">
          <AlertDialog onDismissRequest={() => setAddingRule(false)}>
            <AlertDialog.Title>
              <ComposeText style={{ typography: "headlineSmall" }}>{t("New rule")}</ComposeText>
            </AlertDialog.Title>
            <AlertDialog.Text>
              <Column verticalArrangement={{ spacedBy: 12 }} modifiers={[fillMaxWidth()]}>
                <OutlinedTextField autoFocus singleLine onValueChange={setRuleText} modifiers={[fillMaxWidth()]}>
                  <OutlinedTextField.Placeholder>
                    <ComposeText>{t("When a bot wants to…")}</ComposeText>
                  </OutlinedTextField.Placeholder>
                </OutlinedTextField>
                <ComposeRow
                  verticalAlignment="center"
                  modifiers={[fillMaxWidth(), clickable(() => setRuleBehavior("allow")), padding(0, 4, 0, 4)]}
                >
                  <RadioButton selected={ruleBehavior === "allow"} onClick={() => setRuleBehavior("allow")} />
                  <ComposeText>{t("Allow automatically")}</ComposeText>
                </ComposeRow>
                <ComposeRow
                  verticalAlignment="center"
                  modifiers={[fillMaxWidth(), clickable(() => setRuleBehavior("ask")), padding(0, 4, 0, 4)]}
                >
                  <RadioButton selected={ruleBehavior === "ask"} onClick={() => setRuleBehavior("ask")} />
                  <ComposeText>{t("Ask first")}</ComposeText>
                </ComposeRow>
              </Column>
            </AlertDialog.Text>
            <AlertDialog.ConfirmButton>
              <TextButton
                enabled={!!ruleText.trim()}
                onClick={() => {
                  saveRule(ruleText, ruleBehavior);
                  setAddingRule(false);
                }}
              >
                <ComposeText>{t("Create")}</ComposeText>
              </TextButton>
            </AlertDialog.ConfirmButton>
            <AlertDialog.DismissButton>
              <TextButton onClick={() => setAddingRule(false)}>
                <ComposeText>{t("Cancel")}</ComposeText>
              </TextButton>
            </AlertDialog.DismissButton>
          </AlertDialog>
        </Host>
      ) : null}
    </>
  );
}

const styles = StyleSheet.create({
  deviceIcon: { width: 32, alignItems: "center" },
  deviceDot: {
    position: "absolute",
    right: 0,
    bottom: -2,
    width: 10,
    height: 10,
    borderRadius: 5,
    borderWidth: 2,
  },
  version: { textAlign: "center", fontSize: 12, marginTop: 28 },
  dialogHost: { position: "absolute", top: 0, right: 0, bottom: 0, left: 0 },
});
