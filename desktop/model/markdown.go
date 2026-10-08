package model

import (
	"strings"
	"sync"
	"unicode"

	"github.com/yuin/goldmark"
	"github.com/yuin/goldmark/ast"
	"github.com/yuin/goldmark/extension"
	east "github.com/yuin/goldmark/extension/ast"
	"github.com/yuin/goldmark/text"
	"github.com/yuin/goldmark/util"
)

// Markdown for message bodies, folded into the block structure every Lorca app renders: the same
// Block and Span shapes as `crates/markdown` (pulldown-cmark), which the macOS and phone apps
// link. Here goldmark reads the text (CommonMark, with tables, strikethrough, and task lists), and
// the fold follows the crate's, bare links included.

// Span is a run of text with one style.
type Span struct {
	Text   string
	Bold   bool
	Italic bool
	Code   bool
	Strike bool
	// Link is where the run goes; empty for none.
	Link string
}

func (s Span) sameStyle(o Span) bool {
	return s.Bold == o.Bold && s.Italic == o.Italic && s.Code == o.Code && s.Strike == o.Strike && s.Link == o.Link
}

// PlainSpan is text with no style.
func PlainSpan(text string) Span { return Span{Text: text} }

type ListItem struct {
	Blocks []Block
	// Checked is a task item's box; nil for a plain item.
	Checked *bool
}

type Align int

const (
	AlignAuto Align = iota
	AlignLeft
	AlignCenter
	AlignRight
)

type BlockKind int

const (
	BlockParagraph BlockKind = iota
	BlockHeading
	BlockCode
	BlockList
	BlockQuote
	BlockTable
	BlockRule
)

// Block is one block of a message: what its Kind says it is, with that kind's fields.
type Block struct {
	Kind BlockKind
	// Spans of a paragraph or a heading.
	Spans []Span
	// Level of a heading, 1 to 6.
	Level int
	// Language and Text of a code block.
	Language string
	Text     string
	// A list's numbering and items.
	Ordered bool
	Start   int
	Items   []ListItem
	// Blocks of a quote.
	Blocks []Block
	// A table's columns, header, and rows.
	Alignments []Align
	Header     [][]Span
	Rows       [][][]Span
}

var markdownParser = goldmark.New(
	goldmark.WithExtensions(extension.Table, extension.Strikethrough, extension.TaskList),
)

type style struct {
	bold, italic, strike int
	links                []string
}

type folder struct {
	source []byte
	spans  []Span
	style  style
}

// ParseMarkdown parses a message body. Tables, strikethrough, task lists, and bare links (GitHub's
// autolinks) are on; everything else is CommonMark.
func ParseMarkdown(markdown string) []Block {
	source := []byte(markdown)
	doc := markdownParser.Parser().Parse(text.NewReader(source))
	f := &folder{source: source}
	return f.blocks(doc)
}

func (f *folder) blocks(parent ast.Node) []Block {
	var out []Block
	for node := parent.FirstChild(); node != nil; node = node.NextSibling() {
		out = append(out, f.block(node)...)
	}
	return out
}

func (f *folder) lines(node ast.Node) string {
	var b strings.Builder
	lines := node.Lines()
	for i := 0; i < lines.Len(); i++ {
		segment := lines.At(i)
		b.Write(segment.Value(f.source))
	}
	return b.String()
}

func (f *folder) block(node ast.Node) []Block {
	switch n := node.(type) {
	case *ast.Paragraph, *ast.TextBlock:
		spans := f.inlines(n)
		if len(spans) == 0 && n.ChildCount() == 0 {
			return nil
		}
		return []Block{{Kind: BlockParagraph, Spans: spans}}
	case *ast.Heading:
		return []Block{{Kind: BlockHeading, Level: n.Level, Spans: f.inlines(n)}}
	case *ast.FencedCodeBlock:
		language := ""
		if n.Info != nil {
			language = string(n.Language(f.source))
		}
		return []Block{{Kind: BlockCode, Language: language, Text: strings.TrimRight(f.lines(n), "\n")}}
	case *ast.CodeBlock:
		return []Block{{Kind: BlockCode, Text: strings.TrimRight(f.lines(n), "\n")}}
	case *ast.Blockquote:
		return []Block{{Kind: BlockQuote, Blocks: f.blocks(n)}}
	case *ast.List:
		block := Block{Kind: BlockList, Ordered: n.IsOrdered(), Start: 1}
		if n.IsOrdered() {
			block.Start = n.Start
		}
		for item := n.FirstChild(); item != nil; item = item.NextSibling() {
			block.Items = append(block.Items, f.listItem(item))
		}
		return []Block{block}
	case *ast.ThematicBreak:
		return []Block{{Kind: BlockRule}}
	case *ast.HTMLBlock:
		// pulldown-cmark hands an HTML block over a line at a time, each its own paragraph.
		raw := f.lines(n)
		if n.HasClosure() {
			raw += string(n.ClosureLine.Value(f.source))
		}
		var out []Block
		for _, line := range strings.Split(strings.TrimRight(raw, "\n"), "\n") {
			out = append(out, Block{Kind: BlockParagraph, Spans: []Span{PlainSpan(line)}})
		}
		return out
	case *east.Table:
		block := Block{Kind: BlockTable}
		for _, alignment := range n.Alignments {
			switch alignment {
			case east.AlignLeft:
				block.Alignments = append(block.Alignments, AlignLeft)
			case east.AlignCenter:
				block.Alignments = append(block.Alignments, AlignCenter)
			case east.AlignRight:
				block.Alignments = append(block.Alignments, AlignRight)
			default:
				block.Alignments = append(block.Alignments, AlignAuto)
			}
		}
		for row := n.FirstChild(); row != nil; row = row.NextSibling() {
			var cells [][]Span
			for cell := row.FirstChild(); cell != nil; cell = cell.NextSibling() {
				cells = append(cells, f.inlines(cell))
			}
			if _, header := row.(*east.TableHeader); header {
				block.Header = cells
			} else {
				block.Rows = append(block.Rows, cells)
			}
		}
		return []Block{block}
	}
	return nil
}

func (f *folder) listItem(item ast.Node) ListItem {
	var out ListItem
	// A task item's box opens its first paragraph.
	if first := item.FirstChild(); first != nil {
		if box, ok := first.FirstChild().(*east.TaskCheckBox); ok {
			checked := box.IsChecked
			out.Checked = &checked
			first.RemoveChild(first, box)
		}
	}
	out.Blocks = f.blocks(item)
	return out
}

// inlines folds a leaf's inline content into spans, trimmed at both ends so a stray line break
// never opens or closes a block, with their bare links made links.
func (f *folder) inlines(leaf ast.Node) []Span {
	f.spans = nil
	f.style = style{}
	for node := leaf.FirstChild(); node != nil; node = node.NextSibling() {
		f.inline(node)
	}
	spans := f.spans
	f.spans = nil
	if len(spans) > 0 {
		spans[0].Text = strings.TrimLeftFunc(spans[0].Text, unicode.IsSpace)
		spans[len(spans)-1].Text = strings.TrimRightFunc(spans[len(spans)-1].Text, unicode.IsSpace)
	}
	kept := spans[:0]
	for _, span := range spans {
		if span.Text != "" {
			kept = append(kept, span)
		}
	}
	return linkify(kept)
}

func unescape(value []byte) string {
	value = util.ResolveEntityNames(value)
	value = util.ResolveNumericReferences(value)
	return string(util.UnescapePunctuations(value))
}

func (f *folder) children(node ast.Node) {
	for child := node.FirstChild(); child != nil; child = child.NextSibling() {
		f.inline(child)
	}
}

func (f *folder) inline(node ast.Node) {
	switch n := node.(type) {
	case *ast.Text:
		f.text(unescape(n.Segment.Value(f.source)), false)
		if n.SoftLineBreak() || n.HardLineBreak() {
			f.text("\n", false)
		}
	case *ast.String:
		f.text(string(n.Value), false)
	case *ast.CodeSpan:
		var b strings.Builder
		for child := n.FirstChild(); child != nil; child = child.NextSibling() {
			switch c := child.(type) {
			case *ast.Text:
				b.Write(c.Segment.Value(f.source))
			case *ast.String:
				b.Write(c.Value)
			}
		}
		f.text(b.String(), true)
	case *ast.Emphasis:
		if n.Level >= 2 {
			f.style.bold++
			f.children(n)
			f.style.bold--
		} else {
			f.style.italic++
			f.children(n)
			f.style.italic--
		}
	case *east.Strikethrough:
		f.style.strike++
		f.children(n)
		f.style.strike--
	case *ast.Link:
		f.style.links = append(f.style.links, string(n.Destination))
		f.children(n)
		f.style.links = f.style.links[:len(f.style.links)-1]
	case *ast.AutoLink:
		f.style.links = append(f.style.links, string(n.URL(f.source)))
		f.text(string(n.Label(f.source)), false)
		f.style.links = f.style.links[:len(f.style.links)-1]
	case *ast.Image:
		// An image reads as its words, linked to its source, as pulldown-cmark folds it.
		f.style.links = append(f.style.links, string(n.Destination))
		f.children(n)
		f.style.links = f.style.links[:len(f.style.links)-1]
	case *ast.RawHTML:
		var b strings.Builder
		for i := 0; i < n.Segments.Len(); i++ {
			segment := n.Segments.At(i)
			b.Write(segment.Value(f.source))
		}
		f.text(b.String(), false)
	default:
		f.children(node)
	}
}

func (f *folder) text(value string, code bool) {
	if value == "" {
		return
	}
	span := Span{Text: value, Bold: f.style.bold > 0, Italic: f.style.italic > 0, Code: code, Strike: f.style.strike > 0}
	if n := len(f.style.links); n > 0 {
		span.Link = f.style.links[n-1]
	}
	if n := len(f.spans); n > 0 && f.spans[n-1].sameStyle(span) {
		f.spans[n-1].Text += span.Text
		return
	}
	f.spans = append(f.spans, span)
}

// linkify splits each span of words that is not code or a link already around the bare links in it.
func linkify(spans []Span) []Span {
	var out []Span
	for _, span := range spans {
		var links []Autolink
		if !span.Code && span.Link == "" {
			links = FindAutolinks(span.Text)
		}
		if len(links) == 0 {
			out = append(out, span)
			continue
		}
		at := 0
		for _, link := range links {
			if link.Start > at {
				piece := span
				piece.Text = span.Text[at:link.Start]
				out = append(out, piece)
			}
			piece := span
			piece.Text, piece.Link = span.Text[link.Start:link.End], link.Href
			out = append(out, piece)
			at = link.End
		}
		if at < len(span.Text) {
			piece := span
			piece.Text = span.Text[at:]
			out = append(out, piece)
		}
	}
	return out
}

// SpansText is the words of spans, without their styles.
func SpansText(spans []Span) string {
	var b strings.Builder
	for _, span := range spans {
		b.WriteString(span.Text)
	}
	return b.String()
}

// PlainText is the words as shown, without the Markdown: a table row by row, its cells between
// commas.
func PlainText(blocks []Block) string {
	var lines []string
	var walk func(block Block)
	walk = func(block Block) {
		switch block.Kind {
		case BlockParagraph, BlockHeading:
			lines = append(lines, SpansText(block.Spans))
		case BlockCode:
			lines = append(lines, block.Text)
		case BlockList:
			for _, item := range block.Items {
				for _, inner := range item.Blocks {
					walk(inner)
				}
			}
		case BlockQuote:
			for _, inner := range block.Blocks {
				walk(inner)
			}
		case BlockTable:
			for _, row := range append([][][]Span{block.Header}, block.Rows...) {
				cells := make([]string, 0, len(row))
				for _, cell := range row {
					cells = append(cells, SpansText(cell))
				}
				lines = append(lines, strings.Join(cells, ", "))
			}
		}
	}
	for _, block := range blocks {
		walk(block)
	}
	return strings.TrimSpace(strings.Join(lines, "\n"))
}

var (
	markdownMu    sync.Mutex
	markdownCache = map[string][]Block{}
)

// MarkdownBlocks is the parse of `text`, kept for the texts on screen: a transcript builds a
// message again every frame, and the rest of its messages come back unchanged.
func MarkdownBlocks(text string) []Block {
	markdownMu.Lock()
	defer markdownMu.Unlock()
	if blocks, ok := markdownCache[text]; ok {
		return blocks
	}
	blocks := ParseMarkdown(text)
	if len(markdownCache) > 2000 {
		clear(markdownCache)
	}
	markdownCache[text] = blocks
	return blocks
}
