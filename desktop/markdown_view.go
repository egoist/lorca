package main

import (
	"net/url"
	"strconv"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// Message Markdown on screen, after the macOS app's MarkdownRenderer: every block a blank line's
// worth from the next (a code box sits closer), list items two points apart, a bar for quotes, a
// grid for a table (inside a list or quote its rows as lines, cells three spaces apart), and a
// rule as a line of box-drawing characters. A message is one selection: a drag runs across its
// paragraphs, lists, and code, and Copy joins them a line apart.

const (
	paragraphGap = 24
	codeGap      = 8
	itemGap      = 2
)

type markdownOptions struct {
	// OnUserBubble keeps links in the bubble's text color on the accent fill.
	OnUserBubble bool
	// Size is the body's text size, textMessage by default.
	Size float32
	// Menu adds items above Copy and Select All in the text's context menu, as a message's Reply.
	Menu func(m *ui.Menu)
}

// openLink opens a link from a message: a web or mail link in the browser, anything else not at all.
func openLink(href string) {
	u, err := url.Parse(href)
	if err != nil {
		return
	}
	switch u.Scheme {
	case "http", "https":
		if u.Host == "" {
			return
		}
	case "mailto":
	default:
		return
	}
	go mygo.Shell.OpenExternal(u.String())
}

// markdownView builds a message body.
func markdownView(c *ui.Context, text string, o markdownOptions) ui.Element {
	if o.Size == 0 {
		o.Size = textMessage
	}
	blocks := model.MarkdownBlocks(text)
	body := ui.Column(c).Selectable().MinWidth(0).FontSize(o.Size).LineHeight(1.38)
	if o.Menu != nil {
		body.ContextMenu(func(m *ui.Menu) {
			o.Menu(m)
			m.Separator()
			m.EditItems()
		})
	}
	body.Children(func() { markdownBlocks(c, blocks, o, false) })
	return body
}

func blockGap(after, before model.Block) float32 {
	if after.Kind == model.BlockCode || before.Kind == model.BlockCode {
		return codeGap
	}
	return paragraphGap
}

func markdownBlocks(c *ui.Context, blocks []model.Block, o markdownOptions, nested bool) {
	for i, block := range blocks {
		top := float32(0)
		if i > 0 {
			top = blockGap(blocks[i-1], block)
		}
		markdownBlock(c, block, o, nested).Margin(top, 0, 0, 0)
	}
}

// spansView builds a paragraph of spans, its links clickable.
func spansView(c *ui.Context, spans []model.Span, o markdownOptions, bold bool) ui.Element {
	p := colors(c)
	para := ui.RichText(c).MinWidth(0)
	para.Children(func() {
		for _, span := range spans {
			t := ui.Text(c, span.Text)
			if span.Bold || bold {
				t.FontWeight(600)
			}
			if span.Italic {
				t.Italic()
			}
			if span.Strike {
				t.Strikethrough()
			}
			if span.Code {
				t.Font(monoFont).FontSize(o.Size - 1).TextBackground(p.Code)
			}
			if span.Link != "" {
				href := span.Link
				t.Underline().Cursor(ui.CursorPointer).Tooltip(href)
				if !o.OnUserBubble {
					t.TextColor(p.Link)
				}
				if t.Clicked() {
					openLink(href)
				}
			}
		}
	})
	return para
}

func taskMarker(ordered bool, number int, checked *bool) string {
	if checked != nil {
		if *checked {
			return "☑"
		}
		return "☐"
	}
	if ordered {
		return strconv.Itoa(number) + "."
	}
	return "•"
}

// cellsLine is a table row as one line, its cells three spaces apart.
func cellsLine(cells [][]model.Span) []model.Span {
	var line []model.Span
	for i, cell := range cells {
		if i > 0 {
			line = append(line, model.PlainSpan("   "))
		}
		line = append(line, cell...)
	}
	return line
}

func markdownBlock(c *ui.Context, block model.Block, o markdownOptions, nested bool) ui.Element {
	p := colors(c)
	switch block.Kind {
	case model.BlockParagraph:
		return spansView(c, block.Spans, o, false)
	case model.BlockHeading:
		h := spansView(c, block.Spans, o, true).FontWeight(700)
		if block.Level <= 2 {
			h.FontSize(o.Size + 2)
		}
		return h
	case model.BlockCode:
		code := ui.Box(c).Padding(8, 10).Radius(7).Background(p.Code).MinWidth(0)
		code.Children(func() {
			text := block.Text
			if text == "" {
				text = " "
			}
			ui.Text(c, text).Font(monoFont).FontSize(12).LineHeight(1.42)
		})
		return code
	case model.BlockList:
		list := ui.Column(c).Gap(itemGap).MinWidth(0)
		list.Children(func() {
			for i, item := range block.Items {
				ui.Row(c).AlignItems(ui.Start).MinWidth(0).Children(func() {
					// The marker stays out of the selection, so a copied list is its items' text.
					ui.Text(c, taskMarker(block.Ordered, block.Start+i, item.Checked)).MinWidth(18).Padding(0, 4, 0, 0).NoWrap().Unselectable()
					ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
						markdownBlocks(c, item.Blocks, o, true)
					})
				})
			}
		})
		return list
	case model.BlockQuote:
		quote := ui.Row(c).AlignItems(ui.Stretch).Gap(8).MinWidth(0)
		quote.Children(func() {
			ui.Box(c).Width(2).Radius(1).Background(p.Label3)
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
				markdownBlocks(c, block.Blocks, o, true)
			})
		})
		return quote
	case model.BlockTable:
		if nested {
			rows := ui.Column(c).Gap(itemGap).MinWidth(0)
			rows.Children(func() {
				spansView(c, cellsLine(block.Header), o, true)
				for _, row := range block.Rows {
					spansView(c, cellsLine(row), o, false)
				}
			})
			return rows
		}
		wrap := ui.ScrollHorizontal(c).MaxWidthPercent(100)
		wrap.Children(func() {
			tracks := make([]ui.Track, len(block.Header))
			for i := range tracks {
				tracks[i] = ui.FitContent()
			}
			ui.Grid(c).ColumnTracks(tracks...).FontSize(o.Size - 0.5).LineHeight(1.35).Children(func() {
				cell := func(spans []model.Span, column int, header, first bool) {
					box := ui.Box(c).Padding(6, 10).MinWidth(44)
					if header {
						box.BorderWidth(0, 0, 1, 0).BorderColor(p.Label3)
					} else if !first {
						box.BorderWidth(1, 0, 0, 0).BorderColor(p.Separator)
					}
					align := ui.Start
					if column < len(block.Alignments) {
						switch block.Alignments[column] {
						case model.AlignCenter:
							align = ui.Center
						case model.AlignRight:
							align = ui.End
						}
					}
					box.Children(func() {
						if len(spans) == 0 {
							spans = []model.Span{model.PlainSpan(" ")}
						}
						spansView(c, spans, o, header).TextAlign(align)
					})
				}
				for column, spans := range block.Header {
					cell(spans, column, true, true)
				}
				for r, row := range block.Rows {
					for column := range block.Header {
						var spans []model.Span
						if column < len(row) {
							spans = row[column]
						}
						cell(spans, column, false, r == 0)
					}
				}
			})
		})
		return wrap
	case model.BlockRule:
		return ui.Text(c, strings.Repeat("─", 24)).TextColor(p.Label3).NoWrap()
	}
	return ui.Box(c)
}
