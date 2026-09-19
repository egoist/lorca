// One Device, slid in from its row in Settings inside the same sheet: the bots assigned to it,
// its plugins when it is a Runner, and the machine itself.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { Alert, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../../src/core/engine";
import { isRunner, providerLabel, type Device, type PluginStatus } from "../../../src/core/model";
import { deviceIsOnline, useStore } from "../../../src/core/store";
import { BotAvatar } from "../../../src/ui/Avatar";
import { deviceSymbol } from "../../../src/ui/devices";
import { Row, Section } from "../../../src/ui/forms";
import { lastSeen } from "../../../src/ui/format";
import { Symbol } from "../../../src/ui/Symbol";
import { usePalette } from "../../../src/ui/theme";

const OS_NAMES: Record<string, string> = { macos: "macOS", linux: "Linux", windows: "Windows", ios: "iOS", ipados: "iPadOS", android: "Android" };

const PLUGIN_STATES: Record<PluginStatus["state"], string> = {
  ready: "Ready",
  needs_setup: "Needs setup",
  needs_auth: "Needs sign-in",
  connecting: "Connecting…",
  error: "Error",
};

export default function DeviceScreen() {
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const device = useStore((s) => s.devices.find((d) => d.id === id));
  const seen = useStore((s) => s.device_seen[id]);
  const allBots = useStore((s) => s.bots);
  const chats = useStore((s) => s.chats);
  const relayConnected = useStore((s) => s.relayConnected);
  const relayUrl = useStore((s) => s.relayUrl);

  if (!device) return null;

  const isThis = device.id === engine.deviceId;
  const runner = isRunner(device);
  const online = isThis || deviceIsOnline(device.id);
  const bots = allBots.filter((b) => b.runner_id === device.id);
  const osName = OS_NAMES[device.os] ?? device.os;
  const relay = relayUrl?.replace(/^https?:\/\//, "");

  function openChat(botId: string) {
    const dm = chats.find((c) => c.kind === "dm" && c.bot_ids[0] === botId);
    if (!dm) return;
    router.dismissAll();
    router.push(`/chat/${dm.id}`);
  }

  function confirmUnpair(target: Device) {
    const detail = isRunner(target)
      ? "It loses its keys and synced chats the next time it connects, and bots assigned to it stop running until you assign them to another Runner. You can pair it again any time."
      : "It loses its keys and synced chats the next time it connects. You can pair it again any time.";
    Alert.alert(`Unpair ${target.name}?`, detail, [
      { text: "Cancel", style: "cancel" },
      {
        text: "Unpair",
        style: "destructive",
        onPress: () => {
          engine
            .unpairDevice(target.id)
            .then(() => router.back())
            .catch((error: unknown) => Alert.alert(`Couldn’t unpair ${target.name}`, error instanceof Error ? error.message : String(error)));
        },
      },
    ]);
  }

  return (
    <>
      <Stack.Screen options={{ title: "" }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <View style={styles.header}>
          <Symbol name={deviceSymbol(device.os, device.model)} size={48} color={p.label} />
          <Text style={[styles.name, { color: p.label }]}>{device.name}</Text>
          <Text style={[styles.model, { color: p.secondaryLabel }]}>{[device.model, device.os_version].filter(Boolean).join(" · ")}</Text>
          <View style={styles.status}>
            <View style={[styles.dot, { backgroundColor: online ? p.green : p.tertiaryLabel }]} />
            <Text style={[styles.statusText, { color: online ? p.green : p.secondaryLabel }]}>{online ? "Online" : lastSeen(seen)}</Text>
          </View>
        </View>

        {runner && (
          <Section title="Bots assigned here">
            {bots.length === 0 ? <Row title="No bots assigned" /> : bots.map((bot) => <Row key={bot.id} title={bot.name} subtitle={[bot.label, providerLabel(bot.provider)].filter(Boolean).join(" · ")} leading={<BotAvatar bot={bot} size={32} />} chevron onPress={() => openChat(bot.id)} />)}
          </Section>
        )}

        {runner && (
          <Section title="Plugins">
            {(device.plugins ?? []).length === 0 ? (
              <Row title="No plugins installed" />
            ) : (
              (device.plugins ?? []).map((plugin) => <Row key={plugin.id} title={plugin.name} subtitle={plugin.state === "error" ? plugin.detail || plugin.description : plugin.description} detail={PLUGIN_STATES[plugin.state]} icon={plugin.icon ?? "puzzlepiece.extension"} />)
            )}
          </Section>
        )}

        <Section title="Machine" footer={runner ? undefined : `${osName} Devices hold your keys and chats but never run a bot. Assign bots to a Runner: a Device running macOS, Linux, or Windows.`}>
          <Row title="Machine key" detail={device.machine_key} />
          <Row title="OS" detail={device.os_version || osName} />
          <Row title="Role" detail={runner ? "Runner" : "Device"} />
          <Row title="Last seen" detail={online ? "Active now" : lastSeen(seen)} />
          <Row title="Relay" detail={relay ? (relayConnected ? relay : `${relay} · offline`) : "Not configured"} />
        </Section>

        {!isThis && (
          <Section>
            <Row title="Unpair This Device" icon="xmark" destructive onPress={() => confirmUnpair(device)} />
          </Section>
        )}
      </ScrollView>
    </>
  );
}

const styles = StyleSheet.create({
  header: { alignItems: "center", paddingTop: 12, paddingHorizontal: 24, gap: 4 },
  name: { fontSize: 22, fontWeight: "600", marginTop: 8, textAlign: "center" },
  model: { fontSize: 13, textAlign: "center" },
  status: { flexDirection: "row", alignItems: "center", gap: 6, marginTop: 4 },
  dot: { width: 8, height: 8, borderRadius: 4 },
  statusText: { fontSize: 13, fontWeight: "500" },
});
