// Bot avatars: the bot's character (Grok Bot's, a shape flat in its accent with two white eyes),
// or the bot's own image once this phone has it, a breathing green dot while the bot works (as
// in Grok Bot: 7 px, bottom-right, scale 1 → 0.8 over 2.4 s), a cluster for group chats, and a
// channel conversation's service mark on its corner. The shapes come from the desktop app's
// outlines as white masks this view tints (characters.ts).

import { Image } from "expo-image";
import { LinearGradient } from "expo-linear-gradient";
import { memo, useEffect } from "react";
import { StyleSheet, Text, View, type ColorValue, type StyleProp, type ViewStyle } from "react-native";
import Animated, { Easing, useAnimatedStyle, useSharedValue, withRepeat, withTiming } from "react-native-reanimated";
import { engine } from "../core/engine";
import type { Bot } from "../core/model";
import { useStore } from "../core/store";
import { t } from "../i18n";
import { Pressable } from "./Pressable";
import { BOT_SHAPES, type BotShape, EYE_LENGTH, EYE_TILT, EYE_WIDTH, SHAPE_EYES, SHAPE_MASKS } from "./characters";
import { Symbol } from "./Symbol";
import { accentColor, usePalette } from "./theme";

/// The bot's shape: the one it was given, or one its id picks (FNV-1a, as the desktop app's
/// `shapeFor`), so bots told apart by nothing else still look different.
export function shapeFor(shape: string | undefined, id: string): BotShape {
  if (shape && (BOT_SHAPES as readonly string[]).includes(shape)) return shape as BotShape;
  let hash = 2166136261;
  for (let i = 0; i < id.length; i++) {
    hash ^= id.charCodeAt(i);
    hash = Math.imul(hash, 16777619) >>> 0;
  }
  return BOT_SHAPES[hash % BOT_SHAPES.length];
}

/// The character `size` across: the shape's mask in `color`, the eyes over it in white.
export function Character({ shape, color, size, style }: { shape: BotShape; color: string; size: number; style?: StyleProp<ViewStyle> }) {
  const half = size / 2;
  const width = EYE_WIDTH * half;
  const length = EYE_LENGTH * half;
  return (
    <View style={[{ width: size, height: size }, style]}>
      <Image source={SHAPE_MASKS[shape]} tintColor={color} style={{ width: size, height: size }} accessible={false} />
      {SHAPE_EYES[shape].map(([x, y], i) => (
        <View
          key={i}
          style={{
            position: "absolute",
            left: half + x * half - width / 2,
            top: half + y * half - length / 2,
            width,
            height: length,
            borderRadius: width / 2,
            backgroundColor: "#FFFFFF",
            transform: [{ rotate: `${-EYE_TILT}deg` }],
          }}
        />
      ))}
    </View>
  );
}

/// The file URI of the bot's profile image, once this phone has the bytes; asks the core for
/// them the first time. Undefined while they are on their way or when the bot has none.
export function useBotAvatarUri(bot: Bot | undefined): string | undefined {
  const avatar = bot?.avatar;
  const uri = useStore((s) => (avatar ? s.files[avatar.id] : undefined));
  useEffect(() => {
    if (avatar && !uri) void engine.fetchFile(avatar);
  }, [avatar, uri]);
  return avatar ? uri : undefined;
}

export function shapeTitle(shape: BotShape): string {
  switch (shape) {
    case "blob":
      return t("Blob");
    case "pebble":
      return t("Pebble");
    case "squircle":
      return t("Squircle");
    case "cloud":
      return t("Cloud");
    case "drop":
      return t("Drop");
    case "sparkle":
      return t("Sparkle");
    case "clover":
      return t("Clover");
    case "flower":
      return t("Flower");
    case "heart":
      return t("Heart");
    case "hex":
      return t("Hexagon");
    case "star":
      return t("Star");
    case "cushion":
      return t("Cushion");
  }
}

/// The shapes to pick from, each a character in `accent`, the picked one outlined in it.
export function ShapePicker({ shape, accent, onPick }: { shape: BotShape; accent: string; onPick: (shape: BotShape) => void }) {
  const p = usePalette();
  const color = accentColor(accent, p.dark);
  return (
    <View style={styles.shapeGrid}>
      {BOT_SHAPES.map((s) => (
        <Pressable
          key={s}
          onPress={() => onPick(s)}
          style={[styles.shapeCell, { backgroundColor: p.fill, borderColor: s === shape ? color : "transparent" }]}
          accessibilityLabel={shapeTitle(s)}
          accessibilityRole="button"
          accessibilityState={{ selected: s === shape }}
        >
          <Character shape={s} color={color} size={30} />
        </Pressable>
      ))}
    </View>
  );
}

/// The look as a character, or an image in a circle; the picker previews a pending choice this way.
export function LookAvatar({ shape, accent, uri, size, style }: { shape: BotShape; accent: string; uri?: string; size: number; style?: StyleProp<ViewStyle> }) {
  const p = usePalette();
  if (uri) {
    return (
      <View style={[styles.disc, { width: size, height: size, borderRadius: size / 2, overflow: "hidden", backgroundColor: p.fill }, style]}>
        <Image source={{ uri }} contentFit="cover" transition={120} style={{ width: size, height: size }} />
      </View>
    );
  }
  return <Character shape={shape} color={accentColor(accent, p.dark)} size={size} style={style} />;
}

export const BotAvatar = memo(function BotAvatar({ bot, size = 40, working = false, service, style }: { bot: Bot | undefined; size?: number; working?: boolean; service?: string; style?: StyleProp<ViewStyle> }) {
  const p = usePalette();
  const uri = useBotAvatarUri(bot);
  return (
    <View style={[{ width: size, height: size }, style]}>
      <LookAvatar shape={shapeFor(bot?.shape, bot?.id ?? "")} accent={bot?.accent ?? "indigo"} uri={uri} size={size} />
      {service && <ServiceBadge service={service} size={size} ring={p.background} />}
      {working && <PresenceDot size={size} ring={p.background} top={!!service} />}
    </View>
  );
});

/// The services' real marks as round badges, rendered from the Mac's PluginLogo SVGs:
/// Telegram's disc as it is, Slack's mark on white.
const SERVICE_MARKS: Record<string, number> = {
  telegram: require("../../assets/services/telegram.png"),
  slack: require("../../assets/services/slack.png"),
};

/// A channel conversation's service mark over the avatar's bottom-right edge, ringed in the
/// surface color.
function ServiceBadge({ service, size, ring }: { service: string; size: number; ring: any }) {
  const mark = SERVICE_MARKS[service];
  if (!mark) return null;
  const badge = Math.round(size * 0.4);
  const ringWidth = Math.max(1.5, badge * 0.1);
  const outer = badge + ringWidth * 2;
  return (
    <View style={[styles.badge, { width: outer, height: outer, borderRadius: outer / 2, backgroundColor: ring, right: -ringWidth, bottom: -ringWidth }]}>
      <Image source={mark} style={{ width: badge, height: badge }} />
    </View>
  );
}

/// Someone outside Lorca: the first letter of their name on gray.
export function ContactAvatar({ name, size = 40 }: { name: string; size?: number }) {
  const initial = Array.from(name.trim())[0]?.toUpperCase() ?? "?";
  return (
    <LinearGradient colors={["#B0B0B5", "#8E8E93"]} style={[styles.disc, { width: size, height: size, borderRadius: size / 2 }]}>
      <Text style={{ color: "#FFFFFF", fontSize: Math.round(size * 0.46), fontWeight: "600" }} allowFontScaling={false}>
        {initial}
      </Text>
    </LinearGradient>
  );
}

export function YouAvatar({ size = 40 }: { size?: number }) {
  const p = usePalette();
  return (
    <View style={[styles.disc, { width: size, height: size, borderRadius: size / 2, backgroundColor: p.fill }]}>
      <Symbol name="person.fill" size={size * 0.5} color={p.secondaryLabel} />
    </View>
  );
}

/// The presence dot: a green disc with a ring in the surface color, pulsing while working, at the
/// top-right when a service's mark holds the bottom-right.
/// Memoized: an animated style re-rendered reads its shared values back from the UI thread.
export const PresenceDot = memo(function PresenceDot({ size, ring, top = false }: { size: number; ring: any; top?: boolean }) {
  const p = usePalette();
  const scale = useSharedValue(1);
  useEffect(() => {
    scale.value = withRepeat(withTiming(0.8, { duration: 1200, easing: Easing.inOut(Easing.ease) }), -1, true);
  }, [scale]);
  const animated = useAnimatedStyle(() => ({ transform: [{ scale: scale.value }] }));
  const dot = Math.max(7, Math.round(size * 0.27));
  const ringWidth = Math.max(1.5, dot * 0.22);
  return (
    <Animated.View
      style={[
        styles.presence,
        animated,
        {
          width: dot + ringWidth * 2,
          height: dot + ringWidth * 2,
          borderRadius: (dot + ringWidth * 2) / 2,
          backgroundColor: ring,
          right: -ringWidth * 0.6,
          ...(top ? { top: -ringWidth * 0.6 } : { bottom: -ringWidth * 0.6 }),
        },
      ]}
    >
      <View style={{ width: dot, height: dot, borderRadius: dot / 2, backgroundColor: p.green }} />
    </Animated.View>
  );
});

/// One to six members. One shows its avatar; two sit diagonally; more stack the first two
/// with the rest implied, the way the Mac sidebar clusters them.
/// `service` is a channel conversation's (telegram, slack), whose mark the avatar wears.
export const AvatarCluster = memo(function AvatarCluster({ bots, size = 40, working = false, service }: { bots: Bot[]; size?: number; working?: boolean; service?: string }) {
  const p = usePalette();
  if (bots.length <= 1) return <BotAvatar bot={bots[0]} size={size} working={working} service={service} />;
  const small = size * 0.66;
  return (
    <View style={{ width: size, height: size }}>
      <BotAvatar bot={bots[1]} size={small} style={{ position: "absolute", right: 0, top: 0 }} />
      <RingedAvatar bot={bots[0]} size={small} ring={2} color={p.background} style={{ position: "absolute", left: 0, bottom: 0 }} />
      {working && <PresenceDot size={size} ring={p.background} />}
    </View>
  );
});

/// A bot's avatar standing off what is behind it by `ring` of `color`: the character's mask a
/// little larger, or a circle around an image. A mask takes only a plain color, so a semantic
/// iOS color stands in as the system background it is.
function RingedAvatar({ bot, size, ring, color, style }: { bot: Bot | undefined; size: number; ring: number; color: ColorValue; style?: StyleProp<ViewStyle> }) {
  const p = usePalette();
  const uri = useBotAvatarUri(bot);
  const outer = size + ring * 2;
  const tint = typeof color === "string" ? color : p.dark ? "#000000" : "#FFFFFF";
  return (
    <View style={[{ width: outer, height: outer, alignItems: "center", justifyContent: "center" }, style]}>
      {uri ? (
        <View style={{ position: "absolute", width: outer, height: outer, borderRadius: outer / 2, backgroundColor: color }} />
      ) : (
        <Image source={SHAPE_MASKS[shapeFor(bot?.shape, bot?.id ?? "")]} tintColor={tint} style={{ position: "absolute", width: outer, height: outer }} accessible={false} />
      )}
      <BotAvatar bot={bot} size={size} />
    </View>
  );
}

const styles = StyleSheet.create({
  disc: { alignItems: "center", justifyContent: "center" },
  presence: { position: "absolute", alignItems: "center", justifyContent: "center" },
  badge: { position: "absolute", alignItems: "center", justifyContent: "center" },
  shapeGrid: { flexDirection: "row", flexWrap: "wrap", gap: 10, padding: 14 },
  shapeCell: { width: 48, height: 44, borderRadius: 10, borderWidth: 2, alignItems: "center", justifyContent: "center" },
});
