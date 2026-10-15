// A coding agent's mark, where a bot shows its avatar: a symbol per agent, or pi's own letter.

import { StyleSheet, Text, View, type ColorValue } from "react-native";
import { Symbol } from "./Symbol";
import { usePalette } from "./theme";

const SYMBOLS: Record<string, string> = {
  claude: "asterisk",
  codex: "hexagon",
  opencode: "chevron.left.forwardslash.chevron.right",
};

/// `kind` is `claude`, `codex`, `opencode`, or `pi`; an agent not picked yet gets the code mark.
export function AgentLogo({ kind, size, color }: { kind: string; size: number; color?: ColorValue }) {
  const p = usePalette();
  const tint = color ?? p.label;
  if (kind === "pi") {
    return (
      <View style={[styles.box, { width: size, height: size }]}>
        <Text style={{ color: tint, fontSize: size * 0.95, lineHeight: size, fontWeight: "700", includeFontPadding: false }}>π</Text>
      </View>
    );
  }
  return <Symbol name={SYMBOLS[kind] ?? "chevron.left.forwardslash.chevron.right"} size={size} color={tint} weight="semibold" />;
}

const styles = StyleSheet.create({
  box: { alignItems: "center", justifyContent: "center" },
});
