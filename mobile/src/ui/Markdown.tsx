// A small Markdown renderer for replies: paragraphs, headings, lists, fenced code, and inline
// bold, italic, code, and links. Enough for what a bot says; nothing fancier.

import { Linking, Platform, StyleSheet, Text, View, type ColorValue } from "react-native";
import { Font, usePalette } from "./theme";

type Block =
  | { type: "paragraph"; text: string }
  | { type: "heading"; text: string; level: number }
  | { type: "code"; text: string }
  | { type: "list"; ordered: boolean; items: string[] }
  | { type: "quote"; text: string };

function parseBlocks(source: string): Block[] {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim() === "") {
      i++;
      continue;
    }
    if (line.trimStart().startsWith("```")) {
      const code: string[] = [];
      i++;
      while (i < lines.length && !lines[i].trimStart().startsWith("```")) code.push(lines[i++]);
      i++;
      blocks.push({ type: "code", text: code.join("\n") });
      continue;
    }
    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      blocks.push({ type: "heading", level: heading[1].length, text: heading[2] });
      i++;
      continue;
    }
    if (/^\s*([-*•]|\d+[.)])\s+/.test(line)) {
      const ordered = /^\s*\d+[.)]\s+/.test(line);
      const items: string[] = [];
      while (i < lines.length && /^\s*([-*•]|\d+[.)])\s+/.test(lines[i])) {
        let item = lines[i].replace(/^\s*([-*•]|\d+[.)])\s+/, "");
        i++;
        while (i < lines.length && /^\s{2,}\S/.test(lines[i]) && !/^\s*([-*•]|\d+[.)])\s+/.test(lines[i])) {
          item += " " + lines[i].trim();
          i++;
        }
        items.push(item);
      }
      blocks.push({ type: "list", ordered, items });
      continue;
    }
    if (line.startsWith(">")) {
      const quote: string[] = [];
      while (i < lines.length && lines[i].startsWith(">")) quote.push(lines[i++].replace(/^>\s?/, ""));
      blocks.push({ type: "quote", text: quote.join("\n") });
      continue;
    }
    const para: string[] = [line];
    i++;
    while (i < lines.length && lines[i].trim() !== "" && !lines[i].trimStart().startsWith("```") && !/^(#{1,6})\s/.test(lines[i]) && !/^\s*([-*•]|\d+[.)])\s+/.test(lines[i]) && !lines[i].startsWith(">")) {
      para.push(lines[i++]);
    }
    blocks.push({ type: "paragraph", text: para.join("\n") });
  }
  return blocks;
}

type Span = { text: string; bold?: boolean; italic?: boolean; code?: boolean; link?: string };

const INLINE = /(\*\*[^*]+\*\*|__[^_]+__|`[^`]+`|\[[^\]]+\]\([^)]+\)|\*[^*\n]+\*|_[^_\n]+_)/g;

function parseInline(text: string): Span[] {
  const spans: Span[] = [];
  let last = 0;
  for (const match of text.matchAll(INLINE)) {
    const start = match.index ?? 0;
    if (start > last) spans.push({ text: text.slice(last, start) });
    const token = match[0];
    if (token.startsWith("**") || token.startsWith("__")) spans.push({ text: token.slice(2, -2), bold: true });
    else if (token.startsWith("`")) spans.push({ text: token.slice(1, -1), code: true });
    else if (token.startsWith("[")) {
      const m = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(token);
      if (m) spans.push({ text: m[1], link: m[2] });
      else spans.push({ text: token });
    } else spans.push({ text: token.slice(1, -1), italic: true });
    last = start + token.length;
  }
  if (last < text.length) spans.push({ text: text.slice(last) });
  return spans;
}

const MONO = Platform.select({ ios: "Menlo", android: "monospace", default: "monospace" });

function Inline({ text, color, size, codeBackground, link }: { text: string; color: ColorValue; size: number; codeBackground: ColorValue; link: ColorValue }) {
  return (
    <Text style={{ color, fontSize: size, lineHeight: size * 1.35 }} selectable>
      {parseInline(text).map((span, index) => (
        <Text
          key={index}
          onPress={span.link ? () => Linking.openURL(span.link!).catch(() => {}) : undefined}
          style={[
            span.bold && { fontWeight: "600" },
            span.italic && { fontStyle: "italic" },
            span.code && { fontFamily: MONO, fontSize: size - 1.5, backgroundColor: codeBackground, borderRadius: 4 },
            span.link && { color: link, textDecorationLine: "underline" },
          ]}
        >
          {span.text}
        </Text>
      ))}
    </Text>
  );
}

export function Markdown({ text, color, size = Font.message }: { text: string; color: ColorValue; size?: number }) {
  const p = usePalette();
  const blocks = parseBlocks(text);
  return (
    <View style={styles.stack}>
      {blocks.map((block, index) => {
        switch (block.type) {
          case "paragraph":
            return <Inline key={index} text={block.text} color={color} size={size} codeBackground={p.code} link={color === p.userBubbleText ? color : p.link} />;
          case "heading":
            return (
              <Text key={index} style={{ color, fontSize: block.level <= 2 ? size + 2 : size, fontWeight: "700", lineHeight: (size + 2) * 1.3 }}>
                {block.text}
              </Text>
            );
          case "code":
            return (
              <View key={index} style={[styles.code, { backgroundColor: p.code }]}>
                <Text selectable style={{ color, fontFamily: MONO, fontSize: Font.code, lineHeight: Font.code * 1.4 }}>
                  {block.text}
                </Text>
              </View>
            );
          case "quote":
            return (
              <View key={index} style={[styles.quote, { borderLeftColor: p.tertiaryLabel }]}>
                <Inline text={block.text} color={color} size={size} codeBackground={p.code} link={p.link} />
              </View>
            );
          case "list":
            return (
              <View key={index} style={styles.list}>
                {block.items.map((item, n) => (
                  <View key={n} style={styles.item}>
                    <Text style={{ color, fontSize: size, lineHeight: size * 1.35, width: block.ordered ? 22 : 16 }}>{block.ordered ? `${n + 1}.` : "•"}</Text>
                    <View style={{ flex: 1 }}>
                      <Inline text={item} color={color} size={size} codeBackground={p.code} link={p.link} />
                    </View>
                  </View>
                ))}
              </View>
            );
        }
      })}
    </View>
  );
}

const styles = StyleSheet.create({
  stack: { gap: 8 },
  code: { borderRadius: 8, paddingHorizontal: 10, paddingVertical: 8 },
  quote: { borderLeftWidth: 2, paddingLeft: 10 },
  list: { gap: 3 },
  item: { flexDirection: "row" },
});
