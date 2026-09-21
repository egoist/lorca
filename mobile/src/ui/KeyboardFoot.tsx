import type { ReactNode } from "react";
import type { LayoutChangeEvent, StyleProp, ViewStyle } from "react-native";
import { useReanimatedKeyboardAnimation } from "react-native-keyboard-controller";
import Animated, { useAnimatedStyle } from "react-native-reanimated";

/// A view at the foot of the screen that rides the keyboard. On the keys it drops `tuck` points,
/// the home-indicator padding it no longer needs. It never sinks below where it rests: a hardware
/// or floating keyboard reports open with no height.
export function KeyboardFoot({ tuck, style, children, onLayout }: { tuck: number; style?: StyleProp<ViewStyle>; children?: ReactNode; onLayout?: (e: LayoutChangeEvent) => void }) {
  const { height, progress } = useReanimatedKeyboardAnimation();
  const lift = useAnimatedStyle(() => ({ transform: [{ translateY: Math.min(0, height.value + progress.value * tuck) }] }), [tuck]);
  return (
    <Animated.View style={[style, lift]} onLayout={onLayout} pointerEvents="box-none">
      {children}
    </Animated.View>
  );
}
