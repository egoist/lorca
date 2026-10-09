package main

import (
	"math"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The app's colors and type, after the macOS app's Theme: semantic label and fill colors, the
// system accent colors bots take, and the bubble, code, chip, and composer fills, light and dark.

type palette struct {
	Label, Label2, Label3, Label4 ui.Color
	Separator, Placeholder, Link  ui.Color

	Accent, AccentText ui.Color

	Indigo, Blue, Teal, Green, Orange, Pink, Purple, Red ui.Color

	Window, Sidebar, Inspector, Content, ToolbarLine ui.Color

	BotBubble, BotBubbleBorder, Code, CodeHover, Chip, Card, CardBorder ui.Color

	ComposerField, ComposerBorder, ComposerControl, ComposerPrimary, ComposerPrimaryContent ui.Color
	// GlassButton fills a round glass button over its frosted backdrop, as AppKit's glass bezel.
	GlassButton ui.Color

	Hover, Pressed, ButtonBG                      ui.Color
	Selection, SelectionText, SelectionInactive   ui.Color
	Field, FieldBorder, Popover, Scrim, Sheet     ui.Color
	SearchBG, RowHover, Shadow, ShadowEdge, White ui.Color
}

var lightPalette = palette{
	Label:       ui.RGBA(0, 0, 0, 0.86),
	Label2:      ui.RGBA(0, 0, 0, 0.52),
	Label3:      ui.RGBA(0, 0, 0, 0.3),
	Label4:      ui.RGBA(0, 0, 0, 0.12),
	Separator:   ui.RGBA(0, 0, 0, 0.1),
	Placeholder: ui.RGBA(0, 0, 0, 0.3),
	Link:        ui.Hex("#0068da"),

	Accent:     ui.Hex("#007aff"),
	AccentText: ui.Hex("#ffffff"),

	Indigo: ui.Hex("#5856d6"),
	Blue:   ui.Hex("#007aff"),
	Teal:   ui.Hex("#30b0c7"),
	Green:  ui.Hex("#34c759"),
	Orange: ui.Hex("#ff9500"),
	Pink:   ui.Hex("#ff2d55"),
	Purple: ui.Hex("#af52de"),
	Red:    ui.Hex("#ff3b30"),

	Window:      ui.Hex("#ececed"),
	Sidebar:     ui.Hex("#f3f3f4"),
	Inspector:   ui.Hex("#f6f6f7"),
	Content:     ui.Hex("#ffffff"),
	ToolbarLine: ui.RGBA(0, 0, 0, 0.09),

	BotBubble:       ui.Hex("#f1f1f1"),
	BotBubbleBorder: ui.RGBA(0, 0, 0, 0.04),
	Code:            ui.RGBA(0, 0, 0, 0.06),
	CodeHover:       ui.RGBA(0, 0, 0, 0.1),
	Chip:            ui.RGBA(0, 0, 0, 0.05),
	Card:            ui.Hex("#f1f1f2"),
	CardBorder:      ui.RGBA(0, 0, 0, 0.05),

	ComposerField:          ui.RGBA(255, 255, 255, 0.86),
	ComposerBorder:         ui.RGBA(0, 0, 0, 0.12),
	ComposerControl:        ui.RGBA(0, 0, 0, 0.07),
	ComposerPrimary:        ui.Hex("#000000"),
	ComposerPrimaryContent: ui.Hex("#ffffff"),
	GlassButton:            ui.RGBA(250, 250, 250, 0.9),

	Hover:             ui.RGBA(0, 0, 0, 0.06),
	Pressed:           ui.RGBA(0, 0, 0, 0.12),
	ButtonBG:          ui.RGBA(0, 0, 0, 0.08),
	Selection:         ui.Hex("#0064e1"),
	SelectionText:     ui.Hex("#ffffff"),
	SelectionInactive: ui.RGBA(0, 0, 0, 0.1),
	Field:             ui.Hex("#ffffff"),
	FieldBorder:       ui.RGBA(0, 0, 0, 0.15),
	Popover:           ui.Hex("#ffffff"),
	Scrim:             ui.RGBA(0, 0, 0, 0.22),
	Sheet:             ui.Hex("#ffffff"),
	SearchBG:          ui.RGBA(0, 0, 0, 0.055),
	RowHover:          ui.RGBA(0, 0, 0, 0.04),
	Shadow:            ui.RGBA(0, 0, 0, 0.18),
	ShadowEdge:        ui.RGBA(0, 0, 0, 0.12),
	White:             ui.Hex("#ffffff"),
}

var darkPalette = palette{
	Label:       ui.RGBA(255, 255, 255, 0.87),
	Label2:      ui.RGBA(255, 255, 255, 0.55),
	Label3:      ui.RGBA(255, 255, 255, 0.28),
	Label4:      ui.RGBA(255, 255, 255, 0.12),
	Separator:   ui.RGBA(255, 255, 255, 0.1),
	Placeholder: ui.RGBA(255, 255, 255, 0.3),
	Link:        ui.Hex("#4ea1ff"),

	Accent:     ui.Hex("#0a84ff"),
	AccentText: ui.Hex("#ffffff"),

	Indigo: ui.Hex("#5e5ce6"),
	Blue:   ui.Hex("#0a84ff"),
	Teal:   ui.Hex("#40cbe0"),
	Green:  ui.Hex("#30d158"),
	Orange: ui.Hex("#ff9f0a"),
	Pink:   ui.Hex("#ff375f"),
	Purple: ui.Hex("#bf5af2"),
	Red:    ui.Hex("#ff453a"),

	Window:      ui.Hex("#262628"),
	Sidebar:     ui.Hex("#232325"),
	Inspector:   ui.Hex("#222224"),
	Content:     ui.Hex("#1c1c1c"),
	ToolbarLine: ui.RGBA(255, 255, 255, 0.08),

	BotBubble:       ui.RGBA(255, 255, 255, 0.1),
	BotBubbleBorder: ui.RGBA(255, 255, 255, 0.06),
	Code:            ui.RGBA(0, 0, 0, 0.24),
	CodeHover:       ui.RGBA(0, 0, 0, 0.34),
	Chip:            ui.RGBA(255, 255, 255, 0.08),
	Card:            ui.RGBA(255, 255, 255, 0.06),
	CardBorder:      ui.RGBA(255, 255, 255, 0.06),

	ComposerField:          ui.RGBA(46, 46, 48, 0.86),
	ComposerBorder:         ui.RGBA(255, 255, 255, 0.14),
	ComposerControl:        ui.RGBA(255, 255, 255, 0.12),
	ComposerPrimary:        ui.Hex("#ffffff"),
	ComposerPrimaryContent: ui.Hex("#000000"),
	GlassButton:            ui.RGBA(64, 64, 68, 0.9),

	Hover:             ui.RGBA(255, 255, 255, 0.07),
	Pressed:           ui.RGBA(255, 255, 255, 0.14),
	ButtonBG:          ui.RGBA(255, 255, 255, 0.12),
	Selection:         ui.Hex("#0a64d8"),
	SelectionText:     ui.Hex("#ffffff"),
	SelectionInactive: ui.RGBA(255, 255, 255, 0.1),
	Field:             ui.RGBA(255, 255, 255, 0.06),
	FieldBorder:       ui.RGBA(255, 255, 255, 0.14),
	Popover:           ui.Hex("#2c2c2e"),
	Scrim:             ui.RGBA(0, 0, 0, 0.4),
	Sheet:             ui.Hex("#262628"),
	SearchBG:          ui.RGBA(255, 255, 255, 0.08),
	RowHover:          ui.RGBA(255, 255, 255, 0.05),
	Shadow:            ui.RGBA(0, 0, 0, 0.5),
	ShadowEdge:        ui.RGBA(255, 255, 255, 0.14),
	White:             ui.Hex("#ffffff"),
}

// Text sizes, in DIPs.
const (
	textMessage = 13.5
	textBody    = 13
	textSmall   = 12
	textCaption = 10.5
	textNotice  = 11.5
)

// monoFont is the system's monospaced family.
const monoFont = "monospace"

// colors is the palette of the appearance the view draws in, in the desktop's accent color when
// it has one, as the Mac app follows the system's.
func colors(c *ui.Context) *palette {
	base := &lightPalette
	if c.Theme().Dark {
		base = &darkPalette
	}
	accent := c.Preferences().Accent
	if accent.A == 0 {
		return base
	}
	key := accentKey{dark: base == &darkPalette, r: accent.R, g: accent.G, b: accent.B}
	if p, ok := accentPalettes[key]; ok {
		return p
	}
	p := *base
	p.Accent = ui.RGB(accent.R, accent.G, accent.B)
	// The selected row's fill is the accent a shade darker, as macOS's is.
	p.Selection = p.Accent.Mix(ui.RGB(0, 0, 0), 0.12)
	if luminance(p.Accent) > 0.6 {
		p.AccentText, p.SelectionText = ui.Hex("#000000"), ui.Hex("#000000")
	}
	accentPalettes[key] = &p
	return &p
}

type accentKey struct {
	dark    bool
	r, g, b uint8
}

// accentPalettes are the palettes made for the desktop's accents, on the main thread.
var accentPalettes = map[accentKey]*palette{}

// luminance is a color's relative luminance, 0 for black and 1 for white.
func luminance(c ui.Color) float64 {
	channel := func(v uint8) float64 {
		x := float64(v) / 255
		if x <= 0.03928 {
			return x / 12.92
		}
		return math.Pow((x+0.055)/1.055, 2.4)
	}
	return 0.2126*channel(c.R) + 0.7152*channel(c.G) + 0.0722*channel(c.B)
}

// accentColor is a bot's accent.
func (p *palette) accentColor(a model.Accent) ui.Color {
	switch a {
	case "blue":
		return p.Blue
	case "teal":
		return p.Teal
	case "green":
		return p.Green
	case "orange":
		return p.Orange
	case "pink":
		return p.Pink
	case "purple":
		return p.Purple
	case "red":
		return p.Red
	}
	return p.Indigo
}

// tone is the color of a state's words.
func (p *palette) tone(t model.Tone) ui.Color {
	switch t {
	case model.ToneTertiary:
		return p.Label3
	case model.ToneGreen:
		return p.Green
	case model.ToneAccent:
		return p.Accent
	case model.ToneRed:
		return p.Red
	case model.ToneOrange:
		return p.Orange
	}
	return p.Label2
}

// applyTheme gives the window's widgets the app's colors and type, over the theme that follows the
// desktop: its appearance, accent, contrast, and text size.
func applyTheme(c *ui.Context) {
	base := c.Theme()
	p := colors(c)
	t := *base
	t.Background = p.Content
	t.Text = p.Label
	t.TextMuted = p.Label2
	t.Border = p.FieldBorder
	t.Surface = p.ButtonBG
	t.SurfaceHover = p.ButtonBG.Mix(p.Label, 0.04)
	t.SurfacePressed = p.ButtonBG.Mix(p.Label, 0.08)
	t.Accent = p.Accent
	t.AccentHover = p.Accent.Mix(ui.RGB(0, 0, 0), 0.12)
	t.AccentPressed = p.Accent.Mix(ui.RGB(0, 0, 0), 0.2)
	t.AccentText = p.AccentText
	t.Danger = p.Red
	t.Warning = p.Orange
	t.Success = p.Green
	t.Selection = p.Accent.Alpha(0.32)
	t.Focus = p.Accent.Alpha(0.7)
	t.Scrollbar = p.Label3
	t.Radius = 6
	t.FontSize = textBody * scaleOf(c)
	c.SetTheme(&t)
}

// scaleOf is the desktop's text size, which the default theme's font size carries.
func scaleOf(c *ui.Context) float32 {
	if s := c.Preferences().TextScale; s > 0 {
		return s
	}
	return 1
}
