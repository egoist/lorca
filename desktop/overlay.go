package main

import (
	_ "embed"
	"strings"
	"sync"

	"github.com/egoist/mygo/ui"
)

// Sheets, alerts, and popovers, after AppKit's: a sheet is modal to the window and stacks over the
// one before it; an alert is a sheet with a message and buttons; a popover hangs off the control
// that opened it and closes when the user clicks elsewhere.

//go:embed assets/icon-192.png
var appIconPNG []byte

//go:embed assets/icon-dev-192.png
var appIconDevPNG []byte

var appIconOnce = sync.OnceValue(func() *ui.Bitmap {
	data := appIconPNG
	if isDevelopment() {
		data = appIconDevPNG
	}
	bitmap, _ := ui.DecodeBitmap(data)
	return bitmap
})

// appIcon is the app's icon, Lorca Dev's in a development build.
func appIcon() *ui.Bitmap { return appIconOnce() }

// MARK: - Sheets

// sheet is one sheet up over a window. Its view builds it every frame until it is dismissed.
type sheet struct {
	id        int
	view      func(c *ui.Context, s *sheet)
	onDismiss func()
	window    *appWindow
}

// dismiss takes the sheet away; the keyboard goes back to what had it when it came up.
func (s *sheet) dismiss() {
	if s.window == nil {
		return
	}
	w := s.window
	s.window = nil
	for i, each := range w.sheets {
		if each == s {
			w.sheets = append(w.sheets[:i:i], w.sheets[i+1:]...)
			break
		}
	}
	if s.onDismiss != nil {
		s.onDismiss()
	}
	w.invalidate()
}

// present puts a sheet up over the window, in front of any other.
func (w *appWindow) present(view func(c *ui.Context, s *sheet), onDismiss func()) *sheet {
	w.nextSheet++
	s := &sheet{id: w.nextSheet, view: view, onDismiss: onDismiss, window: w}
	w.sheets = append(w.sheets, s)
	w.invalidate()
	return s
}

// hasSheet is whether a sheet is up, so the window's commands leave the keyboard to it.
func (w *appWindow) hasSheet() bool { return len(w.sheets) > 0 }

// sheetsView builds the window's sheets over its content: the one in front over a dimmed window,
// those behind it as they were.
func (w *appWindow) sheetsView(c *ui.Context) {
	p := colors(c)
	for i, s := range w.sheets {
		front := i == len(w.sheets)-1
		ui.Overlay(c, func() {
			scrim := ui.Column(c.Key(s.id)).Absolute().Left(0).Top(0).Right(0).Bottom(0).AlignItems(ui.Center).Padding(44, 16, 16, 16)
			if front {
				scrim.Background(p.Scrim).Modal()
			}
			scrim.Transition(ui.ElementTransition{Enter: &ui.Motion{}, Duration: 120_000_000})
			scrim.Children(func() { s.view(c, s) })
		})
	}
}

// fieldSubmitted is set when a field in the sheet being built took Return, which confirms the
// sheet as Return elsewhere in it does.
var fieldSubmitted bool

// sheetOptions describe the shared sheet chrome: a title, an optional subtitle, the content, and a
// trailing row of buttons. Return confirms and Escape cancels, as a sheet's default and cancel
// buttons do.
type sheetOptions struct {
	Title    string
	Subtitle string
	// Width is 420 by default.
	Width   float32
	Confirm string
	// Cancel is the cancel button's title, Cancel by default.
	Cancel string
	// NoCancel leaves only the confirm button, which then answers Escape as well.
	NoCancel        bool
	ConfirmKind     buttonKind
	ConfirmDisabled bool
	// Leading builds controls at the start of the buttons' row.
	Leading func()
	// Footer builds a row of actions under the content, outside its scroll, so a tall sheet in a
	// short window keeps them in view.
	Footer func()
	// ReturnInContent leaves Return to the content, as a multi-line editor's.
	ReturnInContent bool
}

type sheetResult struct {
	Confirmed bool
	Cancelled bool
}

// sheetPanel is a sheet's panel: as round as macOS 26's sheets, white in the light appearance. It
// is no taller than the window; what it holds between its title and its buttons goes in a
// sheetBody, which scrolls when the window is too short for it.
func sheetPanel(c *ui.Context, width float32) ui.Element {
	p := colors(c)
	if width == 0 {
		width = 420
	}
	return ui.Column(c).Width(width).MaxWidthPercent(100).MaxHeightPercent(100).Padding(20).Radius(22).
		Background(p.Sheet).Shadow(0, 10, 40, 0, p.Shadow).Border(0.5, p.ShadowEdge).
		Transition(ui.ElementTransition{Enter: &ui.Motion{Y: -10}, Duration: 160_000_000})
}

// sheetFrame builds the chrome around `content` and answers whether the sheet was confirmed or
// cancelled this frame.
func sheetFrame(c *ui.Context, o sheetOptions, content func()) sheetResult {
	p := colors(c)
	var result sheetResult
	panel := sheetPanel(c, o.Width)
	// The sheet takes the keyboard itself as it comes up, unless a field in it does, so Return
	// confirms it rather than pressing the first button.
	panel.Focusable().FocusRing(false).AutoFocus()
	panel.Children(func() {
		ui.Column(c).Gap(4).Children(func() {
			ui.Text(c, o.Title).FontSize(15).FontWeight(600)
			if o.Subtitle != "" {
				ui.Text(c, o.Subtitle).FontSize(12).TextColor(p.Label2).LineHeight(1.4)
			}
		})
		if content != nil {
			fieldSubmitted = false
			sheetBody(c).Margin(13, -20, 0, -20).Children(func() {
				ui.Column(c).Gap(12).Children(content)
			})
			if fieldSubmitted && o.Confirm != "" && !o.ConfirmDisabled && !o.ReturnInContent {
				result.Confirmed = true
			}
			fieldSubmitted = false
		}
		if o.Footer != nil {
			ui.Row(c).Margin(12, 0, 0, 0).Children(o.Footer)
		}
		if o.Confirm != "" || o.Leading != nil {
			ui.Row(c).Gap(10).Margin(20, 0, 0, 0).Children(func() {
				if o.Leading != nil {
					ui.Row(c).Gap(8).Children(o.Leading)
				}
				ui.Spacer(c)
				if !o.NoCancel && o.Confirm != "" {
					if pushButton(c, firstNonEmpty(o.Cancel, L("Cancel")), pushOptions{}).Clicked() {
						result.Cancelled = true
					}
				}
				if o.Confirm != "" {
					kind := o.ConfirmKind
					if kind == buttonDefault {
						kind = buttonPrimary
					}
					if pushButton(c, o.Confirm, pushOptions{Kind: kind, Disabled: o.ConfirmDisabled}).Clicked() {
						result.Confirmed = true
					}
				}
			})
		}
	})
	if panel.OverlayShortcut(0, ui.KeyEscape) {
		if o.NoCancel {
			result.Confirmed = true
		} else {
			result.Cancelled = true
		}
	}
	if !o.ReturnInContent && o.Confirm != "" && !o.ConfirmDisabled && panel.OverlayShortcut(0, ui.KeyEnter) {
		result.Confirmed = true
	}
	return result
}

// formRow is a label in the labels' column and a control filling the rest of the line, as New
// Bot's form lays them out. A control taller than a line keeps its label by its first line.
func formRow(c *ui.Context, label string, top bool, control func()) {
	row := ui.Row(c).Gap(10)
	if top {
		row.AlignItems(ui.Start)
	}
	row.Children(func() {
		text := ui.Text(c, label).Width(formLabelWidth).FontSize(12).TextColor(colors(c).Label2).SingleLine()
		if top {
			text.Padding(6, 0, 0, 0)
		}
		ui.Row(c).Grow(1).MinWidth(0).Children(control)
	})
}

// formLabelWidth is the labels' column; the controls take the rest of the sheet's width.
const formLabelWidth = 76

// sheetBody is the part of a sheet's panel that scrolls when the window is too short for the
// panel, reaching the panel's edges so its fields' focus rings show whole.
func sheetBody(c *ui.Context) ui.Element {
	return ui.Scroll(c).Shrink(1).MinHeight(0).Padding(3, 20)
}

// submits marks a field whose Return confirms the sheet it is in.
func submits(e ui.Element) ui.Element {
	if e.Submitted() {
		fieldSubmitted = true
	}
	return e
}

// MARK: - Alerts

type alertStyle int

const (
	alertInformational alertStyle = iota
	alertWarning
	alertCritical
)

type alertButton struct {
	Title       string
	Destructive bool
}

// alertOptions describe an alert: the first button is the default (Return); one titled Cancel
// answers Escape, else the last, unless Escape names another.
type alertOptions struct {
	Message     string
	Informative string
	Buttons     []alertButton
	// Escape is the button Escape presses, when it is neither Cancel nor the last; -1 is unset.
	Escape int
	Style  alertStyle
	// Accessory is a control under the text, as an alert's accessory view.
	Accessory func(c *ui.Context)
	Width     float32
}

// showAlert puts an alert up as a sheet, and answers with the index of the button chosen.
func (w *appWindow) showAlert(o alertOptions, done func(index int)) {
	buttons := o.Buttons
	if len(buttons) == 0 {
		buttons = []alertButton{{Title: L("OK")}}
	}
	cancel := o.Escape
	if cancel <= 0 {
		cancel = -1
		for i, button := range buttons {
			if button.Title == L("Cancel") {
				cancel = i
			}
		}
		if o.Escape > 0 {
			cancel = o.Escape
		}
	}
	if cancel < 0 {
		cancel = len(buttons) - 1
	}
	answered := false
	answer := func(s *sheet, index int) {
		if answered {
			return
		}
		answered = true
		s.dismiss()
		if done != nil {
			done(index)
		}
	}
	w.present(func(c *ui.Context, s *sheet) {
		width := o.Width
		if width == 0 {
			width = 380
		}
		panel := sheetPanel(c, width)
		panel.Children(func() {
			sheetBody(c).Margin(-3, -20, 0, -20).Children(func() { alertBody(c, o, s, answer) })
			ui.Row(c).Gap(10).Margin(20, 0, 0, 0).Children(func() {
				ui.Spacer(c)
				for i := len(buttons) - 1; i >= 0; i-- {
					button := buttons[i]
					kind := buttonDefault
					switch {
					case button.Destructive:
						kind = buttonDestructive
					case i == 0:
						kind = buttonPrimary
					}
					b := pushButton(c, button.Title, pushOptions{Kind: kind})
					if i == 0 && o.Accessory == nil {
						b.AutoFocus()
					}
					if b.Clicked() {
						answer(s, i)
					}
				}
			})
		})
		if panel.OverlayShortcut(0, ui.KeyEscape) {
			answer(s, cancel)
		}
		if panel.OverlayShortcut(0, ui.KeyEnter) {
			answer(s, 0)
		}
	}, func() {
		if !answered {
			answered = true
			if done != nil {
				done(cancel)
			}
		}
	})
}

// alertBody is an alert's icon, words, and accessory.
func alertBody(c *ui.Context, o alertOptions, s *sheet, answer func(*sheet, int)) {
	p := colors(c)
	icon := ui.Box(c).Size(48, 48).Margin(0, 0, 12, 0)
	icon.Children(func() {
		if bitmap := appIcon(); bitmap != nil {
			ui.Image(c, bitmap).Size(48, 48)
		}
		if o.Style == alertCritical {
			ui.Row(c).Absolute().Right(-6).Bottom(-4).TextColor(p.Orange).Children(func() {
				symbol(c, "exclamationmark.triangle.fill", 20, 2.2)
			})
		}
	})
	ui.Column(c).Gap(4).Children(func() {
		ui.Text(c, o.Message).FontSize(15).FontWeight(600)
		if o.Informative != "" {
			ui.Text(c, o.Informative).FontSize(12).TextColor(p.Label2).LineHeight(1.4)
		}
	})
	if o.Accessory != nil {
		fieldSubmitted = false
		ui.Column(c).Margin(12, 0, 0, 0).Children(func() { o.Accessory(c) })
		if fieldSubmitted {
			answer(s, 0)
		}
		fieldSubmitted = false
	}
}

// showNote is an alert of one paragraph and OK.
func (w *appWindow) showNote(title, body string) {
	w.showAlert(alertOptions{Message: title, Informative: body, Width: 420, Escape: -1}, nil)
}

// MARK: - Popovers

// popoverPanel styles a popover's panel: a rounded card on the popover color, below its anchor.
func popoverPanel(c *ui.Context, panel ui.Element) ui.Element {
	p := colors(c)
	return panel.Margin(8, 0, 0, 0).Radius(10).Background(p.Popover).Shadow(0, 10, 40, 0, p.Shadow).Border(0.5, p.ShadowEdge)
}

// textPopover makes a click on `anchor` show the full text behind a one-line marker, rendered like
// a message body, scrolling past a screenful, sized to its content up to a bubble's width.
func textPopover(c *ui.Context, anchor ui.Element, text string) {
	if strings.TrimSpace(text) == "" {
		return
	}
	open := ui.Local(anchor, "popover", func() bool { return false })
	if anchor.Clicked() {
		*open = !*open
	}
	ui.PopoverBase(c, anchor, open, func(panel ui.Element) {
		popoverPanel(c, panel)
		ui.Scroll(c).MaxWidth(468).MinWidth(188).MaxHeight(388).Padding(14).Children(func() {
			markdownView(c, text, markdownOptions{})
		})
	})
}

// firstLineOf is the first non-empty line of `text`, with fence markers skipped and bold and code
// marks dropped, as the sidebar's preview drops them: what a one-line preview shows.
func firstLineOf(text string) string {
	for _, line := range strings.Split(strings.ReplaceAll(text, "\r\n", "\n"), "\n") {
		line = strings.TrimSpace(line)
		if line != "" && !strings.HasPrefix(line, "```") {
			return strings.NewReplacer("**", "", "`", "").Replace(line)
		}
	}
	return ""
}
