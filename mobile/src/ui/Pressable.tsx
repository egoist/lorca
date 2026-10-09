// Press feedback as each platform draws it. iOS highlights the element through its `pressed`
// styles. Android ripples instead, bounded to the element (masked to its corners) or round for an
// icon button, and keeps the styles in their resting state so nothing else flashes.

import { forwardRef } from "react";
import { Platform, Pressable as NativePressable, type PressableProps, type PressableStateCallbackType, type View } from "react-native";
import { usePalette, withAlpha } from "./theme";

export type Ripple = "bounded" | "borderless" | "none";

export const Pressable = forwardRef<View, PressableProps & { ripple?: Ripple; rippleRadius?: number }>(function Pressable(
  { ripple = "bounded", rippleRadius, style, android_ripple, ...props },
  ref,
) {
  const p = usePalette();
  if (Platform.OS !== "android") return <NativePressable ref={ref} style={style} {...props} />;
  // Material's pressed state: the content color at 12%.
  const color = typeof p.label === "string" ? withAlpha(p.label, 0.12) : undefined;
  const feedback = android_ripple ?? (ripple === "none" ? undefined : ripple === "borderless" ? { color, borderless: true, radius: rippleRadius ?? 22 } : { color });
  const resting = typeof style === "function" ? (state: PressableStateCallbackType) => style({ ...state, pressed: false }) : style;
  return <NativePressable ref={ref} style={resting} android_ripple={feedback} {...props} />;
});
