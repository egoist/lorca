// A photo from the transcript, full screen on black: pinch or double-tap to zoom, drag to look
// around. iOS zooms in a UIScrollView, with its bounce; Android follows the fingers on the UI
// thread.

import { Image } from "expo-image";
import { Stack, useLocalSearchParams, useRouter } from "expo-router";
import { Platform, ScrollView, StyleSheet, View } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, { useAnimatedStyle, useSharedValue, withTiming } from "react-native-reanimated";
import { useStore } from "../../src/core/store";
import { t, useLanguage } from "../../src/i18n";
import { IconCloseToolbar } from "../../src/ui/navigation";

const MAX_ZOOM = 4;
const DOUBLE_TAP_ZOOM = 2.5;

export default function AttachmentScreen() {
  useLanguage();
  const { id, name } = useLocalSearchParams<{ id: string; name?: string }>();
  const router = useRouter();
  const uri = useStore((s) => s.files[id]);
  return (
    <View style={styles.screen}>
      <Stack.Screen options={{ title: name ?? t("Photo") }} />
      <IconCloseToolbar label={t("Close")} onClose={() => router.back()} />
      {uri ? Platform.OS === "ios" ? <ScrollZoom uri={uri} /> : <GestureZoom uri={uri} /> : null}
    </View>
  );
}

function ScrollZoom({ uri }: { uri: string }) {
  return (
    <ScrollView
      style={styles.screen}
      contentContainerStyle={styles.fill}
      maximumZoomScale={MAX_ZOOM}
      minimumZoomScale={1}
      centerContent
      showsHorizontalScrollIndicator={false}
      showsVerticalScrollIndicator={false}
      contentInsetAdjustmentBehavior="never"
    >
      <Image source={{ uri }} style={styles.fill} contentFit="contain" />
    </ScrollView>
  );
}

function GestureZoom({ uri }: { uri: string }) {
  // The viewer's own size: the photo is centered in it, under the bar.
  const size = useSharedValue({ width: 0, height: 0 });
  const scale = useSharedValue(1);
  const startScale = useSharedValue(1);
  const x = useSharedValue(0);
  const y = useSharedValue(0);
  const startX = useSharedValue(0);
  const startY = useSharedValue(0);

  // How far the zoomed image may move before its edge leaves the screen's.
  const clamp = (value: number, zoom: number, size: number) => {
    "worklet";
    const room = (size * (zoom - 1)) / 2;
    return Math.min(room, Math.max(-room, value));
  };
  const settle = () => {
    "worklet";
    if (scale.value <= 1) {
      scale.value = withTiming(1);
      x.value = withTiming(0);
      y.value = withTiming(0);
      return;
    }
    x.value = withTiming(clamp(x.value, scale.value, size.value.width));
    y.value = withTiming(clamp(y.value, scale.value, size.value.height));
  };

  const pinch = Gesture.Pinch()
    .onStart(() => {
      startScale.value = scale.value;
      startX.value = x.value;
      startY.value = y.value;
    })
    // The point under the fingers stays under them.
    .onUpdate((event) => {
      const next = Math.min(MAX_ZOOM, Math.max(0.8, startScale.value * event.scale));
      const ratio = next / startScale.value;
      const fx = event.focalX - size.value.width / 2;
      const fy = event.focalY - size.value.height / 2;
      x.value = fx - (fx - startX.value) * ratio;
      y.value = fy - (fy - startY.value) * ratio;
      scale.value = next;
    })
    .onEnd(settle);
  const pan = Gesture.Pan()
    .minPointers(1)
    .averageTouches(true)
    .onStart(() => {
      startX.value = x.value;
      startY.value = y.value;
    })
    .onUpdate((event) => {
      if (scale.value <= 1) return;
      x.value = startX.value + event.translationX;
      y.value = startY.value + event.translationY;
    })
    .onEnd(settle);
  const doubleTap = Gesture.Tap()
    .numberOfTaps(2)
    .onEnd((event) => {
      if (scale.value > 1) {
        scale.value = withTiming(1);
        x.value = withTiming(0);
        y.value = withTiming(0);
        return;
      }
      const fx = event.x - size.value.width / 2;
      const fy = event.y - size.value.height / 2;
      scale.value = withTiming(DOUBLE_TAP_ZOOM);
      x.value = withTiming(clamp(-fx * (DOUBLE_TAP_ZOOM - 1), DOUBLE_TAP_ZOOM, size.value.width));
      y.value = withTiming(clamp(-fy * (DOUBLE_TAP_ZOOM - 1), DOUBLE_TAP_ZOOM, size.value.height));
    });

  const style = useAnimatedStyle(() => ({ transform: [{ translateX: x.value }, { translateY: y.value }, { scale: scale.value }] }));
  return (
    <GestureDetector gesture={Gesture.Simultaneous(pinch, pan, doubleTap)}>
      <View style={styles.fill} onLayout={(e) => (size.value = { width: e.nativeEvent.layout.width, height: e.nativeEvent.layout.height })}>
        <Animated.View style={[styles.fill, style]}>
          <Image source={{ uri }} style={styles.fill} contentFit="contain" />
        </Animated.View>
      </View>
    </GestureDetector>
  );
}

const styles = StyleSheet.create({
  screen: { flex: 1, backgroundColor: "#000000" },
  fill: { flex: 1 },
});
