// Message Markdown, rendered by the native text view: selectable in place, links tappable,
// sized to its text within the width the bubble allows.

import { useMemo } from "react";
import { processColor, type ColorValue } from "react-native";
import { MarkdownView, measureMarkdown } from "../../modules/lorca-core/MarkdownView";
import { Font, usePalette } from "./theme";

export function Markdown({ text, color, maxWidth, size = Font.message }: { text: string; color: ColorValue; maxWidth: number; size?: number }) {
  const p = usePalette();
  const link = color === p.userBubbleText ? color : p.link;
  const measured = useMemo(() => measureMarkdown(text, maxWidth, size, Font.code), [text, maxWidth, size]);
  return (
    <MarkdownView
      style={measured}
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
