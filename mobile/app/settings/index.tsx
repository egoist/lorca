import * as Application from "expo-application";
import { Stack, useRouter } from "expo-router";
import { useState } from "react";
import { Alert, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../src/core/engine";
import { connectedProviders, isRunner, providerLabel } from "../../src/core/model";
import { deviceIsOnline, useStore } from "../../src/core/store";
import { FieldRow, Row, Section, ToggleRow } from "../../src/ui/forms";
import { lastSeen } from "../../src/ui/format";
import { Symbol } from "../../src/ui/Symbol";
import { usePalette } from "../../src/ui/theme";
import { deviceSymbol } from "../../src/ui/devices";
import { languageName, pickDictationLanguage, useDictationLanguage } from "../../src/ui/dictation";

export default function SettingsScreen() {
  const router = useRouter();
  const p = usePalette();
  const devices = useStore((s) => s.devices);
  const seen = useStore((s) => s.device_seen);
  const relayConnected = useStore((s) => s.relayConnected);
  const relayUrl = useStore((s) => s.relayUrl);
  const identity = useStore((s) => s.identityId);
  const autoReview = useStore((s) => s.auto_review);
  const thisDevice = devices.find((d) => d.is_this_device);
  const [name, setName] = useState(thisDevice?.name ?? "");
  const dictation = useDictationLanguage();
  const thisId = engine.deviceId;
  const sorted = [...devices].sort((a, b) => (a.id === thisId ? -1 : b.id === thisId ? 1 : a.name.localeCompare(b.name)));

  function commitName() {
    if (thisDevice && name.trim() && name.trim() !== thisDevice.name) void engine.renameDevice(name);
  }

  function addRule() {
    Alert.prompt("New rule", "When a bot wants to…", [
      { text: "Cancel", style: "cancel" },
      { text: "Allow automatically", onPress: (text?: string) => saveRule(text, "allow") },
      { text: "Ask first", onPress: (text?: string) => saveRule(text, "ask") },
    ]);
  }

  function saveRule(text: string | undefined, behavior: "allow" | "ask") {
    const trimmed = (text ?? "").trim();
    if (!trimmed) return;
    engine.setAutoReview({ ...autoReview, rules: [...autoReview.rules, { id: "", text: trimmed, behavior }] });
  }

  function editRule(id: string) {
    const rule = autoReview.rules.find((r) => r.id === id);
    if (!rule) return;
    const flipped: "allow" | "ask" = rule.behavior === "allow" ? "ask" : "allow";
    Alert.alert(rule.text, rule.behavior === "allow" ? "Allow automatically" : "Ask first", [
      { text: "Cancel", style: "cancel" },
      { text: flipped === "allow" ? "Allow automatically" : "Ask first", onPress: () => engine.setAutoReview({ ...autoReview, rules: autoReview.rules.map((r) => (r.id === id ? { ...r, behavior: flipped } : r)) }) },
      { text: "Delete", style: "destructive", onPress: () => engine.setAutoReview({ ...autoReview, rules: autoReview.rules.filter((r) => r.id !== id) }) },
    ]);
  }

  function confirmUnpair() {
    Alert.alert("Unpair this phone?", "Its keys and the synced chats are removed from this phone. Your Mac keeps everything, and you can pair again any time.", [
      { text: "Cancel", style: "cancel" },
      {
        text: "Unpair",
        style: "destructive",
        onPress: () => {
          // Forgetting the identity flips `paired`, and the guarded stack swaps to the pair
          // screen on its own; dismissing here would find nothing to pop.
          void engine.unpair();
        },
      },
    ]);
  }

  return (
    <>
      <Stack.Screen options={{ title: "Settings" }} />
      <Stack.Toolbar placement="right">
        <Stack.Toolbar.Button variant="done" onPress={() => router.dismiss()}>
          Done
        </Stack.Toolbar.Button>
      </Stack.Toolbar>
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }} keyboardDismissMode="on-drag">
        <Section title="This phone" footer="How this phone shows up in the Device list on your other Devices.">
          <FieldRow label="Name" value={name} onChangeText={setName} onBlur={commitName} onSubmitEditing={commitName} returnKeyType="done" autoCapitalize="words" />
          <Row title="Role" detail="Device" />
          <Row title="Dictation Language" detail={dictation.setting ? languageName(dictation.setting) : `Automatic (${languageName(dictation.language)})`} chevron onPress={() => void pickDictationLanguage()} />
        </Section>

        <Section title="Account">
          <Row title="Identity" detail={identity ?? "—"} />
          <Row title="Relay" detail={relayUrl?.replace(/^https?:\/\//, "") ?? "—"} subtitle={relayConnected ? "Connected" : "Connecting…"} />
        </Section>

        <Section title="Auto-review" footer={autoReview.is_enabled ? "Lorca checks each plugin action before it runs and asks you first when needed. Add rules to customize what bots can do automatically; \"Ask first\" wins if rules conflict. Built-in safety checks always apply." : "Off: every plugin action that changes something asks you first."}>
          <ToggleRow title="Check actions before they run" value={autoReview.is_enabled} onValueChange={(v) => engine.setAutoReview({ ...autoReview, is_enabled: v })} />
          {autoReview.rules.map((rule) => (
            <Row key={rule.id} title={rule.text} detail={rule.behavior === "allow" ? "Allow automatically" : "Ask first"} onPress={() => editRule(rule.id)} />
          ))}
          <Row title="Add rule…" onPress={addRule} />
        </Section>

        <Section title="Devices" footer="Desktop Devices are Runners: they run bots with their own provider credentials. Phones and tablets read and write chats.">
          {sorted.map((device) => (
            <Row
              key={device.id}
              title={device.id === thisId ? `${device.name} (this phone)` : device.name}
              onPress={() => router.push(`/settings/device/${device.id}`)}
              chevron
              subtitle={[device.model, isRunner(device) ? "Runner" : "Device", device.id === thisId ? "Online" : lastSeen(seen[device.id]), ...connectedProviders(device).map(providerLabel)].filter(Boolean).join(" · ")}
              leading={
                <View style={styles.deviceIcon}>
                  <Symbol name={deviceSymbol(device.os, device.model)} size={22} color={p.label} />
                  <View style={[styles.deviceDot, { backgroundColor: device.id === thisId || deviceIsOnline(device.id) ? p.green : p.tertiaryLabel, borderColor: p.cell }]} />
                </View>
              }
            />
          ))}
        </Section>

        <Section>
          <Row title="Unpair This Phone" icon="xmark" destructive onPress={confirmUnpair} />
        </Section>

        <Text style={[styles.version, { color: p.tertiaryLabel }]}>
          Lorca {Application.nativeApplicationVersion ?? ""} ({Application.nativeBuildVersion ?? ""})
        </Text>
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  deviceIcon: { width: 32, alignItems: "center" },
  deviceDot: { position: "absolute", right: 0, bottom: -2, width: 10, height: 10, borderRadius: 5, borderWidth: 2 },
  version: { textAlign: "center", fontSize: 12, marginTop: 28 },
});
