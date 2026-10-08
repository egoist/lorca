package model

import (
	"reflect"
	"testing"
)

// The same cases as crates/markdown's tests, so the desktop fold and the crate's agree.

func spansText(spans []Span) string { return SpansText(spans) }

func hasSpan(spans []Span, match func(Span) bool) bool {
	for _, span := range spans {
		if match(span) {
			return true
		}
	}
	return false
}

func TestParagraphsKeepLineBreaksAndStyles(t *testing.T) {
	doc := ParseMarkdown("Hello **bold** and *it* with `code`\nsecond line\n\nNext ~~gone~~ [Docs](https://x.y)")
	if len(doc) != 2 {
		t.Fatalf("blocks %d: %+v", len(doc), doc)
	}
	if got := spansText(doc[0].Spans); got != "Hello bold and it with code\nsecond line" {
		t.Errorf("text %q", got)
	}
	if !hasSpan(doc[0].Spans, func(s Span) bool { return s.Bold && s.Text == "bold" }) ||
		!hasSpan(doc[0].Spans, func(s Span) bool { return s.Italic && s.Text == "it" }) ||
		!hasSpan(doc[0].Spans, func(s Span) bool { return s.Code && s.Text == "code" }) {
		t.Errorf("styles %+v", doc[0].Spans)
	}
	if !hasSpan(doc[1].Spans, func(s Span) bool { return s.Strike && s.Text == "gone" }) {
		t.Errorf("strike %+v", doc[1].Spans)
	}
	last := doc[1].Spans[len(doc[1].Spans)-1]
	if last.Link != "https://x.y" || last.Text != "Docs" {
		t.Errorf("link %+v", last)
	}
}

func TestHeadingsCodeQuotesRules(t *testing.T) {
	doc := ParseMarkdown("## Title\n\n```rust\nfn main() {}\n```\n\n> quoted **line**\n> more\n\n---\n")
	if len(doc) != 4 {
		t.Fatalf("blocks %d: %+v", len(doc), doc)
	}
	if doc[0].Kind != BlockHeading || doc[0].Level != 2 || spansText(doc[0].Spans) != "Title" {
		t.Errorf("heading %+v", doc[0])
	}
	if !reflect.DeepEqual(doc[1], Block{Kind: BlockCode, Language: "rust", Text: "fn main() {}"}) {
		t.Errorf("code %+v", doc[1])
	}
	if doc[2].Kind != BlockQuote || spansText(doc[2].Blocks[0].Spans) != "quoted line\nmore" {
		t.Errorf("quote %+v", doc[2])
	}
	if doc[3].Kind != BlockRule {
		t.Errorf("rule %+v", doc[3])
	}
}

func TestLists(t *testing.T) {
	doc := ParseMarkdown("- one\n- two **b**\n  - nested\n\n3. three\n4. four\n\n- [x] done\n- [ ] todo\n")
	bullets := doc[0]
	if bullets.Kind != BlockList || bullets.Ordered || len(bullets.Items) != 2 {
		t.Fatalf("bullets %+v", bullets)
	}
	if spansText(bullets.Items[0].Blocks[0].Spans) != "one" || len(bullets.Items[1].Blocks[1].Items) != 1 {
		t.Errorf("items %+v", bullets.Items)
	}
	numbered := doc[1]
	if !numbered.Ordered || numbered.Start != 3 || len(numbered.Items) != 2 {
		t.Errorf("numbered %+v", numbered)
	}
	tasks := doc[2]
	if tasks.Items[0].Checked == nil || !*tasks.Items[0].Checked || tasks.Items[1].Checked == nil || *tasks.Items[1].Checked {
		t.Errorf("tasks %+v", tasks.Items)
	}
	if got := spansText(tasks.Items[0].Blocks[0].Spans); got != "done" {
		t.Errorf("task text %q", got)
	}
}

func TestTables(t *testing.T) {
	doc := ParseMarkdown("| a | b |\n|---|--:|\n| 1 | **2** |\n")
	table := doc[0]
	if table.Kind != BlockTable || !reflect.DeepEqual(table.Alignments, []Align{AlignAuto, AlignRight}) {
		t.Fatalf("table %+v", table)
	}
	if len(table.Header) != 2 || spansText(table.Header[1]) != "b" || len(table.Rows) != 1 || !table.Rows[0][1][0].Bold {
		t.Errorf("cells %+v", table)
	}
}

func TestBareLinks(t *testing.T) {
	doc := ParseMarkdown("See https://x.y/a, **www.b.org** or me@c.io.\n\n`https://code.y` [named](https://n.y) <https://auto.y>\n\n```\nhttps://block.y\n```\n\n| www.cell.org |\n|---|\n")
	if got := spansText(doc[0].Spans); got != "See https://x.y/a, www.b.org or me@c.io." {
		t.Errorf("text %q", got)
	}
	var links [][3]any
	for _, span := range doc[0].Spans {
		if span.Link != "" {
			links = append(links, [3]any{span.Text, span.Link, span.Bold})
		}
	}
	want := [][3]any{{"https://x.y/a", "https://x.y/a", false}, {"www.b.org", "http://www.b.org", true}, {"me@c.io", "mailto:me@c.io", false}}
	if !reflect.DeepEqual(links, want) {
		t.Errorf("links %v", links)
	}
	second := doc[1].Spans
	if !hasSpan(second, func(s Span) bool { return s.Code && s.Text == "https://code.y" && s.Link == "" }) ||
		!hasSpan(second, func(s Span) bool { return s.Text == "named" && s.Link == "https://n.y" }) ||
		!hasSpan(second, func(s Span) bool { return s.Text == "https://auto.y" && s.Link == "https://auto.y" }) {
		t.Errorf("second %+v", second)
	}
	if !reflect.DeepEqual(doc[2], Block{Kind: BlockCode, Text: "https://block.y"}) {
		t.Errorf("code %+v", doc[2])
	}
	if doc[3].Header[0][0].Link != "http://www.cell.org" {
		t.Errorf("cell %+v", doc[3].Header)
	}
}

func TestPlainAndEmpty(t *testing.T) {
	if doc := ParseMarkdown(""); len(doc) != 0 {
		t.Errorf("empty %+v", doc)
	}
	if doc := ParseMarkdown("just words"); spansText(doc[0].Spans) != "just words" {
		t.Errorf("plain %+v", doc)
	}
	if doc := ParseMarkdown(`a \*b\* &amp; c`); spansText(doc[0].Spans) != "a *b* & c" {
		t.Errorf("escapes %q", spansText(doc[0].Spans))
	}
}
