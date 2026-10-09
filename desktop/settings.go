package main

import (
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Settings panes in the main window's content area, after the macOS app's settings view
// controllers: section cards and footnotes in one scrolling column, the window's header carrying
// the page title. General and Advanced also fill the small Settings window onboarding opens.

// settingsPageState is the settings page's own state, kept by the main window: a search pick
// being revealed, the picked Runner's mcp.json, and how its service stands.
type settingsPageState struct {
	// target is the row or section a search picked, until the pane builds it; picked is when, as
	// rows that load later (an mcp.json's servers) get a second to come.
	target string
	picked time.Time
	// found is the frame that built the target, which scrolls it into view.
	found time.Time
	// flash is the label flashing since flashAt: a section's card, or a row.
	flash        string
	flashSection bool
	flashAt      time.Time
	// claimed is an element of this build having taken the reveal, so one element has it.
	claimed bool

	mcp     settingsMcpState
	service settingsServiceState
}

const (
	// A picked setting's highlight holds, then fades: 1.2 seconds in all.
	settingsFlashHold = 480 * time.Millisecond
	settingsFlashFade = 720 * time.Millisecond
	// settingsFlashAlpha is the accent's share of the highlight.
	settingsFlashAlpha = 0.22
)

// begin takes up a search pick for this pane, and lets go of one that was revealed or never came.
func (s *settingsPageState) begin(c *ui.Context, revealed *revealedSetting, pane model.SettingsPane) {
	now := c.Now()
	s.claimed = false
	if revealed != nil && !revealed.shown && revealed.entry.pane == pane {
		revealed.shown = true
		s.target, s.picked, s.found = revealed.entry.row, now, time.Time{}
		s.flash = ""
	}
	switch {
	case s.target == "":
	case !s.found.IsZero() && !now.Equal(s.found):
		s.target = ""
	case s.found.IsZero() && now.Sub(s.picked) > time.Second:
		s.target = ""
	}
}

// claim answers whether the element labelled `label` scrolls into view in this frame, and how
// strongly it flashes. A section claims ahead of its rows, so a section of that name flashes its
// card ahead of a row with the same label, as on the Mac.
func (s *settingsPageState) claim(c *ui.Context, label string, section bool) (reveal bool, flash float32) {
	if s.claimed || label == "" {
		return false, 0
	}
	now := c.Now()
	if label == s.target {
		if s.found.IsZero() {
			s.found = now
			s.flash, s.flashSection, s.flashAt = label, section, now
		}
		reveal = now.Equal(s.found)
	}
	if label != s.flash || section != s.flashSection {
		return reveal, 0
	}
	elapsed := now.Sub(s.flashAt)
	if elapsed >= settingsFlashHold+settingsFlashFade {
		s.flash = ""
		return reveal, 0
	}
	s.claimed = true
	c.AnimationFrame()
	flash = settingsFlashAlpha
	if elapsed > settingsFlashHold {
		flash *= 1 - ui.EaseOut(float32(elapsed-settingsFlashHold)/float32(settingsFlashFade))
	}
	return reveal, flash
}

// settingsPane is a pane while it is built: the window its sheets and alerts go over, and the
// page that carries out a search's reveal, which the small Settings window does without.
type settingsPane struct {
	w    *appWindow
	page *settingsPageState
}

// settingsPage is the content of a settings pane in the main window.
func (m *mainWindow) settingsPage(c *ui.Context, pane model.SettingsPane) {
	m.settings.begin(c, m.revealed, pane)
	s := settingsPane{w: &m.appWindow, page: &m.settings}
	if pane != model.PanePlugins {
		// The Plugins pane asks for the Runner's mcp.json again whenever it comes back.
		m.settings.mcp.key = ""
	}
	if pane != model.PaneDevice {
		// So does the Devices pane for the Runner's service.
		m.settings.service.asked = ""
	}
	switch pane {
	case model.PaneGeneral:
		s.general(c)
	case model.PaneProviders:
		s.providers(c)
	case model.PaneAutoReview:
		s.autoReview(c)
	case model.PanePlugins:
		s.plugins(c, m)
	case model.PaneBots:
		s.bots(c, m)
	case model.PaneDevice:
		s.device(c, m)
	case model.PaneAdvanced:
		s.advanced(c)
	}
}

// frame is a page: its cards and footnotes in a column that scrolls. Another pane, or another
// Device's page, starts at its top.
func (s settingsPane) frame(c *ui.Context, key string, content func()) {
	ui.Scroll(c.Key(key)).Grow(1).MinHeight(0).Background(colors(c).Content).Children(func() {
		ui.Column(c).Gap(22).Padding(24, 28, 32, 28).Children(content)
	})
}

// section is a titled card of rows, the target of a search pick by its title.
func (s settingsPane) section(c *ui.Context, title string, accessory func(), rows func(k *card)) {
	section(c, title, sectionHeading, accessory, func(k *card) {
		if s.page == nil {
			rows(k)
			return
		}
		reveal, flash := s.page.claim(c, title, true)
		// Under the rows, over the card's fill, as the row's own flash is.
		ui.Box(c).Absolute().Left(0).Top(0).Right(0).Bottom(0).PassThrough().Background(colors(c).Accent.Alpha(flash))
		rows(k)
		if reveal {
			settingsRevealBox(c)
		}
	})
}

// mark makes a row the target of a search pick by its label: it scrolls into view and flashes.
func (s settingsPane) mark(c *ui.Context, e ui.Element, label string) ui.Element {
	if s.page == nil {
		return e
	}
	reveal, flash := s.page.claim(c, label, false)
	if flash > 0 {
		fill := colors(c).Accent.Alpha(flash)
		e.Draw(func(p *ui.Painter, r ui.Rect) { p.Fill(r, fill, 0) })
	}
	if reveal {
		e.Children(func() { settingsRevealBox(c) })
	}
	return e
}

// markBox marks a row whose builder answers no element by a box around it.
func (s settingsPane) markBox(c *ui.Context, label string, build func()) {
	s.mark(c, ui.Column(c).Children(build), label)
}

// settingsRevealBox scrolls what holds it into view with room above it for its section's
// heading, and for the header the page runs under.
func settingsRevealBox(c *ui.Context) {
	ui.Box(c).Absolute().Top(-40).Bottom(-40).Left(0).Width(1).PassThrough().ScrollIntoView()
}

// footnote is a paragraph under a card, in the tertiary color.
func (s settingsPane) footnote(c *ui.Context, text string) {
	ui.Text(c, text).FontSize(textCaption).LineHeight(1.4).TextColor(colors(c).Label3)
}

// MARK: - The small window

// settingsWindow is Settings while onboarding is up: General and Advanced, the panes that work
// without an account, as tabs in a window of their own.
type settingsWindow struct {
	appWindow
	// tab is the pane it shows.
	tab model.SettingsPane
}

func newSettingsWindow() *settingsWindow { return &settingsWindow{tab: model.PaneGeneral} }

func (s *settingsWindow) view(c *ui.Context) {
	p := colors(c)
	ui.Column(c).Fill().Background(p.Content).Children(func() {
		tabs := ui.Row(c).Justify(ui.Center).Gap(4).Padding(8, 0, 6, 0).Background(p.Window).Role(ui.RoleTabList)
		tabs.Children(func() {
			for _, pane := range []model.SettingsPane{model.PaneGeneral, model.PaneAdvanced} {
				selected := s.tab == pane
				tab := ui.ButtonBase(c).Column().Gap(3).MinWidth(72).Padding(5, 10, 4, 10).Radius(7).
					FontSize(11).TextColor(p.Label2).Role(ui.RoleTab).Selected(selected).Label(pane.Title())
				switch {
				case selected:
					tab.Background(p.SelectionInactive).TextColor(p.Accent)
				case tab.Hovered():
					tab.Background(p.Hover)
				}
				tab.Children(func() {
					symbol(c, pane.Symbol(), 20, 1.6)
					ui.Text(c, pane.Title()).SingleLine()
				})
				if tab.Clicked() {
					s.tab = pane
				}
			}
		})
		ui.Box(c).Height(1).Background(p.ToolbarLine)
		ui.Column(c).Grow(1).MinHeight(0).Children(func() {
			pane := settingsPane{w: &s.appWindow}
			if s.tab == model.PaneAdvanced {
				pane.advanced(c)
			} else {
				pane.general(c)
			}
		})
	})
}
