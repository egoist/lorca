// One Device, slid in from its row in Settings inside the same sheet: the bots assigned to it,
// its plugins when it is a Runner, and the machine itself.

import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { useState } from "react";
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from "react-native";
import { engine } from "../../../src/core/engine";
import { deviceName, isRunner, providerLabel, type Device, type UpdateStatus } from "../../../src/core/model";
import { deviceIsOnline, useStore } from "../../../src/core/store";
import { t, useLanguage } from "../../../src/i18n";
import { BotAvatar } from "../../../src/ui/Avatar";
import { deviceSymbol } from "../../../src/ui/devices";
import { Row, Section } from "../../../src/ui/forms";
import { pluginStateWord } from "../../../src/ui/plugins";
import { lastSeen } from "../../../src/ui/format";
import { Symbol } from "../../../src/ui/Symbol";
import { Font, usePalette } from "../../../src/ui/theme";
import { alert } from "../../../src/ui/alert";

const OS_NAMES: Record<string, string> = { macos: "macOS", linux: "Linux", windows: "Windows", ios: "iOS", ipados: "iPadOS", android: "Android" };

/// A self-updating Runner's CLI: its version, then where its updates stand.
function cliStatus(version: string, update: UpdateStatus): string {
  const latest = update.latest ?? "";
  switch (update.state) {
    case "installing":
      return t("{version} · Installing {latest}…", { version, latest });
    case "restarting":
      return t("{version} · Restarts into {latest} once no bot is at work", { version, latest });
    case "installed":
      return t("{version} · {latest} is installed; restart lorca serve to run it", { version, latest });
  }
  if (update.latest) return t("{version} · {latest} is available", { version, latest });
  if (update.error) return `${version} · ${update.error}`;
  return update.auto ? t("{version} · Up to date", { version }) : t("{version} · Up to date · automatic updates off", { version });
}

export default function DeviceScreen() {
  useLanguage();
  const { id } = useLocalSearchParams<{ id: string }>();
  const router = useRouter();
  const p = usePalette();
  const device = useStore((s) => s.devices.find((d) => d.id === id));
  const seen = useStore((s) => s.device_seen[id]);
  const allBots = useStore((s) => s.bots);
  const providers = useStore((s) => s.providers);
  const chats = useStore((s) => s.chats);
  const relayConnected = useStore((s) => s.relayConnected);
  const relayUpdateRequired = useStore((s) => s.relayUpdateRequired);
  const relayUrl = useStore((s) => s.relayUrl);
  const [updating, setUpdating] = useState(false);

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
      ? t("It loses its keys and synced chats the next time it connects, and bots assigned to it stop running until you assign them to another Runner. You can pair it again any time.")
      : t("It loses its keys and synced chats the next time it connects. You can pair it again any time.");
    alert(t("Unpair {name}?", { name: deviceName(target) }), detail, [
      { text: t("Cancel"), style: "cancel" },
      {
        text: t("Unpair"),
        style: "destructive",
        onPress: () => {
          engine
            .unpairDevice(target.id)
            .then(() => router.back())
            .catch((error: unknown) => alert(t("Couldn’t unpair {name}", { name: deviceName(target) }), error instanceof Error ? error.message : String(error)));
        },
      },
    ]);
  }

  // A newer release than the CLI runs, not yet on its way: the Lorca CLI row installs it.
  const update = device.update;
  const offersUpdate = !!update?.latest && !update.state;
  function installUpdate(target: Device) {
    setUpdating(true);
    engine
      .updateDevice(target.id)
      .catch((error: unknown) => alert(t("Couldn’t update {name}", { name: deviceName(target) }), error instanceof Error ? error.message : String(error)))
      .finally(() => setUpdating(false));
  }

  return (
    <>
      <Stack.Screen options={{ title: "" }} />
      <ScrollView contentInsetAdjustmentBehavior="automatic" contentContainerStyle={{ paddingBottom: 40 }}>
        <View style={styles.header}>
          <Symbol name={deviceSymbol(device.os, device.model)} size={48} color={p.label} />
          <Text style={[styles.name, { color: p.label }]}>{deviceName(device)}</Text>
          <Text style={[styles.model, { color: p.secondaryLabel }]}>{[device.model, device.os_version].filter(Boolean).join(" · ")}</Text>
          <View style={styles.status}>
            <View style={[styles.dot, { backgroundColor: online ? p.green : p.tertiaryLabel }]} />
            <Text style={[styles.statusText, { color: online ? p.green : p.secondaryLabel }]}>{online ? t("Online") : lastSeen(seen)}</Text>
          </View>
        </View>

        {runner && (
          <Section title={t("Bots assigned here")}>
            {bots.length === 0 ? <Row title={t("No bots assigned")} /> : bots.map((bot) => <Row key={bot.id} title={bot.name} subtitle={providerLabel(bot.provider, providers)} leading={<BotAvatar bot={bot} size={32} />} chevron onPress={() => openChat(bot.id)} />)}
          </Section>
        )}

        {runner && (
          <Section title={t("Plugins")}>
            {(device.plugins ?? []).length === 0 ? (
              <Row title={t("No plugins installed")} />
            ) : (
              (device.plugins ?? []).map((plugin) =>
                // A named account (Gmail · Work) opens its own screen; its name already says which.
                plugin.account_name ? (
                  <Row key={plugin.id} title={plugin.name} detail={pluginStateWord(plugin)} icon={plugin.icon ?? "puzzlepiece.extension"} chevron onPress={() => router.push({ pathname: "/settings/account/[id]", params: { id: plugin.id, runner: device.id } })} />
                ) : (
                  <Row key={plugin.id} title={plugin.name} subtitle={plugin.state === "error" ? plugin.detail || plugin.description : plugin.description} detail={pluginStateWord(plugin)} icon={plugin.icon ?? "puzzlepiece.extension"} chevron onPress={() => router.push({ pathname: "/settings/account/[id]", params: { id: plugin.id, runner: device.id } })} />
                ),
              )
            )}
          </Section>
        )}

        <Section
          title={t("Machine")}
          footer={
            device.unknown
              ? t("This machine is paired to your account but has not sent its name or system. If you don't recognize it, unpair it.")
              : runner
                ? undefined
                : t("{os} Devices hold your keys and chats but never run a bot. Assign bots to a Runner: a Device running macOS, Linux, or Windows.", { os: osName })
          }
        >
          <Row title={t("Machine key")} detail={device.machine_key} />
          {!device.unknown && <Row title={t("OS")} detail={device.os_version || osName} />}
          {update && (
            <Row
              title={t("Lorca CLI")}
              subtitle={cliStatus(device.version ?? "", update)}
              subtitleLines={3}
              onPress={offersUpdate && online && !updating ? () => installUpdate(device) : undefined}
              accessory={
                updating ? (
                  <ActivityIndicator />
                ) : offersUpdate ? (
                  <Text style={[styles.action, { color: online ? p.tint : p.tertiaryLabel }]}>{t("Update")}</Text>
                ) : undefined
              }
            />
          )}
          <Row title={t("Role")} detail={runner ? t("Runner") : t("Device")} />
          <Row title={t("Last seen")} detail={online ? t("Active now") : lastSeen(seen)} />
          <Row title={t("Relay")} detail={relay ? (relayUpdateRequired ? t("{relay} · update Lorca to sync", { relay }) : relayConnected ? relay : t("{relay} · offline", { relay })) : t("Not configured")} />
        </Section>

        {!isThis && (
          <Section>
            <Row title={t("Unpair This Device")} icon="xmark" destructive onPress={() => confirmUnpair(device)} />
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
  action: { fontSize: Font.body },
});
