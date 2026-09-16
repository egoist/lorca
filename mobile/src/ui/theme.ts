// Colors and type. On iOS the semantic colors are the system's own (PlatformColor), so light,
// dark, and accessibility settings follow the OS; Android gets matching hex tokens.

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
      green: ios("systemGreen")!,
      red: ios("systemRed")!,
      link: ios("link")!,
      dark,
    };
  }
  return dark
    ? {
        label: "#F2F2F2",
        secondaryLabel: "rgba(235,235,245,0.6)",
        tertiaryLabel: "rgba(235,235,245,0.3)",
        separator: "rgba(255,255,255,0.15)",
        background: "#000000",
        groupedBackground: "#000000",
        cell: "#1C1C1E",
        fill: "rgba(120,120,128,0.36)",
        secondaryFill: "rgba(120,120,128,0.32)",
        tint: "#0A84FF",
        userBubble: "#0A84FF",
        userBubbleText: "#FFFFFF",
        botBubble: "rgba(255,255,255,0.10)",
        botBubbleText: "#F2F2F2",
        code: "rgba(0,0,0,0.28)",
        green: "#30D158",
        red: "#FF453A",
        link: "#0A84FF",
        dark,
      }
    : {
        label: "#000000",
        secondaryLabel: "rgba(60,60,67,0.6)",
        tertiaryLabel: "rgba(60,60,67,0.3)",
        separator: "rgba(60,60,67,0.29)",
        background: "#FFFFFF",
        groupedBackground: "#F2F2F7",
        cell: "#FFFFFF",
        fill: "rgba(120,120,128,0.2)",
        secondaryFill: "rgba(120,120,128,0.16)",
        tint: "#007AFF",
        userBubble: "#007AFF",
        userBubbleText: "#FFFFFF",
        botBubble: "#E9E9EB",
        botBubbleText: "#000000",
        code: "rgba(0,0,0,0.06)",
        green: "#34C759",
        red: "#FF3B30",
        link: "#007AFF",
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
  body: 17,
  message: 16.5,
  caption: 12,
  author: 13,
  small: 13,
  code: 14,
} as const;
