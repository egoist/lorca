// A coding agent's mark, where a bot shows its avatar: the agents' real marks, rendered from the
// SVGs the desktop app draws (desktop/pluginmarks.go). Claude Code's spark keeps its orange and a
// bot's own sub agent shows Lorca's app icon (assets/splash-icon.png, scaled down); the one-color
// marks (Codex's OpenAI knot, OpenCode's, pi's, fx's) take the text color they are given.

import { Image } from "expo-image";
import { type ColorValue } from "react-native";
import { Symbol } from "./Symbol";
import { usePalette } from "./theme";

const MARKS: Record<string, number> = {
  claude: require("../../assets/agents/claude.png"),
  codex: require("../../assets/agents/codex.png"),
  opencode: require("../../assets/agents/opencode.png"),
  pi: require("../../assets/agents/pi.png"),
  fx: require("../../assets/agents/fx.png"),
  lorca: require("../../assets/agents/lorca.png"),
};

/// Marks drawn in their own colors, which no tint covers.
const COLORED = new Set(["claude", "lorca"]);

/// `kind` is `claude`, `codex`, `opencode`, `pi`, `fx`, or `lorca`; an agent not picked yet gets the code mark.
export function AgentLogo({ kind, size, color }: { kind: string; size: number; color?: ColorValue }) {
  const p = usePalette();
  const tint = color ?? p.label;
  const mark = MARKS[kind];
  if (!mark) return <Symbol name="chevron.left.forwardslash.chevron.right" size={size} color={tint} weight="semibold" />;
  return (
    <Image
      source={mark}
      style={{ width: size, height: size }}
      tintColor={COLORED.has(kind) ? undefined : (tint as string)}
      contentFit="contain"
      accessible={false}
    />
  );
}
