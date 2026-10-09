// Message Markdown, rendered by the native text view: selectable in place, links tappable,
// sized to its text within the width the bubble allows.

import { useMemo } from "react";
import { Platform, processColor, useWindowDimensions, type ColorValue } from "react-native";
import { MarkdownView, measureMarkdown } from "../../modules/lorca-core/MarkdownView";
import { Font, usePalette } from "./theme";

export function Markdown({ text, color, maxWidth, size = Font.message }: { text: string; color: ColorValue; maxWidth: number; size?: number }) {
  const p = usePalette();
  // The text size the user picked: Android's view sizes its text in sp, which follows it already;
  // UIKit's takes points, so iOS scales them here, measuring and drawing alike.
  const { fontScale } = useWindowDimensions();
  const scale = Platform.OS === "ios" ? fontScale : 1;
  const fontSize = size * scale;
  const codeFontSize = Font.code * scale;
  const link = color === p.userBubbleText ? color : p.link;
  const measured = useMemo(() => measureMarkdown(text, maxWidth, fontSize, codeFontSize), [text, maxWidth, fontSize, codeFontSize]);
  return (
    <MarkdownView
      style={measured}
      markdown={text}
      maxWidth={maxWidth}
      fontSize={fontSize}
      codeFontSize={codeFontSize}
      color={processColor(color)!}
      linkColor={processColor(link)!}
      codeBackground={processColor(p.code)!}
      quoteColor={processColor(p.tertiaryLabel)!}
      border={processColor(p.separator)!}
      tint={processColor(link)!}
    />
  );
}
