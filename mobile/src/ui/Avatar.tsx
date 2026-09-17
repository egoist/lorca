// Bot avatars: an accent gradient disc with the bot's SF Symbol, or the bot's own image once
// this phone has it, a breathing green dot while the bot works (as in Grok Bot: 7 px,
// bottom-right, scale 1 → 0.8 over 2.4 s), and a cluster for group chats.

import { Image } from "expo-image";
import { LinearGradient } from "expo-linear-gradient";
import { useEffect } from "react";
import { StyleSheet, View, type StyleProp, type ViewStyle } from "react-native";
import Animated, { Easing, useAnimatedStyle, useSharedValue, withRepeat, withTiming } from "react-native-reanimated";
import { engine } from "../core/engine";
import type { Bot } from "../core/model";
import { useStore } from "../core/store";
import { Symbol } from "./Symbol";
import { accentColors, usePalette } from "./theme";

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

/// The look as a symbol on an accent, or an image; the picker previews a pending choice this way.
export function AvatarDisc({ symbol, accent, uri, size, style }: { symbol: string; accent: string; uri?: string; size: number; style?: StyleProp<ViewStyle> }) {
  const p = usePalette();
  const colors = accentColors(accent, p.dark);
  if (uri) {
    return (
      <View style={[styles.disc, { width: size, height: size, borderRadius: size / 2, overflow: "hidden", backgroundColor: p.fill }, style]}>
        <Image source={{ uri }} contentFit="cover" transition={120} style={{ width: size, height: size }} />
      </View>
    );
  }
  return (
    <LinearGradient colors={colors} start={{ x: 0.3, y: 0 }} end={{ x: 0.7, y: 1 }} style={[styles.disc, { width: size, height: size, borderRadius: size / 2 }, style]}>
      <Symbol name={symbol} size={size * 0.5} color="#FFFFFF" weight="semibold" />
    </LinearGradient>
  );
}

export function BotAvatar({ bot, size = 40, working = false, style }: { bot: Bot | undefined; size?: number; working?: boolean; style?: StyleProp<ViewStyle> }) {
  const p = usePalette();
  const uri = useBotAvatarUri(bot);
  return (
    <View style={[{ width: size, height: size }, style]}>
      <AvatarDisc symbol={bot?.symbol_name ?? "sparkles"} accent={bot?.accent ?? "indigo"} uri={uri} size={size} />
      {working && <PresenceDot size={size} ring={p.background} />}
    </View>
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

/// The presence dot: a green disc with a ring in the surface color, pulsing while working.
export function PresenceDot({ size, ring }: { size: number; ring: any }) {
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
          bottom: -ringWidth * 0.6,
        },
      ]}
    >
      <View style={{ width: dot, height: dot, borderRadius: dot / 2, backgroundColor: p.green }} />
    </Animated.View>
  );
}

/// One to six members. One shows its avatar; two sit diagonally; more stack the first two
/// with the rest implied, the way the Mac sidebar clusters them.
export function AvatarCluster({ bots, size = 40, working = false }: { bots: Bot[]; size?: number; working?: boolean }) {
  const p = usePalette();
  if (bots.length <= 1) return <BotAvatar bot={bots[0]} size={size} working={working} />;
  const small = size * 0.66;
  return (
    <View style={{ width: size, height: size }}>
      <BotAvatar bot={bots[1]} size={small} style={{ position: "absolute", right: 0, top: 0 }} />
      <View style={{ position: "absolute", left: 0, bottom: 0, width: small + 4, height: small + 4, borderRadius: (small + 4) / 2, backgroundColor: p.background, alignItems: "center", justifyContent: "center" }}>
        <BotAvatar bot={bots[0]} size={small} />
      </View>
      {working && <PresenceDot size={size} ring={p.background} />}
    </View>
  );
}

const styles = StyleSheet.create({
  disc: { alignItems: "center", justifyContent: "center" },
  presence: { position: "absolute", alignItems: "center", justifyContent: "center" },
});
