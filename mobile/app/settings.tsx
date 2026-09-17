import * as Application from "expo-application";
import { Stack, useRouter } from "expo-router";
import { useState } from "react";
import { Alert, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../src/core/engine";
import { connectedProviders, isRunner, providerLabel } from "../src/core/model";
import { deviceIsOnline, useStore } from "../src/core/store";
import { FieldRow, Row, Section } from "../src/ui/forms";
import { lastSeen } from "../src/ui/format";
import { Symbol } from "../src/ui/Symbol";
import { usePalette } from "../src/ui/theme";
import { deviceSymbol } from "../src/ui/devices";
import { languageName, pickDictationLanguage, useDictationLanguage } from "../src/ui/dictation";

export default function SettingsScreen() {
  const router = useRouter();
  const p = usePalette();
  const devices = useStore((s) => s.devices);
  const seen = useStore((s) => s.device_seen);
  const relayConnected = useStore((s) => s.relayConnected);
  const relayUrl = useStore((s) => s.relayUrl);
  const identity = useStore((s) => s.identityId);
  const thisDevice = devices.find((d) => d.is_this_device);
  const [name, setName] = useState(thisDevice?.name ?? "");
  const dictation = useDictationLanguage();
  const thisId = engine.deviceId;
  const sorted = [...devices].sort((a, b) => (a.id === thisId ? -1 : b.id === thisId ? 1 : a.name.localeCompare(b.name)));

  function commitName() {
    if (thisDevice && name.trim() && name.trim() !== thisDevice.name) void engine.renameDevice(name);
  }

  function confirmUnpair() {
    Alert.alert("Unpair this phone?", "Its keys and the synced chats are removed from this phone. Your Mac keeps everything, and you can pair again any time.", [
      { text: "Cancel", style: "cancel" },
      {
        text: "Unpair",
        style: "destructive",
        onPress: () => {
          void engine.unpair().then(() => router.dismissAll());
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

        <Section title="Devices" footer="Desktop Devices are Runners: they run bots with their own provider credentials. Phones and tablets read and write chats.">
          {sorted.map((device) => (
            <Row
              key={device.id}
              title={device.id === thisId ? `${device.name} (this phone)` : device.name}
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
          Tinybot {Application.nativeApplicationVersion ?? ""} ({Application.nativeBuildVersion ?? ""})
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
