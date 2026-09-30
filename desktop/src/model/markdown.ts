// Markdown for message bodies, folded into the block structure every Lorca app renders: the same
// Block and Span shapes as `crates/markdown` (pulldown-cmark), which the macOS and phone apps
// link. Here markdown-it reads the text (CommonMark, with tables and strikethrough; task list
// markers are read as pulldown-cmark reads them), and the fold follows the crate's.

import MarkdownIt, { type Token } from "markdown-it";

/** A run of text with one style. */
export interface Span {
  text: string;
  bold: boolean;
  italic: boolean;
  code: boolean;
  strike: boolean;
  link: string | null;
}

export interface ListItem {
  blocks: Block[];
  /** A task item's box; null for a plain item. */
  checked: boolean | null;
}

export type Align = "auto" | "left" | "center" | "right";

export type Block =
  | { type: "paragraph"; spans: Span[] }
  | { type: "heading"; level: number; spans: Span[] }
  | { type: "code"; language: string | null; text: string }
  | { type: "listing"; ordered: boolean; start: number; items: ListItem[] }
  | { type: "quote"; blocks: Block[] }
  | { type: "table"; alignments: Align[]; header: Span[][]; rows: Span[][][] }
  | { type: "rule" };

const parser = new MarkdownIt("commonmark", { html: true, linkify: false }).enable(["table", "strikethrough"]);

interface Style {
  bold: number;
  italic: number;
  strike: number;
  links: string[];
}

class Builder {
  /** Block lists being filled: the document, then each open quote or list item. */
  private containers: Block[][] = [[]];
  private lists: { ordered: boolean; start: number; items: ListItem[]; checked: boolean | null }[] = [];
  private table: { alignments: Align[]; header: Span[][]; rows: Span[][][]; row: Span[][]; inHead: boolean } | null = null;
  private spans: Span[] = [];
  private style: Style = { bold: 0, italic: 0, strike: 0, links: [] };

  finish(): Block[] {
    while (this.containers.length > 1) {
      const blocks = this.containers.pop()!;
      this.push({ type: "quote", blocks });
    }
    return this.containers.pop() ?? [];
  }

  private push(block: Block): void {
    this.containers[this.containers.length - 1]!.push(block);
  }

  run(tokens: Token[]): void {
    for (const token of tokens) this.block(token);
  }

  private block(token: Token): void {
    switch (token.type) {
      case "paragraph_open":
      case "heading_open":
        this.spans = [];
        break;
      case "paragraph_close":
        this.push({ type: "paragraph", spans: this.takeSpans() });
        break;
      case "heading_close":
        this.push({ type: "heading", level: Number(token.tag.slice(1)) || 1, spans: this.takeSpans() });
        break;
      case "inline":
        this.inline(token.children ?? []);
        break;
      case "fence":
      case "code_block": {
        const language = token.type === "fence" ? token.info.trim().split(/\s+/)[0] || null : null;
        this.push({ type: "code", language, text: token.content.replace(/\n+$/, "") });
        break;
      }
      case "blockquote_open":
        this.containers.push([]);
        break;
      case "blockquote_close":
        if (this.containers.length > 1) {
          const blocks = this.containers.pop()!;
          this.push({ type: "quote", blocks });
        }
        break;
      case "bullet_list_open":
      case "ordered_list_open": {
        const start = Number(token.attrGet("start") ?? 1);
        this.lists.push({ ordered: token.type === "ordered_list_open", start: Number.isFinite(start) ? start : 1, items: [], checked: null });
        break;
      }
      case "bullet_list_close":
      case "ordered_list_close": {
        const list = this.lists.pop();
        if (list) this.push({ type: "listing", ordered: list.ordered, start: list.start, items: list.items });
        break;
      }
      case "list_item_open":
        this.containers.push([]);
        break;
      case "list_item_close":
        if (this.containers.length > 1) {
          const blocks = this.containers.pop()!;
          const list = this.lists[this.lists.length - 1];
          if (list) {
            list.items.push({ blocks, checked: list.checked });
            list.checked = null;
          }
        }
        break;
      case "table_open":
        this.table = { alignments: [], header: [], rows: [], row: [], inHead: false };
        break;
      case "thead_open":
        if (this.table) this.table.inHead = true;
        break;
      case "thead_close":
        if (this.table) {
          this.table.header = this.table.row;
          this.table.row = [];
          this.table.inHead = false;
        }
        break;
      case "th_open":
      case "td_open":
        this.spans = [];
        if (this.table?.inHead) {
          const style = String(token.attrGet("style") ?? "");
          const align = /text-align:\s*(left|center|right)/.exec(style)?.[1] as Align | undefined;
          this.table.alignments.push(align ?? "auto");
        }
        break;
      case "th_close":
      case "td_close":
        this.table?.row.push(this.takeSpans());
        break;
      case "tr_close":
        if (this.table && !this.table.inHead) {
          this.table.rows.push(this.table.row);
          this.table.row = [];
        }
        break;
      case "table_close":
        if (this.table) {
          const { alignments, header, rows } = this.table;
          this.push({ type: "table", alignments, header, rows });
          this.table = null;
        }
        break;
      case "hr":
        this.push({ type: "rule" });
        break;
      case "html_block":
        // pulldown-cmark hands an HTML block over a line at a time, each its own paragraph.
        for (const line of token.content.replace(/\n+$/, "").split("\n")) {
          this.push({ type: "paragraph", spans: [plain(line)] });
        }
        break;
    }
  }

  private inline(children: Token[]): void {
    const list = this.lists[this.lists.length - 1];
    // A task item's box: "[ ] " or "[x] " opening the item's first line.
    if (list && this.containers[this.containers.length - 1]!.length === 0 && this.spans.length === 0) {
      const first = children[0];
      const marker = first?.type === "text" ? /^\[([ xX])\][ \t]+/.exec(first.content) : null;
      if (first && marker && list.checked === null) {
        list.checked = marker[1] !== " ";
        first.content = first.content.slice(marker[0].length);
      }
    }
    for (const token of children) {
      switch (token.type) {
        case "text":
          this.text(token.content, false);
          break;
        case "code_inline":
          this.text(token.content, true);
          break;
        case "softbreak":
        case "hardbreak":
          this.text("\n", false);
          break;
        case "html_inline":
          this.text(token.content, false);
          break;
        case "strong_open":
          this.style.bold++;
          break;
        case "strong_close":
          this.style.bold = Math.max(0, this.style.bold - 1);
          break;
        case "em_open":
          this.style.italic++;
          break;
        case "em_close":
          this.style.italic = Math.max(0, this.style.italic - 1);
          break;
        case "s_open":
          this.style.strike++;
          break;
        case "s_close":
          this.style.strike = Math.max(0, this.style.strike - 1);
          break;
        case "link_open":
          this.style.links.push(String(token.attrGet("href") ?? ""));
          break;
        case "link_close":
          this.style.links.pop();
          break;
        case "image":
          // An image reads as its words, linked to its source, as pulldown-cmark folds it.
          this.style.links.push(String(token.attrGet("src") ?? ""));
          this.inline(token.children ?? []);
          this.style.links.pop();
          break;
      }
    }
  }

  private text(text: string, code: boolean): void {
    if (text === "") return;
    const span: Span = {
      text,
      bold: this.style.bold > 0,
      italic: this.style.italic > 0,
      code,
      strike: this.style.strike > 0,
      link: this.style.links[this.style.links.length - 1] ?? null,
    };
    const last = this.spans[this.spans.length - 1];
    if (last && sameStyle(last, span)) last.text += span.text;
    else this.spans.push(span);
  }

  /** The leaf's spans, trimmed at both ends so a stray line break never opens or closes a block. */
  private takeSpans(): Span[] {
    const spans = this.spans;
    this.spans = [];
    if (spans[0]) spans[0].text = spans[0].text.trimStart();
    const last = spans[spans.length - 1];
    if (last) last.text = last.text.trimEnd();
    return spans.filter((span) => span.text !== "");
  }
}

function sameStyle(a: Span, b: Span): boolean {
  return a.bold === b.bold && a.italic === b.italic && a.code === b.code && a.strike === b.strike && a.link === b.link;
}

export function plain(text: string): Span {
  return { text, bold: false, italic: false, code: false, strike: false, link: null };
}

/** Parses a message body. Tables, strikethrough, and task lists are on; everything else is
 * CommonMark. */
export function parseMarkdown(text: string): Block[] {
  const builder = new Builder();
  builder.run(parser.parse(text, {}));
  return builder.finish();
}

/** The words as shown, without the Markdown: a table row by row, its cells between commas. */
export function plainText(blocks: Block[]): string {
  const spansText = (spans: Span[]) => spans.map((span) => span.text).join("");
  const lines: string[] = [];
  const walk = (block: Block) => {
    switch (block.type) {
      case "paragraph":
      case "heading":
        lines.push(spansText(block.spans));
        break;
      case "code":
        lines.push(block.text);
        break;
      case "listing":
        for (const item of block.items) item.blocks.forEach(walk);
        break;
      case "quote":
        block.blocks.forEach(walk);
        break;
      case "table":
        for (const row of [block.header, ...block.rows]) lines.push(row.map(spansText).join(", "));
        break;
      case "rule":
        break;
    }
  };
  blocks.forEach(walk);
  return lines.join("\n").trim();
}

const cache = new Map<string, Block[]>();

/** The parse of `text`, kept for the texts on screen: a transcript renders a message again as it
 * streams, and the rest of its messages come back unchanged. */
export function markdownBlocks(text: string): Block[] {
  let blocks = cache.get(text);
  if (!blocks) {
    blocks = parseMarkdown(text);
    if (cache.size > 2000) cache.clear();
    cache.set(text, blocks);
  }
  return blocks;
}
