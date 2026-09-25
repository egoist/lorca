// Colors and type. Each platform uses its own semantic system colors: UIKit colors on iOS and
// Material 3 dynamic colors on Android, including the user's wallpaper palette on Android 12+.

import { Color } from "expo-router";
import { Platform, PlatformColor, useColorScheme, type ColorValue } from "react-native";
import type { Accent } from "../core/model";

export interface Palette {
  label: ColorValue;
  secondaryLabel: ColorValue;
  tertiaryLabel: ColorValue;
  separator: ColorValue;
  background: ColorValue;
  groupedBackground: ColorValue;
  cell: ColorValue;
  fill: ColorValue;
  secondaryFill: ColorValue;
  tint: ColorValue;
  userBubble: ColorValue;
  userBubbleText: ColorValue;
  botBubble: ColorValue;
  botBubbleText: ColorValue;
  code: ColorValue;
  /// `code` over `cell` as one solid color, then that color clear: the ends of the fade at a
  /// scrolling code block's edge. iOS only; Android fades a scroll view's edges itself.
  codeFade?: readonly [string, string];
  green: ColorValue;
  red: ColorValue;
  link: ColorValue;
  dark: boolean;
}

const ios = (name: string) => (Platform.OS === "ios" ? PlatformColor(name) : undefined);

export function usePalette(): Palette {
  const scheme = useColorScheme();
  const dark = scheme === "dark";
  if (Platform.OS === "ios") {
    return {
      label: ios("label")!,
      secondaryLabel: ios("secondaryLabel")!,
      tertiaryLabel: ios("tertiaryLabel")!,
      separator: ios("separator")!,
      background: ios("systemBackground")!,
      groupedBackground: ios("systemGroupedBackground")!,
      cell: ios("secondarySystemGroupedBackground")!,
      fill: ios("systemFill")!,
      secondaryFill: ios("secondarySystemFill")!,
      tint: ios("systemBlue")!,
      userBubble: ios("systemBlue")!,
      userBubbleText: "#FFFFFF",
      botBubble: dark ? "rgba(255,255,255,0.10)" : "#E9E9EB",
      botBubbleText: ios("label")!,
      code: dark ? "rgba(0,0,0,0.28)" : "rgba(0,0,0,0.06)",
      // secondarySystemGroupedBackground (#1C1C1E, #FFFFFF) under `code`.
      codeFade: dark ? ["rgb(20,20,22)", "rgba(20,20,22,0)"] : ["rgb(240,240,240)", "rgba(240,240,240,0)"],
      green: ios("systemGreen")!,
      red: ios("systemRed")!,
      link: ios("link")!,
      dark,
    };
  }
  const material = Color.android.dynamic;
  return {
    label: material.onSurface,
    secondaryLabel: material.onSurfaceVariant,
    tertiaryLabel: material.outline,
    separator: material.outlineVariant,
    background: material.surface,
    groupedBackground: material.surfaceContainerLow,
    cell: material.surfaceContainer,
    fill: material.surfaceContainerHighest,
    secondaryFill: material.secondaryContainer,
    tint: material.primary,
    userBubble: material.primary,
    userBubbleText: material.onPrimary,
    botBubble: material.surfaceContainerHigh,
    botBubbleText: material.onSurface,
    code: material.surfaceContainerHighest,
    green: dark ? "#66DB89" : "#146C2E",
    red: material.error,
    link: material.primary,
    dark,
  };
}

/// The eight bot accents, as the Mac app's system colors, with a lifted twin for gradients.
export const ACCENTS: Record<Accent, { light: [string, string]; dark: [string, string] }> = {
  indigo: { light: ["#8583F5", "#5856D6"], dark: ["#8B89F7", "#5E5CE6"] },
  blue: { light: ["#4DA3FF", "#007AFF"], dark: ["#4FA8FF", "#0A84FF"] },
  teal: { light: ["#5FC5D9", "#30B0C7"], dark: ["#6CD3E6", "#40C8E0"] },
  green: { light: ["#6ADB85", "#34C759"], dark: ["#66E183", "#30D158"] },
  orange: { light: ["#FFB04D", "#FF9500"], dark: ["#FFB454", "#FF9F0A"] },
  pink: { light: ["#FF6B8D", "#FF2D55"], dark: ["#FF6F8F", "#FF375F"] },
  purple: { light: ["#C58CF0", "#AF52DE"], dark: ["#CB94F5", "#BF5AF2"] },
  red: { light: ["#FF7069", "#FF3B30"], dark: ["#FF7A72", "#FF453A"] },
};

export function accentColors(accent: string, dark: boolean): [string, string] {
  const entry = ACCENTS[(accent as Accent) in ACCENTS ? (accent as Accent) : "indigo"];
  return dark ? entry.dark : entry.light;
}

export function accentColor(accent: string, dark: boolean): string {
  return accentColors(accent, dark)[1];
}

export const Font = {
  body: Platform.OS === "android" ? 16 : 17,
  message: Platform.OS === "android" ? 16 : 16.5,
  caption: 12,
  author: Platform.OS === "android" ? 12 : 13,
  small: Platform.OS === "android" ? 12 : 13,
  code: 14,
} as const;
