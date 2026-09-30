// The same cases as crates/markdown's tests, so the desktop fold and the crate's agree.

import { expect, test } from "bun:test";
import { parseMarkdown, type Block, type Span } from "./markdown";

const text = (spans: Span[]) => spans.map((span) => span.text).join("");

test("paragraphs keep their line breaks and inline styles", () => {
  const doc = parseMarkdown("Hello **bold** and *it* with `code`\nsecond line\n\nNext ~~gone~~ [Docs](https://x.y)");
  expect(doc.length).toBe(2);
  const first = doc[0] as Extract<Block, { type: "paragraph" }>;
  expect(first.type).toBe("paragraph");
  expect(text(first.spans)).toBe("Hello bold and it with code\nsecond line");
  expect(first.spans.some((span) => span.bold && span.text === "bold")).toBe(true);
  expect(first.spans.some((span) => span.italic && span.text === "it")).toBe(true);
  expect(first.spans.some((span) => span.code && span.text === "code")).toBe(true);
  const second = doc[1] as Extract<Block, { type: "paragraph" }>;
  expect(second.spans.some((span) => span.strike && span.text === "gone")).toBe(true);
  expect(second.spans[second.spans.length - 1]!.link).toBe("https://x.y");
  expect(second.spans[second.spans.length - 1]!.text).toBe("Docs");
});

test("headings, code, quotes, and rules", () => {
  const doc = parseMarkdown("## Title\n\n```rust\nfn main() {}\n```\n\n> quoted **line**\n> more\n\n---\n");
  expect(doc.length).toBe(4);
  expect(doc[0]).toMatchObject({ type: "heading", level: 2 });
  expect(text((doc[0] as Extract<Block, { type: "heading" }>).spans)).toBe("Title");
  expect(doc[1]).toEqual({ type: "code", language: "rust", text: "fn main() {}" });
  const quote = doc[2] as Extract<Block, { type: "quote" }>;
  expect(quote.type).toBe("quote");
  expect(text((quote.blocks[0] as Extract<Block, { type: "paragraph" }>).spans)).toBe("quoted line\nmore");
  expect(doc[3]).toEqual({ type: "rule" });
});

test("lists: tight, loose, nested, ordered, and tasks", () => {
  const doc = parseMarkdown("- one\n- two **b**\n  - nested\n\n3. three\n4. four\n\n- [x] done\n- [ ] todo\n");
  const bullets = doc[0] as Extract<Block, { type: "listing" }>;
  expect(bullets).toMatchObject({ type: "listing", ordered: false });
  expect(bullets.items.length).toBe(2);
  expect(text((bullets.items[0]!.blocks[0] as Extract<Block, { type: "paragraph" }>).spans)).toBe("one");
  expect((bullets.items[1]!.blocks[1] as Extract<Block, { type: "listing" }>).items.length).toBe(1);
  const numbered = doc[1] as Extract<Block, { type: "listing" }>;
  expect(numbered).toMatchObject({ type: "listing", ordered: true, start: 3 });
  expect(numbered.items.length).toBe(2);
  const tasks = doc[2] as Extract<Block, { type: "listing" }>;
  expect(tasks.items[0]!.checked).toBe(true);
  expect(tasks.items[1]!.checked).toBe(false);
  expect(text((tasks.items[0]!.blocks[0] as Extract<Block, { type: "paragraph" }>).spans)).toBe("done");
});

test("tables", () => {
  const doc = parseMarkdown("| a | b |\n|---|--:|\n| 1 | **2** |\n");
  const table = doc[0] as Extract<Block, { type: "table" }>;
  expect(table.type).toBe("table");
  expect(table.alignments).toEqual(["auto", "right"]);
  expect(table.header.length).toBe(2);
  expect(text(table.header[1]!)).toBe("b");
  expect(table.rows.length).toBe(1);
  expect(table.rows[0]![1]![0]!.bold).toBe(true);
});

test("plain text and empty input survive", () => {
  expect(parseMarkdown("")).toEqual([]);
  const doc = parseMarkdown("just words");
  expect(text((doc[0] as Extract<Block, { type: "paragraph" }>).spans)).toBe("just words");
});
