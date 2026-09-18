// Message Markdown, rendered by the native text view: selectable in place, links tappable,
// sized to its text within the width the bubble allows.

import { processColor, type ColorValue } from "react-native";
import { MarkdownView } from "../../modules/tinybot-core/MarkdownView";
import { Font, usePalette } from "./theme";

export function Markdown({ text, color, maxWidth, size = Font.message }: { text: string; color: ColorValue; maxWidth: number; size?: number }) {
  const p = usePalette();
  const link = color === p.userBubbleText ? color : p.link;
  return (
    <MarkdownView
      markdown={text}
      maxWidth={maxWidth}
      fontSize={size}
      codeFontSize={Font.code}
      color={processColor(color)!}
      linkColor={processColor(link)!}
      codeBackground={processColor(p.code)!}
      quoteColor={processColor(p.tertiaryLabel)!}
      border={processColor(p.separator)!}
      tint={processColor(link)!}
    />
  );
}
