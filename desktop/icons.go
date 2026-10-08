package main

import (
	"fmt"
	"sync"

	"github.com/egoist/mygo/ui"
)

// Symbols. Bots, plugins, and the macOS app name their looks by SF Symbol ("sparkles",
// "chevron.left.forwardslash.chevron.right"); here each name draws the Lucide icon that says the
// same thing, stroked in the text color.

// filledSymbols are symbols SF draws solid, which the stroke icons fill to match.
var filledSymbols = map[string]bool{"stop.fill": true, "pin.fill": true}

type symbolKey struct {
	name   string
	stroke float32
}

var symbols sync.Map // symbolKey → *ui.SVG

// symbolSVG is the icon for an SF Symbol name, stroked `stroke` wide in its 24-unit box. A name
// with no icon draws sparkles.
func symbolSVG(name string, stroke float32) *ui.SVG {
	key := symbolKey{name, stroke}
	if found, ok := symbols.Load(key); ok {
		return found.(*ui.SVG)
	}
	shape, ok := lucideShapes[symbolIcons[name]]
	if !ok {
		shape = lucideShapes["Sparkles"]
	}
	fill := "none"
	if filledSymbols[name] {
		fill = "currentColor"
	}
	svg := ui.MustParseSVG(fmt.Appendf(nil,
		`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="%s" stroke="currentColor" stroke-width="%g" stroke-linecap="round" stroke-linejoin="round">%s</svg>`,
		fill, stroke, shape))
	symbols.Store(key, svg)
	return svg
}

// symbol draws an SF Symbol name `size` DIPs square in the text color.
func symbol(c *ui.Context, name string, size, stroke float32) ui.Element {
	return ui.Icon(c, symbolSVG(name, stroke)).Size(size, size)
}

var marks sync.Map // plugin id → *ui.SVG

// pluginMark is a plugin's real mark, in its own colors, or nil when it has none.
func pluginMark(id string) *ui.SVG {
	if found, ok := marks.Load(id); ok {
		return found.(*ui.SVG)
	}
	source, ok := pluginMarks[id]
	if !ok {
		return nil
	}
	svg, err := ui.ParseSVG([]byte(source))
	if err != nil {
		return nil
	}
	marks.Store(id, svg)
	return svg
}
