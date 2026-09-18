// A message body as a native text view. The core parses the Markdown; the view renders it
// selectable in place (UITextView on iOS, TextView on Android), sizes itself to its text within
// `maxWidth`, and opens links.
import { requireNativeViewManager } from "expo-modules-core";
import type { ProcessedColorValue } from "react-native";

export interface MarkdownViewProps {
  markdown: string;
  /// The widest the text may run, in points; the view claims the width it actually uses.
  maxWidth: number;
  fontSize: number;
  codeFontSize: number;
  color: ProcessedColorValue;
  linkColor: ProcessedColorValue;
  codeBackground: ProcessedColorValue;
  quoteColor: ProcessedColorValue;
  /// Table rules.
  border: ProcessedColorValue;
  /// Selection handles and highlight.
  tint: ProcessedColorValue;
}

export const MarkdownView = requireNativeViewManager<MarkdownViewProps>("MarkdownView");
