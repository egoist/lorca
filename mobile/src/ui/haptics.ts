// Haptics by what happened, in each platform's vocabulary. iOS plays its feedback generators;
// Android plays the system's haptic constants, which follow the user's touch-feedback setting and
// are what Material components play (none where Android plays none).

import * as Haptics from "expo-haptics";
import { Platform } from "react-native";

const android = Platform.OS === "android";
const play = (type: Haptics.AndroidHaptics) => void Haptics.performAndroidHapticsAsync(type);

export const haptic = {
  /// A drag crossed the point where letting go acts (swipe to reply).
  threshold: () => (android ? play(Haptics.AndroidHaptics.Segment_Tick) : void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light)),
  /// A long press opened a menu.
  longPress: () => (android ? play(Haptics.AndroidHaptics.Long_Press) : void Haptics.selectionAsync()),
  /// A message went.
  send: () => (android ? play(Haptics.AndroidHaptics.Confirm) : void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Light)),
  /// Recording started.
  start: () => (android ? play(Haptics.AndroidHaptics.Toggle_On) : void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium)),
  /// A sheet or picker opened from a button; Android plays nothing for that.
  open: () => (android ? undefined : void Haptics.selectionAsync()),
  /// Pull to refresh let go; Android's refresh plays nothing.
  refresh: () => (android ? undefined : void Haptics.selectionAsync()),
  success: () => (android ? play(Haptics.AndroidHaptics.Confirm) : void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Success)),
  error: () => (android ? play(Haptics.AndroidHaptics.Reject) : void Haptics.notificationAsync(Haptics.NotificationFeedbackType.Error)),
  /// A code was scanned.
  scanned: () => (android ? play(Haptics.AndroidHaptics.Confirm) : void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium)),
};
