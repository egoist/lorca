// Message Markdown on screen, after the macOS app's MarkdownRenderer: every block a blank line's
// worth from the next (a code box sits closer), list items two points apart, a bar for quotes, a
// grid for a table (inside a list or quote its rows as lines, cells three spaces apart), and a
// rule as a line of box-drawing characters.

import { createMemo } from "solid-js";
import type { JSX } from "@solidjs/web";
import { markdownBlocks, type Block, type Span } from "../model/markdown";
import { host } from "../host";

const paragraphGap = 24;
const codeGap = 8;
const itemGap = 2;

function gap(after: Block, before: Block): number {
  return after.type === "code" || before.type === "code" ? codeGap : paragraphGap;
}

/** Opens a link from a message: a web or mail link in the browser, anything else not at all. */
export function openLink(href: string): void {
  try {
    const url = new URL(href);
    if (url.protocol === "http:" || url.protocol === "https:" || url.protocol === "mailto:") void host.openExternal(url.toString());
  } catch {}
}

function spanNode(span: Span, bold: boolean): JSX.Element {
  let node: JSX.Element = span.text;
  if (span.strike) node = <s>{node}</s>;
  if (span.italic) node = <em>{node}</em>;
  if ((span.bold || bold) && !span.code) node = <strong>{node}</strong>;
  if (span.code) node = <code class={["md-inline-code", { bold: span.bold || bold }]}>{node}</code>;
  if (span.link) {
    const href = span.link;
    node = (
      <a
        href={href}
        title={href}
        onClick={(event) => {
          event.preventDefault();
          openLink(href);
        }}
      >
        {node}
      </a>
    );
  }
  return node;
}

function spans(list: Span[], bold = false): JSX.Element[] {
  return list.map((span) => spanNode(span, bold));
}

interface Context {
  /** Inside a list or a quote, where a table reads as lines. */
  nested: boolean;
}

function marker(ordered: boolean, number: number, checked: boolean | null): string {
  if (checked !== null) return checked ? "☑" : "☐";
  return ordered ? `${number}.` : "•";
}

function cellsLine(cells: Span[][]): Span[] {
  const line: Span[] = [];
  cells.forEach((cell, index) => {
    if (index > 0) line.push({ text: "   ", bold: false, italic: false, code: false, strike: false, link: null });
    line.push(...cell);
  });
  return line;
}

function blockNode(block: Block, context: Context, top: number): JSX.Element {
  const style = top > 0 ? { "margin-top": `${top}px` } : undefined;
  switch (block.type) {
    case "paragraph":
      return (
        <p class="md-p" style={style}>
          {spans(block.spans)}
        </p>
      );
    case "heading":
      return (
        <p class={["md-p", "md-heading", { large: block.level <= 2 }]} style={style}>
          {spans(block.spans, true)}
        </p>
      );
    case "code":
      return (
        <pre class="md-code" style={style}>
          {block.text === "" ? " " : block.text}
        </pre>
      );
    case "listing":
      return (
        <div class="md-list" style={style}>
          {block.items.map((item, index) => (
            <div class="md-item" style={index > 0 ? { "margin-top": `${itemGap}px` } : undefined}>
              <span class={["md-marker", { task: item.checked !== null }]}>{marker(block.ordered, block.start + index, item.checked)}</span>
              <div class="md-item-body">{blocksNodes(item.blocks, { nested: true })}</div>
            </div>
          ))}
        </div>
      );
    case "quote":
      return (
        <div class="md-quote" style={style}>
          {blocksNodes(block.blocks, { nested: true })}
        </div>
      );
    case "table":
      if (context.nested) {
        return (
          <div style={style}>
            <p class="md-p">
              <strong>{spans(cellsLine(block.header))}</strong>
            </p>
            {block.rows.map((row) => (
              <p class="md-p" style={{ "margin-top": `${itemGap}px` }}>
                {spans(cellsLine(row))}
              </p>
            ))}
          </div>
        );
      }
      return (
        <div class="md-table-wrap" style={style}>
          <table class="md-table">
            <thead>
              <tr>
                {block.header.map((cell, column) => (
                  <th style={{ "text-align": align(block.alignments[column]) }}>{cell.length ? spans(cell) : " "}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((row) => (
                <tr>
                  {block.header.map((_, column) => (
                    <td style={{ "text-align": align(block.alignments[column]) }}>{row[column]?.length ? spans(row[column]!) : " "}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case "rule":
      return (
        <p class="md-p md-rule" style={style}>
          {"─".repeat(24)}
        </p>
      );
  }
}

function align(value: string | undefined): JSX.CSSProperties["text-align"] {
  return value === "center" ? "center" : value === "right" ? "right" : "start";
}

function blocksNodes(blocks: Block[], context: Context): JSX.Element[] {
  return blocks.map((block, index) => blockNode(block, context, index === 0 ? 0 : gap(blocks[index - 1]!, block)));
}

/** A message body. `onUserBubble` keeps links in the bubble's text color on the accent fill. */
export function Markdown(props: { text: string; onUserBubble?: boolean; class?: string }) {
  const blocks = createMemo(() => markdownBlocks(props.text));
  return <div class={["md", props.class, { "on-user": !!props.onUserBubble }]}>{blocksNodes(blocks(), { nested: false })}</div>;
}
