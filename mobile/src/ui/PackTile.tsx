// A workflow's symbol on a light tile with a hairline, as the Mac draws a plugin without a logo.

import { StyleSheet, View } from "react-native";
import { Symbol } from "./Symbol";
import { usePalette } from "./theme";

export function PackTile({ symbol, size }: { symbol: string; size: number }) {
  const p = usePalette();
  return (
    <View style={[styles.tile, { width: size, height: size, borderRadius: size * 0.25, backgroundColor: p.background, borderColor: p.separator }]}>
      <Symbol name={symbol || "sparkles"} size={Math.round(size * 0.46)} color={p.label} />
    </View>
  );
}

const styles = StyleSheet.create({
  tile: { alignItems: "center", justifyContent: "center", borderWidth: StyleSheet.hairlineWidth },
});
