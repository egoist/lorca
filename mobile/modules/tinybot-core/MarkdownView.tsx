// A message body as a native text view. The core parses the Markdown; the view renders it
// selectable in place (UITextView on iOS, TextView on Android), sizes itself to its text within
// `maxWidth`, and opens links.
import { requireNativeModule, requireNativeViewManager } from "expo-modules-core";
import type { ProcessedColorValue, StyleProp, ViewStyle } from "react-native";

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
  style?: StyleProp<ViewStyle>;
}

const native = requireNativeModule<{ measure?(markdown: string, maxWidth: number, fontSize: number, codeFontSize: number): { width: number; height: number } }>("MarkdownView");

/// The size the text takes within `maxWidth`, measured natively before the view renders. iOS
/// sizes the view from this; on Android the view claims its own size.
export function measureMarkdown(markdown: string, maxWidth: number, fontSize: number, codeFontSize: number) {
  return native.measure?.(markdown, maxWidth, fontSize, codeFontSize);
}

export const MarkdownView = requireNativeViewManager<MarkdownViewProps>("MarkdownView");
