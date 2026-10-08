package main

import (
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// The app's controls, after the macOS app's Controls.swift: borderless symbol buttons that fill
// under the pointer, push buttons, switches, pop-up buttons whose menu is the system's, fields,
// the search field, and a button that says it copied.

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}

// hoverButtonOptions describe a borderless symbol button.
type hoverButtonOptions struct {
	Symbol string
	// Size is the symbol's, 16 by default.
	Size float32
	// Title makes it the symbol and the word, as wide as they need; Trailing is a symbol after a
	// word that leads somewhere.
	Title    string
	Trailing string
	Tooltip  string
	// Label names it for assistive technology, else its tooltip or title.
	Label    string
	Disabled bool
	// Active keeps its fill, as while what it opened is up.
	Active bool
}

// hoverButton is a borderless symbol button that fills a rounded rect under the pointer.
func hoverButton(c *ui.Context, o hoverButtonOptions) ui.Element {
	p := colors(c)
	b := ui.ButtonBase(c).Height(28).MinWidth(28).Radius(6).Gap(5).Justify(ui.Center).TextColor(p.Label2)
	if o.Title != "" {
		b.Padding(0, 8).FontSize(13)
	}
	if label := firstNonEmpty(o.Label, o.Tooltip, o.Title); label != "" {
		b.Label(label)
	}
	if o.Tooltip != "" {
		b.Tooltip(o.Tooltip)
	}
	switch {
	case o.Disabled:
		b.Disabled(true)
	case b.Pressed():
		b.Background(p.Pressed)
	case b.Hovered() || o.Active:
		b.Background(p.Hover)
	}
	size := o.Size
	if size == 0 {
		size = 16
	}
	b.Children(func() {
		if o.Symbol != "" {
			symbol(c, o.Symbol, size, 1.75)
		}
		if o.Title != "" {
			ui.Text(c, o.Title).SingleLine()
		}
		if o.Trailing != "" {
			symbol(c, o.Trailing, 10, 2.5)
		}
	})
	return b
}

type buttonKind int

const (
	buttonDefault buttonKind = iota
	buttonPrimary
	buttonDestructive
)

// pushOptions describe a push button.
type pushOptions struct {
	Kind     buttonKind
	Small    bool
	Large    bool
	Disabled bool
	Tooltip  string
	// Symbol leads the title.
	Symbol string
}

// pushButton is a push button as macOS 26 draws one: a flat rounded fill without a border or
// shadow, 24 tall, in the accent color for the default button.
func pushButton(c *ui.Context, title string, o pushOptions) ui.Element {
	p := colors(c)
	b := ui.ButtonBase(c).Height(24).Padding(0, 12).Radius(6).Gap(5).Justify(ui.Center).FontSize(13).TextColor(p.Label).Label(title)
	switch {
	case o.Small:
		b.Height(22).Padding(0, 10).FontSize(11.5).Radius(5)
	case o.Large:
		b.Height(32).Padding(0, 18)
	}
	fill := p.ButtonBG
	// A disabled default button is a plain one, as on macOS; MyGo dims what is disabled.
	if o.Kind == buttonPrimary && !o.Disabled {
		fill = p.Accent
		b.TextColor(p.AccentText)
	} else if o.Kind == buttonDestructive {
		b.TextColor(p.Red)
	}
	switch {
	case o.Disabled:
		b.Disabled(true)
	case b.Pressed():
		if o.Kind == buttonPrimary {
			fill = fill.Mix(ui.RGB(0, 0, 0), 0.2)
		} else {
			fill = fill.Mix(p.Label, 0.08)
		}
	case b.Hovered():
		if o.Kind == buttonPrimary {
			fill = fill.Mix(ui.RGB(0, 0, 0), 0.12)
		} else {
			fill = fill.Mix(p.Label, 0.04)
		}
	}
	b.Background(fill)
	if o.Tooltip != "" {
		b.Tooltip(o.Tooltip)
	}
	b.Children(func() {
		if o.Symbol != "" {
			symbol(c, o.Symbol, 13, 2)
		}
		ui.Text(c, title).SingleLine()
	})
	return b
}

// linkButton is a text button in the accent color, as the rows' Edit… and Change are.
func linkButton(c *ui.Context, title string, disabled bool) ui.Element {
	p := colors(c)
	b := ui.ButtonBase(c).Gap(4).FontSize(12).FontWeight(500).TextColor(p.Accent).Label(title).Cursor(ui.CursorPointer)
	if disabled {
		b.Disabled(true)
	}
	b.Children(func() { ui.Text(c, title).SingleLine() })
	return b
}

// toggleSwitch is a switch: a track in the accent color while on, and a white knob that slides.
func toggleSwitch(c *ui.Context, on *bool, small bool) ui.Element {
	p := colors(c)
	w, h, knob := float32(32), float32(19), float32(15)
	if small {
		w, h, knob = 26, 15, 11
	}
	s := ui.SwitchBase(c, on).Size(w, h).Radius(h / 2)
	track := p.Label4
	if *on {
		track = p.Accent
	}
	s.Background(track).Transition(ui.ElementTransition{Colors: true, Duration: 150 * time.Millisecond})
	left := float32(2)
	if *on {
		left = w - knob - 2
	}
	x := s.Animate("knob", left, 150*time.Millisecond)
	s.Children(func() {
		ui.Box(c).Absolute().Top((h-knob)/2).Left(x).Size(knob, knob).Radius(knob/2).Background(p.White).Shadow(0, 1, 2, 0, ui.RGBA(0, 0, 0, 0.3))
	})
	return s
}

type popUpStyle int

const (
	// popUpBordered is a form control's: a push button's flat fill with the chevrons at its end.
	popUpBordered popUpStyle = iota
	// popUpSettings is System Settings' look: the text, then chevrons on a small platter.
	popUpSettings
	// popUpPlain is text with chevrons, for a toolbar.
	popUpPlain
)

type popUpOption struct {
	Value string
	Label string
	// Symbol shows before the picked option's label, as a Device's.
	Symbol string
	// Separated puts a separator before the option.
	Separated bool
}

// popUpExtra is an item after the choices, such as Pair a Device….
type popUpExtra struct {
	ID    string
	Label string
}

type popUp struct {
	Options  []popUpOption
	Value    string
	Style    popUpStyle
	Disabled bool
	Tooltip  string
	Label    string
	Extras   []popUpExtra
	// Width fixes the button's width; 0 fits its choice.
	Width float32
}

// popUpButton shows the choice as text; its menu is the system's, with a check on the choice. It
// answers the option picked this frame when it differs from the choice, and the extra picked.
func popUpButton(c *ui.Context, o popUp) (picked string, changed bool, extra string) {
	p := colors(c)
	current := popUpOption{}
	for _, option := range o.Options {
		if option.Value == o.Value {
			current = option
		}
	}
	b := ui.ButtonBase(c).Gap(8).TextColor(p.Label)
	if o.Label != "" {
		b.Label(o.Label)
	}
	if o.Tooltip != "" {
		b.Tooltip(o.Tooltip)
	}
	if o.Width > 0 {
		b.Width(o.Width)
	}
	hovered, pressed := b.Hovered() && !o.Disabled, b.Pressed() && !o.Disabled
	chevrons := float32(12)
	switch o.Style {
	case popUpSettings:
		b.Height(24).Padding(0, 4, 0, 9).Radius(8).FontSize(13)
		if hovered {
			b.Background(p.Hover)
		}
		chevrons = 11
	case popUpBordered:
		b.Height(24).Padding(0, 6, 0, 9).Radius(6).FontSize(12).Justify(ui.SpaceBetween)
		fill := p.ButtonBG
		if pressed {
			fill = fill.Mix(p.Label, 0.08)
		} else if hovered {
			fill = fill.Mix(p.Label, 0.04)
		}
		b.Background(fill)
	case popUpPlain:
		b.Height(28).Padding(0, 8, 0, 10).Radius(14).FontSize(13)
		if hovered {
			b.Background(p.Hover)
		}
	}
	if o.Disabled {
		b.Disabled(true)
	}
	b.Children(func() {
		if current.Symbol != "" {
			symbol(c, current.Symbol, 13, 1.9).TextColor(p.Label2).Margin(0, -3, 0, 0)
		}
		title := ui.Text(c, current.Label).SingleLine()
		if o.Width > 0 {
			title.Grow(1).Shrink(1)
		}
		well := ui.Row(c).Center()
		switch o.Style {
		case popUpSettings:
			well.Size(19, 19).Radius(8)
			if pressed {
				well.Background(p.Pressed)
			} else if !hovered {
				well.Background(p.Hover)
			}
		case popUpBordered:
			well.TextColor(p.Label2)
		}
		well.Children(func() { symbol(c, "chevron.up.chevron.down", chevrons, 2.4) })
	})
	b.Menu(func(m *ui.Menu) {
		for i, option := range o.Options {
			if option.Separated && i > 0 {
				m.Separator()
			}
			if m.Item(option.Label).Checked(option.Value == o.Value).Chosen() && option.Value != o.Value {
				picked, changed = option.Value, true
			}
		}
		if len(o.Extras) > 0 {
			m.Separator()
			for _, item := range o.Extras {
				if m.Item(item.Label).Chosen() {
					extra = item.ID
				}
			}
		}
	})
	return picked, changed, extra
}

// fieldOptions describe a text field.
type fieldOptions struct {
	Placeholder string
	Secure      bool
	Mono        bool
	Disabled    bool
	ReadOnly    bool
	AutoFocus   bool
	// Plain has no border or fill, as a field inside a row.
	Plain bool
	Label string
}

// fieldChrome draws a field's border and fill, and its focus as a single ring two wide: the border
// in the accent color and a DIP more outside it.
func fieldChrome(c *ui.Context, e ui.Element, focused, plain bool) {
	p := colors(c)
	e.FocusRing(false)
	if plain {
		return
	}
	e.Radius(6).Background(p.Field)
	if focused {
		e.Border(1, p.Accent).Shadow(0, 0, 0, 1, p.Accent)
	} else {
		e.Border(1, p.FieldBorder)
	}
}

// textField is a one-line field; its Submitted is Return.
func textField(c *ui.Context, value *string, o fieldOptions) ui.Element {
	p := colors(c)
	e := ui.TextInputBase(c, value).MinHeight(26).Padding(4, 8).FontSize(13).TextColor(p.Label)
	if o.Mono {
		e.Font(monoFont).FontSize(12)
	}
	if o.Secure {
		e.Password()
	}
	if o.Placeholder != "" {
		e.Placeholder(o.Placeholder)
	}
	if o.Label != "" {
		e.Label(o.Label)
	}
	if o.AutoFocus {
		e.AutoFocus()
	}
	if o.ReadOnly {
		e.ReadOnly(true)
	}
	if o.Disabled {
		e.Disabled(true)
	}
	fieldChrome(c, e, e.Focused() && !o.Disabled, o.Plain)
	if e.Submitted() {
		fieldSubmitted = true
	}
	return e
}

// textArea is a field of several lines, `lines` high, which scrolls past them.
func textArea(c *ui.Context, value *string, lines int, o fieldOptions) ui.Element {
	p := colors(c)
	e := ui.TextAreaBase(c, value).Padding(6, 8).FontSize(13).LineHeight(1.4).TextColor(p.Label)
	size := float32(13)
	if o.Mono {
		e.Font(monoFont).FontSize(12)
		size = 12
	}
	if lines > 0 {
		e.Height(float32(lines)*size*1.4 + 12)
	}
	if o.Placeholder != "" {
		e.Placeholder(o.Placeholder)
	}
	if o.Label != "" {
		e.Label(o.Label)
	}
	if o.AutoFocus {
		e.AutoFocus()
	}
	if o.ReadOnly {
		e.ReadOnly(true)
	}
	if o.Disabled {
		e.Disabled(true)
	}
	fieldChrome(c, e, e.Focused() && !o.Disabled, o.Plain)
	return e
}

// searchField is a capsule search field with a magnifier and a clear button; Escape clears it
// while it has text. Its Changed and Submitted are the input's.
func searchField(c *ui.Context, value *string, placeholder string) ui.Element {
	p := colors(c)
	if placeholder == "" {
		placeholder = L("Search")
	}
	var input ui.Element
	box := ui.Row(c).Height(30).Gap(6).Padding(0, 8, 0, 10).Radius(15).Background(p.SearchBG).TextColor(p.Label2).Cursor(ui.CursorText)
	box.Children(func() {
		symbol(c, "magnifyingglass", 13, 2)
		input = ui.TextInputBase(c, value).Grow(1).Shrink(1).MinWidth(0).FontSize(13).TextColor(p.Label).Placeholder(placeholder).Label(placeholder).FocusRing(false)
		if *value != "" {
			if input.Shortcut(0, ui.KeyEscape) {
				*value = ""
			}
			clear := ui.ButtonBase(c).Size(16, 16).Radius(8).Center().Background(p.Label3).TextColor(p.Content).Label(L("Clear")).FocusRing(false)
			clear.Children(func() { symbol(c, "xmark", 9, 3) })
			if clear.Clicked() {
				*value = ""
			}
		}
	})
	if input.Focused() {
		box.Shadow(0, 0, 0, 1, p.Accent).Border(1, p.Accent)
	}
	if box.Clicked() {
		input.Focus()
	}
	return input
}

// spinner is the system's spinner, `size` across.
func spinner(c *ui.Context, size float32) ui.Element {
	return ui.Spinner(c).Size(size, size)
}

type copyOptions struct {
	Title    string
	Symbol   string
	Tooltip  string
	Bordered bool
}

// copyButton copies `text` and says so for a moment: its symbol turns into a green check, its
// title into Copied. It reports the click.
func copyButton(c *ui.Context, text string, o copyOptions) ui.Element {
	p := colors(c)
	b := ui.ButtonBase(c).Height(24).MinWidth(24).Gap(5).Justify(ui.Center).Radius(5)
	copiedAt := ui.Local(b, "copied", func() time.Time { return time.Time{} })
	if b.Clicked() {
		mygo.Clipboard.WriteText(text)
		*copiedAt = c.Now()
	}
	copied := !copiedAt.IsZero() && c.Now().Sub(*copiedAt) < 1500*time.Millisecond
	if copied {
		c.After(1500*time.Millisecond - c.Now().Sub(*copiedAt))
	}
	if o.Bordered {
		b.Padding(0, 12).FontSize(13).TextColor(p.Label)
		fill := p.ButtonBG
		if b.Hovered() {
			fill = fill.Mix(p.Label, 0.04)
		}
		b.Background(fill)
	} else {
		b.Padding(0, 4).TextColor(p.Label2)
		if b.Hovered() {
			b.Background(p.Hover)
		}
	}
	if copied {
		b.TextColor(p.Green)
	}
	b.Label(firstNonEmpty(o.Tooltip, o.Title, L("Copy")))
	if o.Tooltip != "" && !copied {
		b.Tooltip(o.Tooltip)
	}
	b.Children(func() {
		if copied {
			symbol(c, "checkmark", 13, 2)
		} else if o.Symbol != "" {
			symbol(c, o.Symbol, 13, 2)
		}
		if o.Title != "" {
			title := o.Title
			if copied {
				title = L("Copied")
			}
			ui.Text(c, title).SingleLine()
		}
	})
	return b
}

// segmented is a segmented control: one choice of a few, each a segment as wide as its label.
func segmented(c *ui.Context, selected *int, label string, labels ...string) ui.Element {
	p := colors(c)
	parts := ui.SegmentedBase(c, selected, len(labels))
	parts.Track.Padding(2).Radius(7).Background(p.Chip).Label(label)
	parts.Track.Children(func() {
		for i, text := range labels {
			// Each segment is as wide as its words.
			seg := parts.Segment(i).Height(22).Padding(0, 10).Radius(5).FontSize(12).TextColor(p.Label).Justify(ui.Center).Shrink(0)
			if i == *selected {
				seg.Background(p.Popover).Shadow(0, 1, 2, 0, ui.RGBA(0, 0, 0, 0.12))
			}
			seg.Children(func() { ui.Text(c, text).SingleLine() })
		}
	})
	return parts.Track
}

// caption is a small line of secondary text.
func caption(c *ui.Context, text string) ui.Element {
	return ui.Text(c, text).FontSize(textCaption).TextColor(colors(c).Label2)
}
