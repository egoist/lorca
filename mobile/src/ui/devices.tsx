import { usePathname, useRouter } from "expo-router";
import type { ReactElement } from "react";
import { StyleSheet, View } from "react-native";
import { engine } from "../core/engine";
import { deviceName, isRunner } from "../core/model";
import { deviceIsOnline, useStore } from "../core/store";
import { t, useLanguage } from "../i18n";
import { lastSeen } from "./format";
import { Row } from "./forms";
import { Symbol } from "./Symbol";
import { usePalette } from "./theme";

// The SF Symbol for a Device, the way the Mac sidebar picks it.
export function deviceSymbol(os: string, model: string): string {
  switch (os) {
    case "macos":
      if (model.includes("MacBook")) return "laptopcomputer";
      if (model.includes("Studio")) return "macstudio";
      if (model.includes("mini")) return "macmini";
      return "desktopcomputer";
    case "linux":
      return "server.rack";
    case "windows":
      return "pc";
    case "ios":
      return "iphone";
    case "ipados":
      return "ipad";
    // A machine the relay lists that never said what it is.
    case "":
      return "questionmark.circle";
    default:
      return "smartphone";
  }
}

/// Where a Device's screen and the screens it slides in live: the Devices tab's stack on a phone,
/// the settings sheet's beside the sidebar.
export function useDevicesBase(): "/devices" | "/settings" {
  return usePathname().startsWith("/settings") ? "/settings" : "/devices";
}

/// The account's Devices, this phone first and machines that never said what they are last, each
/// a row that slides in its screen. Rows, not one element, so a Section separates them.
export function useDeviceRows(): ReactElement[] {
  useLanguage();
  const router = useRouter();
  const base = useDevicesBase();
  const p = usePalette();
  const devices = useStore((s) => s.devices);
  const seen = useStore((s) => s.device_seen);
  const thisId = engine.deviceId;
  const sorted = [...devices].sort((a, b) =>
    a.id === thisId ? -1 : b.id === thisId ? 1 : Number(!!a.unknown) - Number(!!b.unknown) || a.name.localeCompare(b.name),
  );
  return sorted.map((device) => {
    const online = device.id === thisId || deviceIsOnline(device.id);
    return (
      <Row
        key={device.id}
        title={device.id === thisId ? t("{name} (this phone)", { name: device.name }) : deviceName(device)}
        onPress={() => router.push(`${base}/device/${device.id}`)}
        chevron
        subtitle={[device.model, isRunner(device) ? t("Runner") : t("Device"), online ? t("Online") : lastSeen(seen[device.id])].filter(Boolean).join(" · ")}
        leading={
          <View style={styles.icon}>
            <Symbol name={deviceSymbol(device.os, device.model)} size={22} color={p.label} />
            <View style={[styles.dot, { backgroundColor: online ? p.green : p.tertiaryLabel, borderColor: p.cell }]} />
          </View>
        }
      />
    );
  });
}

const styles = StyleSheet.create({
  icon: { width: 32, alignItems: "center" },
  dot: { position: "absolute", right: 0, bottom: -2, width: 10, height: 10, borderRadius: 5, borderWidth: 2 },
});
