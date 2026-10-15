// Rows of one height that reorder by dragging their grip, inside a `Section`'s card: the row
// dragged follows the finger above the others, which slide into the places it leaves. Letting
// go keeps the new order (`onMove`, with the keys in it), and the rows follow `keys` again once
// the order comes back from where it is kept.

import { useEffect, type ReactNode } from "react";
import { StyleSheet, View } from "react-native";
import { Gesture, GestureDetector } from "react-native-gesture-handler";
import Animated, { runOnJS, useAnimatedStyle, useSharedValue, withTiming, type SharedValue } from "react-native-reanimated";
import { t } from "../i18n";
import { Symbol } from "./Symbol";
import { usePalette } from "./theme";

type Places = Record<string, number>;

function placesOf(keys: string[]): Places {
  const places: Places = {};
  keys.forEach((key, index) => (places[key] = index));
  return places;
}

/// `keys` in order, each row drawn by `row`, `height` tall; `label` names a row for its grip.
export function ReorderableRows({
  keys,
  height,
  row,
  label,
  onMove,
}: {
  keys: string[];
  height: number;
  row: (key: string) => ReactNode;
  label: (key: string) => string;
  onMove: (keys: string[]) => void;
}) {
  const p = usePalette();
  const places = useSharedValue<Places>(placesOf(keys));
  const dragging = useSharedValue<string | null>(null);
  const order = keys.join("\n");
  useEffect(() => {
    places.value = placesOf(keys);
    // The order is the keys joined; the array is new on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [order]);
  const finish = (moved: Places) => {
    const next = [...keys].sort((a, b) => moved[a] - moved[b]);
    if (next.join("\n") !== order) onMove(next);
  };
  return (
    <View style={{ height: height * keys.length }}>
      {keys.map((key) => (
        <Row key={key} id={key} count={keys.length} height={height} places={places} dragging={dragging} onDrop={finish} label={label(key)}>
          {row(key)}
        </Row>
      ))}
      {/* The lines between places stay put while rows slide over them. */}
      {keys.slice(1).map((key, index) => (
        <View key={`line-${key}`} pointerEvents="none" style={[styles.separator, { top: height * (index + 1), backgroundColor: p.separator }]} />
      ))}
    </View>
  );
}

function Row({
  id,
  count,
  height,
  places,
  dragging,
  onDrop,
  label,
  children,
}: {
  id: string;
  count: number;
  height: number;
  places: SharedValue<Places>;
  dragging: SharedValue<string | null>;
  onDrop: (places: Places) => void;
  label: string;
  children: ReactNode;
}) {
  const p = usePalette();
  const y = useSharedValue(0);
  const start = useSharedValue(0);
  const touch = useSharedValue(0);
  // The row moves as far as the finger has from where it touched down: the pan's translation
  // leaves out the first few points it waits before it starts, so the row would trail the finger.
  const grip = Gesture.Pan()
    .onBegin((event) => {
      touch.value = event.absoluteY;
    })
    .onStart(() => {
      dragging.value = id;
      start.value = places.value[id] * height;
      y.value = start.value;
    })
    .onUpdate((event) => {
      y.value = Math.min(Math.max(start.value + event.absoluteY - touch.value, 0), (count - 1) * height);
      const place = Math.round(y.value / height);
      const from = places.value[id];
      if (place === from) return;
      // The row in the place it moves to takes the place it leaves.
      const next = { ...places.value };
      for (const key of Object.keys(next)) {
        if (next[key] === place) next[key] = from;
      }
      next[id] = place;
      places.value = next;
    })
    .onFinalize(() => {
      if (dragging.value !== id) return;
      dragging.value = null;
      runOnJS(onDrop)(places.value);
    });
  const style = useAnimatedStyle(() => {
    const lifted = dragging.value === id;
    return {
      top: lifted ? y.value : withTiming(places.value[id] * height, { duration: 180 }),
      zIndex: lifted ? 2 : 0,
      transform: [{ scale: withTiming(lifted ? 1.02 : 1, { duration: 120 }) }],
      shadowOpacity: withTiming(lifted ? 0.18 : 0, { duration: 120 }),
    };
  });
  return (
    <Animated.View style={[styles.row, { height, backgroundColor: p.cell, shadowColor: "#000" }, style]}>
      <View style={styles.content}>{children}</View>
      <GestureDetector gesture={grip}>
        <View style={styles.grip} accessible accessibilityLabel={t("Reorder {name}", { name: label })} hitSlop={8}>
          <Symbol name="line.3.horizontal" size={18} color={p.tertiaryLabel} />
        </View>
      </GestureDetector>
    </Animated.View>
  );
}

const styles = StyleSheet.create({
  row: { position: "absolute", left: 0, right: 0, flexDirection: "row", alignItems: "center", shadowRadius: 8, shadowOffset: { width: 0, height: 2 } },
  content: { flex: 1, flexDirection: "row", alignItems: "center" },
  grip: { paddingHorizontal: 14, alignSelf: "stretch", justifyContent: "center" },
  separator: { position: "absolute", left: 16, right: 0, height: StyleSheet.hairlineWidth, zIndex: 1 },
});
