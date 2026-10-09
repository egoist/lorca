package main

import (
	"strings"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// L is the app's words in the user's language: `L("New Chat")`, `L("%@ is working…", name)`.
func L(key string, args ...any) string { return l10n.L(key, args...) }

// Lc is one English word that means two things, looked up with its context.
func Lc(text, context string) string { return l10n.Lc(text, context) }

// Titled cards of rows, after the macOS app's DetailViews: the inspector's captioned, bordered card
// and System Settings' heading and plain fill, with dividers between the rows (edge to edge under
// a caption, between the rows' text margins under a heading), and the rows they hold.

type sectionStyle int

const (
	sectionCaption sectionStyle = iota
	sectionHeading
)

// card is a section's card while its rows are built: each row after the first draws the divider
// above it.
type card struct {
	line  ui.Color
	inset float32
	rows  int
}

// row draws the divider above `e` when a row came before it, and counts it.
func (k *card) row(e ui.Element) ui.Element {
	if k.rows > 0 {
		inset, line := k.inset, k.line
		e.DrawOver(func(p *ui.Painter, r ui.Rect) {
			p.Fill(ui.Rect{X: r.X + inset, Y: r.Y, W: r.W - 2*inset, H: 1}, line, 0)
		})
	}
	k.rows++
	return e
}

// section is a title over a card of rows. `accessory` sits on the title's line. With no rows, the
// title (and its accessory) stands alone.
func section(c *ui.Context, title string, style sectionStyle, accessory func(), rows func(k *card)) ui.Element {
	p := colors(c)
	s := ui.Column(c).MinWidth(0).Label(title)
	s.Children(func() {
		// Without a title the card starts at the top.
		if title != "" || accessory != nil {
			header := ui.Row(c).Gap(8)
			if style == sectionCaption {
				header.MinHeight(14).Padding(0, 0, 0, 4).Margin(0, 0, 6, 0)
			} else {
				header.MinHeight(18).Padding(0, 12).Margin(0, 0, 9, 0)
			}
			header.Children(func() {
				if style == sectionCaption {
					ui.Text(c, strings.ToUpper(title)).Grow(1).FontSize(10).FontWeight(600).TextColor(p.Label3).LetterSpacing(0.2).SingleLine()
				} else {
					ui.Text(c, title).Grow(1).FontSize(13).FontWeight(700).TextColor(p.Label).SingleLine()
				}
				if accessory != nil {
					ui.Row(c).Margin(-6, 0).Children(accessory)
				}
			})
		}
		if rows == nil {
			return
		}
		k := &card{line: p.Separator}
		body := ui.Column(c).Background(p.BotBubble).Clip()
		if style == sectionCaption {
			body.Radius(9).Border(1, p.BotBubbleBorder)
		} else {
			body.Radius(12)
			k.inset = 12
		}
		body.Children(func() { rows(k) })
	})
	return s
}

// rowBox is a row's box: its children in a line, 32 tall at least.
func rowBox(c *ui.Context) ui.Element {
	return ui.Row(c).Gap(10).MinHeight(32).Padding(8, 12).MinWidth(0)
}

func rowKey(c *ui.Context, label string) ui.Element {
	return ui.Text(c, label).FontSize(12).TextColor(colors(c).Label2).SingleLine()
}

// keyValueRow is a key on the left and a value on the right that wraps and can be selected.
func keyValueRow(c *ui.Context, k *card, label, value string, mono bool, tint *ui.Color) ui.Element {
	r := k.row(rowBox(c).AlignItems(ui.Start).Label(label))
	r.Children(func() {
		rowKey(c, label).Padding(1, 0, 0, 0)
		v := ui.Text(c, value).Grow(1).Shrink(1).MinWidth(0).TextAlign(ui.End).FontSize(12).Selectable()
		if mono {
			v.Font(monoFont).FontSize(11)
		}
		if tint != nil {
			v.TextColor(*tint)
		}
	})
	return r
}

type botRowOptions struct {
	Detail           string
	AccessorySymbol  string
	AccessoryTooltip string
	Clickable        bool
	AvatarClickable  bool
	Working          bool
}

type botRowResult struct {
	Clicked   bool
	Accessory bool
	Avatar    bool
}

// botRow is a bot, as the inspector, the Device pane, and pickers list them.
func botRow(c *ui.Context, k *card, bot *model.Bot, o botRowOptions) (ui.Element, botRowResult) {
	p := colors(c)
	var result botRowResult
	r := k.row(rowBox(c.Key(bot.ID)).MinHeight(46).Padding(0, 10, 0, 12).Gap(9).Label(bot.Name))
	if o.Clickable {
		r.Cursor(ui.CursorPointer)
		if r.Hovered() {
			r.Background(p.RowHover)
		}
		result.Clicked = r.Clicked()
	}
	r.Children(func() {
		a := avatar(c, botAvatar(bot), 28, o.Working)
		if o.AvatarClickable {
			a.Cursor(ui.CursorPointer).Tooltip(L("Change look"))
			result.Avatar = a.Clicked()
		}
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			ui.Text(c, bot.Name).FontSize(13).FontWeight(500).SingleLine()
			ui.Text(c, o.Detail).FontSize(textCaption).TextColor(p.Label2).SingleLine()
		})
		if o.AccessorySymbol != "" {
			b := ui.ButtonBase(c).Size(22, 22).Radius(5).Center().TextColor(p.Label3).Label(o.AccessoryTooltip)
			if o.AccessoryTooltip != "" {
				b.Tooltip(o.AccessoryTooltip)
			}
			if b.Hovered() {
				b.TextColor(p.Label2)
			}
			b.Children(func() { symbol(c, o.AccessorySymbol, 14, 1.8) })
			result.Accessory = b.Clicked()
		}
	})
	if result.Accessory || result.Avatar {
		result.Clicked = false
	}
	return r, result
}

// pluginTile is a plugin's real mark on a white tile, which stays white in dark mode so a dark
// brand color still reads, or its symbol when it has no mark.
func pluginTile(c *ui.Context, pluginID, symbolName string, size float32) ui.Element {
	mark := pluginMark(pluginID)
	if mark == nil {
		return symbol(c, symbolName, float32(int(size*0.8+0.5)), 1.8)
	}
	tile := ui.Box(c).Size(size, size).Radius(size*0.25).Padding(size*0.2).Background(colors(c).White).Border(0.5, ui.RGBA(0, 0, 0, 0.14))
	tile.Children(func() { ui.Image(c, mark).Fill() })
	return tile
}

type statusRowOptions struct {
	Symbol string
	// SymbolColor tints the symbol; secondary text by default.
	SymbolColor *ui.Color
	PluginID    string
	Title       string
	Subtitle    string
	// SubtitleLines cuts the subtitle to that many lines; 0 lets it wrap.
	SubtitleLines int
	State         string
	// StateSymbol shows the state as a symbol, whose words are its tooltip.
	StateSymbol string
	StateColor  *ui.Color
	// StateDetail is what a state that needs something says in full, a click on the state away.
	StateDetail string
	ActionTitle string
	Destructive bool
	Clickable   bool
	Tooltip     string
}

type statusRowResult struct {
	Clicked bool
	Action  bool
}

// statusRow is a leading symbol, a title over a subtitle, and a trailing state, as words or a
// symbol, or an action button.
func statusRow(c *ui.Context, k *card, o statusRowOptions) (ui.Element, statusRowResult) {
	p := colors(c)
	var result statusRowResult
	r := k.row(rowBox(c).MinHeight(44).Label(o.Title))
	if o.Tooltip != "" {
		r.Tooltip(o.Tooltip)
	}
	if o.Clickable {
		r.Cursor(ui.CursorPointer)
		result.Clicked = r.Clicked()
	}
	r.Children(func() {
		tint := p.Label2
		if o.SymbolColor != nil {
			tint = *o.SymbolColor
		}
		ui.Row(c).Width(18).Justify(ui.Center).TextColor(tint).Children(func() {
			if o.PluginID != "" {
				pluginTile(c, o.PluginID, o.Symbol, 18)
			} else if o.Symbol != "" {
				symbol(c, o.Symbol, 16, 1.7)
			}
		})
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			ui.Text(c, o.Title).FontSize(12.5).FontWeight(500).MaxLines(1)
			if o.Subtitle != "" {
				subtitle := ui.Text(c, o.Subtitle).FontSize(textCaption).TextColor(p.Label2)
				if o.SubtitleLines > 0 {
					subtitle.MaxLines(o.SubtitleLines)
				}
			}
		})
		switch {
		case o.ActionTitle != "":
			kind := buttonDefault
			if o.Destructive {
				kind = buttonDestructive
			}
			result.Action = pushButton(c, o.ActionTitle, pushOptions{Kind: kind, Small: true}).Clicked()
		case o.State != "":
			tint := p.Label2
			if o.StateColor != nil {
				tint = *o.StateColor
			}
			var state ui.Element
			if o.StateSymbol != "" {
				state = ui.Row(c).TextColor(tint).Label(o.State).Tooltip(o.State)
				state.Children(func() { symbol(c, o.StateSymbol, 14, 2.4) })
			} else {
				state = ui.Text(c, o.State).FontSize(11).FontWeight(500).TextColor(tint).TextAlign(ui.End).MaxWidthPercent(45)
			}
			if o.StateDetail != "" {
				state.Cursor(ui.CursorPointer)
				textPopover(c, state, o.StateDetail)
			}
		}
	})
	if result.Action {
		result.Clicked = false
	}
	return r, result
}

// pluginRow is a plugin and its state as a symbol: a check when it is ready, an exclamation mark
// when it needs something, whose words are its tooltip and a click away.
func pluginRow(c *ui.Context, k *card, plugin model.InstalledPlugin, clickable bool, tooltip string) (ui.Element, statusRowResult) {
	p := colors(c)
	ready := plugin.State == model.PluginReady
	o := statusRowOptions{
		Symbol:    plugin.Symbol(),
		PluginID:  plugin.MarketplaceID(),
		Title:     plugin.Name,
		Subtitle:  plugin.Description,
		Clickable: clickable,
		Tooltip:   tooltip,
	}
	if plugin.AccountName != "" {
		// A service's accounts would each repeat its description; their names tell them apart.
		o.Subtitle = ""
	}
	if ready {
		o.State, o.StateSymbol, o.StateColor = L("Ready"), "checkmark", &p.Green
	} else {
		o.State, o.StateSymbol, o.StateColor, o.StateDetail = plugin.Detail, "exclamationmark.circle.fill", &p.Orange, plugin.Detail
	}
	return statusRow(c, k, o)
}

// popUpRow is a label on the left and a pop-up on the right.
func popUpRow(c *ui.Context, k *card, label string, o popUp) (string, bool) {
	var picked string
	var changed bool
	r := k.row(rowBox(c).MinHeight(34).Padding(4, 8, 4, 12).Label(label))
	r.Children(func() {
		rowKey(c, label)
		ui.Spacer(c)
		o.Style, o.Width = popUpBordered, 150
		picked, changed, _ = popUpButton(c, o)
	})
	return picked, changed
}

type actionRowOptions struct {
	Value   string
	Tint    *ui.Color
	Action  string
	Mono    bool
	Tooltip string
	// Copied is the action having just copied something: a green check and its title.
	Copied bool
	// Second is an action before the main one, when the row has two.
	Second string
	Menu   func(*ui.Menu)
}

type actionRowResult struct {
	Action bool
	Second bool
}

// actionRow is a key on the left, a status value on the right, and a text action after it. A
// monospaced value is something to copy (a sign-in code), so it is selectable.
func actionRow(c *ui.Context, k *card, label string, o actionRowOptions) (ui.Element, actionRowResult) {
	p := colors(c)
	var result actionRowResult
	r := k.row(rowBox(c).Label(label))
	if o.Tooltip != "" {
		r.Tooltip(o.Tooltip)
	}
	r.Children(func() {
		rowKey(c, label)
		v := ui.Text(c, o.Value).Grow(1).Shrink(1).MinWidth(0).TextAlign(ui.End).FontSize(12).SingleLine()
		if o.Mono {
			v.Font(monoFont).FontWeight(600).Selectable()
		}
		if o.Tint != nil {
			v.TextColor(*o.Tint)
		}
		if o.Second != "" {
			result.Second = linkButton(c, o.Second, false).Clicked()
		}
		if o.Action != "" {
			b := ui.ButtonBase(c).Gap(4).FontSize(12).FontWeight(500).TextColor(p.Accent).Label(o.Action).Cursor(ui.CursorPointer)
			if o.Copied {
				b.TextColor(p.Green)
			}
			b.Children(func() {
				if o.Copied {
					symbol(c, "checkmark", 11, 2.6)
				}
				ui.Text(c, o.Action).SingleLine()
			})
			result.Action = b.Clicked()
			if o.Menu != nil {
				b.Menu(o.Menu)
			}
		}
	})
	return r, result
}

// disclosureRow is a key on the left, a short value and a chevron on the right, after the Mac's
// DisclosureRow. It reports a click anywhere on it, which opens what the value sums up. A tint
// colors the value, as orange does something to act on.
func disclosureRow(c *ui.Context, k *card, label, value string, tint *ui.Color) bool {
	p := colors(c)
	valueColor := p.Label2
	if tint != nil {
		valueColor = *tint
	}
	r := k.row(rowBox(c).Height(32).Padding(0, 12).Label(label).Cursor(ui.CursorPointer))
	if r.Hovered() {
		r.Background(p.RowHover)
	}
	r.Children(func() {
		rowKey(c, label)
		ui.Text(c, value).Grow(1).Shrink(1).MinWidth(0).TextAlign(ui.End).FontSize(12).TextColor(valueColor).SingleLine()
		ui.Row(c).Shrink(0).TextColor(p.Label3).Margin(0, 0, 0, -4).Children(func() { symbol(c, "chevron.right", 12, 2.2) })
	})
	return r.Clicked()
}

// summaryActionRow is a key and an action on the first line, with a wrapping two-line preview
// under them. With no preview, the key and the action sit alone on one line. It reports the action.
func summaryActionRow(c *ui.Context, k *card, label, value, action string) bool {
	p := colors(c)
	clicked := false
	r := k.row(ui.Column(c).Gap(2).Padding(6, 12, 8, 12).Label(label))
	if value == "" {
		r.Height(32).Padding(0, 12).Justify(ui.Center)
	}
	r.Children(func() {
		ui.Row(c).Gap(8).Justify(ui.SpaceBetween).Children(func() {
			rowKey(c, label)
			clicked = linkButton(c, action, false).Clicked()
		})
		if value != "" {
			ui.Text(c, value).FontSize(textCaption).TextColor(p.Label2).MaxLines(2)
		}
	})
	return clicked
}

type editableRowState struct {
	draft   string
	editing bool
}

// editableRow is a key on the left and an editable value on the right that looks like a value
// until it is clicked, and commits when editing ends (Return, or the keyboard leaving). While the
// user types, the value from the model waits. It answers the committed text.
func editableRow(c *ui.Context, k *card, label, value, placeholder string, mono, alignRight bool) (string, bool) {
	p := colors(c)
	committed, ok := "", false
	r := k.row(rowBox(c).AlignItems(ui.Center).Label(label))
	r.Children(func() {
		rowKey(c, label).Width(76)
		holder := ui.Row(c).Grow(1).Shrink(1).MinWidth(0)
		st := ui.Local(holder, "edit", func() *editableRowState { return &editableRowState{} })
		holder.Children(func() {
			s := *st
			if !s.editing {
				s.draft = value
			}
			field := ui.TextInputBase(c, &s.draft).Grow(1).Shrink(1).MinWidth(0).FontSize(12).TextColor(p.Label).FocusRing(false).Label(label)
			if placeholder != "" {
				field.Placeholder(placeholder)
			}
			if mono {
				field.Font(monoFont).FontSize(11)
			}
			focused := field.Focused()
			if alignRight && !focused {
				field.TextAlign(ui.End)
			}
			switch {
			case focused && field.Shortcut(0, ui.KeyEscape):
				// The field takes Escape back to its value; what holds the row stays open.
				s.draft = value
			case field.Submitted():
				committed, ok = strings.TrimSpace(s.draft), true
			case s.editing && !focused:
				committed, ok = strings.TrimSpace(s.draft), true
			}
			s.editing = focused
		})
	})
	return committed, ok
}

// switchRow is a row with an icon for its state, a title over a detail line, and a switch: a
// routine that pauses or resumes. It reports clicks outside the switch; change runs after the
// view is built with the switch's new value.
func switchRow(c *ui.Context, k *card, symbolName string, tint ui.Color, title string, detail []ui.Span, on *bool, toggleTooltip, tooltip string, change func(bool)) bool {
	p := colors(c)
	r := k.row(rowBox(c).MinHeight(44).Label(title).Cursor(ui.CursorPointer))
	if tooltip != "" {
		r.Tooltip(tooltip)
	}
	clicked := r.Clicked()
	r.Children(func() {
		ui.Row(c).Width(18).Justify(ui.Center).TextColor(tint).Children(func() { symbol(c, symbolName, 15, 1.8) })
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			ui.Text(c, title).FontSize(12.5).FontWeight(500).SingleLine()
			ui.RichText(c, detail...).FontSize(textCaption).TextColor(p.Label2).SingleLine()
		})
		toggle := toggleSwitch(c, on, true).Tooltip(toggleTooltip).Label(toggleTooltip).
			OnChange(func() { change(*on) })
		if toggle.Changed() {
			clicked = false
		}
	})
	return clicked
}

// noteRow is a sentence inside a card, for an empty state, or in a color for what went wrong.
func noteRow(c *ui.Context, k *card, text string, tint *ui.Color) ui.Element {
	p := colors(c)
	r := k.row(ui.Row(c).Padding(10, 12))
	r.Children(func() {
		t := ui.Text(c, text).FontSize(textCaption).TextColor(p.Label2).LineHeight(1.4)
		if tint != nil {
			t.TextColor(*tint)
		}
	})
	return r
}

// accessoryRow is a label on the left and any control on the right.
func accessoryRow(c *ui.Context, k *card, label, tooltip string, control func()) ui.Element {
	r := k.row(rowBox(c).MinHeight(36).Label(label))
	if tooltip != "" {
		r.Tooltip(tooltip)
	}
	r.Children(func() {
		ui.Text(c, label).Grow(1).Shrink(1).MinWidth(0).FontSize(12.5)
		ui.Row(c).Children(control)
	})
	return r
}
