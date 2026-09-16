// This phone as the roster sees it. `os` decides the role: phones and tablets are Devices,
// never Runners. Kept apart from pairing.ts so the protocol code runs outside React Native.

import * as Device from "expo-device";
import { Platform } from "react-native";

export interface HostFacts {
  os: string;
  os_version: string;
  model: string;
  name: string;
}

export function hostFacts(): HostFacts {
  const isTablet = Device.deviceType === Device.DeviceType.TABLET;
  const os = Platform.OS === "ios" ? (isTablet ? "ipados" : "ios") : Platform.OS === "android" ? "android" : "ios";
  const osName = Platform.OS === "ios" ? (isTablet ? "iPadOS" : "iOS") : "Android";
  const version = Device.osVersion ?? "";
  const model = Device.modelName ?? (Platform.OS === "ios" ? "iPhone" : "Android");
  const name = Device.deviceName?.trim() || model;
  return { os, os_version: version ? `${osName} ${version}` : osName, model, name };
}
