package main

import (
	"fmt"
	"math"
	"runtime"
	"slices"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// The account's attention, after the macOS app's AttentionViewController: a flyout under the
// header's Attention button with each coordinator's latest brief, then what waits on the user,
// urgent first. A row opens its chat, and an item waiting on a review the inspector lists opens
// that review's sheet over it; its buttons open another source chat or mark it resolved, and a
// brief's dismisses it. The ⋯ menu holds the notification switches and the coordinator. Bots keep
// the list; the CLI sends it whole (attention.changed), and the flyout follows it while it shows.

// attentionWidth is the flyout's, as the Mac's popover.
const attentionWidth = 380

type attentionState struct {
	// open is the flyout showing, or asked for before the button had a place to anchor it.
	open   bool
	flyout flyoutWindow
	// height is the flyout's, fitted to its content.
	height int
}

// flyoutWindow is what the header's flyouts (Attention, Running tasks) need of a mygo flyout;
// tests stand one in.
type flyoutWindow interface {
	Close()
	Invalidate()
	SetContentSize(width, height int)
}

// openFlyout opens a flyout of native UI under `anchor`, in `parent`'s content area, that
// takes the keyboard and closes as the user clicks elsewhere or presses Escape; `closed` runs once
// it has. On macOS it is a system popover, with its material and an arrow at the anchor's middle
// (AppKit places it, so Placement's alignment and Gap apply only elsewhere).
var openFlyout = func(parent *mygo.Window, anchor ui.Rect, width, height int, view func(c *ui.Context), closed func()) flyoutWindow {
	w := mygo.NewFlyout(mygo.FlyoutOptions{
		Parent:    parent,
		Anchor:    mygo.Rectangle{X: int(anchor.X), Y: int(anchor.Y), Width: int(anchor.W), Height: int(anchor.H)},
		Placement: mygo.PlacementBottomEnd,
		Gap:       6,
		Width:     width,
		Height:    height,
		Focusable: true,
		Shadow:    true,
		Popover:   true,
		Content:   ui.View(view),
	})
	w.OnClosed(closed)
	return w
}

// attentionButton is shown while anything waits on the user, or while its flyout is open, with
// how many as its badge.
func (m *mainWindow) attentionButton(c *ui.Context) {
	p := colors(c)
	s := &m.attention
	count := len(store.Attention.Items)
	if count == 0 && !s.open {
		return
	}
	holder := ui.Box(c.Key("attention"))
	var button ui.Element
	holder.Children(func() {
		button = hoverButton(c, hoverButtonOptions{Symbol: "tray.full", Tooltip: L("Attention (%@)", shortcutText("CmdOrCtrl+Shift+A")), Label: L("Attention (%d)", count), Active: s.open})
		if count > 0 {
			ui.Row(c).Absolute().Top(-2).Right(-4).MinWidth(15).Height(15).Padding(0, 4).Radius(8).Justify(ui.Center).Background(p.Red).PassThrough().Children(func() {
				ui.Text(c, fmt.Sprint(count)).FontSize(10).FontWeight(600).TextColor(p.White).FontFeatures("tnum")
			})
		}
	})
	if button.Clicked() {
		if s.open {
			m.closeAttention()
		} else {
			s.open = true
		}
	}
	// The button is placed a frame after it first shows: the flyout opens under it then.
	if s.open && s.flyout == nil {
		if anchor := button.Bounds(); anchor.W > 0 {
			m.openAttention(anchor)
		} else {
			c.Invalidate()
		}
	}
}

func (m *mainWindow) openAttention(anchor ui.Rect) {
	// One header flyout shows at a time.
	if m.tasks.open {
		m.closeRunningTasks()
	}
	s := &m.attention
	s.height = flyoutHeight(attentionWidth, m.attentionPanel)
	var flyout flyoutWindow
	flyout = openFlyout(m.win, anchor, attentionWidth, s.height, m.attentionFlyout, func() {
		if s.flyout == flyout {
			s.flyout, s.open = nil, false
			m.invalidate()
		}
	})
	s.flyout = flyout
}

func (m *mainWindow) closeAttention() {
	s := &m.attention
	flyout := s.flyout
	s.flyout, s.open = nil, false
	if flyout != nil {
		flyout.Close()
	}
	m.invalidate()
}

// toggleAttention is View › Attention: the flyout opens under the button, which shows for it even
// with nothing to list.
func (m *mainWindow) toggleAttention() {
	if m.isSettings() {
		return
	}
	if m.attention.open {
		m.closeAttention()
		return
	}
	m.attention.open = true
	m.invalidate()
}

// attentionFlyout is the flyout's content: the panel, which the flyout follows as the list
// changes while it shows.
func (m *mainWindow) attentionFlyout(c *ui.Context) {
	fitFlyout(m.attention.flyout, &m.attention.height, attentionWidth, m.attentionPanel(c))
}

// attentionPanel is what the flyout draws.
func (m *mainWindow) attentionPanel(c *ui.Context) ui.Element {
	return flyoutPanel(c, attentionWidth).Children(func() { m.attentionList(c) })
}

// flyoutPanel is the root of a header flyout's content, `width` wide. On macOS the flyout is a
// system popover, whose material shows through; elsewhere the window draws nothing, and the panel
// its own background.
func flyoutPanel(c *ui.Context, width int) ui.Element {
	applyTheme(c)
	p := colors(c)
	c.Root().Background(ui.Transparent)
	panel := ui.Column(c).Width(float32(width)).Shrink(0)
	switch {
	case c.Vibrancy():
	case runtime.GOOS == "windows":
		panel.Background(p.Popover)
	default:
		panel.Background(p.Popover).Radius(10).Border(0.5, p.ShadowEdge)
	}
	return panel
}

// flyoutHeight is the height of what `panel` draws, laid out offscreen, for a flyout to open at: a
// window sized after it opens jumps.
func flyoutHeight(width int, panel func(c *ui.Context) ui.Element) int {
	var height float32
	// An element's bounds are its last layout's: the second pass reads the first's.
	tt := ui.NewTester(func(c *ui.Context) { height = panel(c).Bounds().H }, width, 1000)
	tt.Frame()
	return max(int(math.Ceil(float64(height))), 1)
}

// fitFlyout sizes `flyout`, while it shows, to `panel`'s height as its content changes; `height`
// is the one it has.
func fitFlyout(flyout flyoutWindow, height *int, width int, panel ui.Element) {
	if h := int(math.Ceil(float64(panel.Bounds().H))); h > 0 && h != *height && flyout != nil {
		*height = h
		flyout.SetContentSize(width, h)
	}
}

func (m *mainWindow) attentionList(c *ui.Context) {
	p := colors(c)
	attention := store.Attention
	ui.Row(c).Height(40).Padding(2, 8, 0, 16).Children(func() {
		ui.Text(c, L("Attention")).FontSize(13).FontWeight(600)
		ui.Spacer(c)
		hoverButton(c.Key("attention-options"), hoverButtonOptions{Symbol: "ellipsis.circle", Tooltip: L("Options")}).Menu(m.attentionOptions)
	})
	if len(attention.Items) == 0 && len(attention.Briefs) == 0 {
		ui.Row(c).Height(52).Justify(ui.Center).Padding(0, 0, 8, 0).Children(func() {
			ui.Text(c, L("Nothing needs your attention")).FontSize(12.5).TextColor(p.Label2)
		})
		return
	}
	ui.Scroll(c).MaxHeight(480).Padding(0, 0, 6, 0).Children(func() {
		first := true
		separate := func(row ui.Element) {
			if !first {
				line := p.Separator
				row.DrawOver(func(painter *ui.Painter, r ui.Rect) {
					painter.Fill(ui.Rect{X: r.X + 10, Y: r.Y - 1, W: r.W - 20, H: 1}, line, 0)
				})
			}
			first = false
		}
		for _, brief := range attention.Briefs {
			separate(m.attentionBrief(c.Key("brief-"+brief.CoordinatorBotID), brief))
		}
		for _, item := range attention.Items {
			separate(m.attentionItem(c.Key("item-"+item.ID), item))
		}
	})
}

// attentionRow is a row lit under the pointer, as a menu item is, that opens `chatID` on
// `messageID` when there is one, then
// `review`'s sheet when there is one.
func (m *mainWindow) attentionRow(c *ui.Context, chatID, messageID string, review *model.ReviewItem, label string, children func()) ui.Element {
	p := colors(c)
	outer := ui.Box(c).Padding(1, 6)
	var row ui.Element
	outer.Children(func() {
		row = ui.Row(c).AlignItems(ui.Start).Gap(6).Padding(10, 10).Radius(6).Label(label).Role(ui.RoleButton)
		if row.Hovered() {
			row.Background(p.Hover)
		}
		row.Children(children)
	})
	if row.Clicked() && store.Chat(chatID) != nil {
		m.showAttentionChat(chatID, messageID)
		if review != nil && !m.hasSheet() {
			m.presentReview(*review)
		}
	}
	return row
}

// attentionBrief is "Project Manager · 9:41 AM", then what was decided, what changed, and what
// comes next.
func (m *mainWindow) attentionBrief(c *ui.Context, brief model.AttentionBrief) ui.Element {
	p := colors(c)
	bot := store.Bot(brief.CoordinatorBotID)
	name := L("Coordinator")
	if bot != nil {
		name = bot.Name
	}
	type line struct{ key, text string }
	var lines []line
	for _, text := range brief.Decisions {
		lines = append(lines, line{L("Decision"), text})
	}
	for _, text := range brief.Changes {
		lines = append(lines, line{L("Changed"), text})
	}
	lines = append(lines, line{L("Next"), brief.NextAction})
	spoken := []string{L("%@’s brief", name)}
	keys := make([]string, 0, len(lines))
	for _, l := range lines {
		spoken = append(spoken, l.key+": "+l.text)
		keys = append(keys, l.key)
	}
	keyWidth := mcpFormLabelWidth(c, keys...)
	row := m.attentionRow(c, brief.ChatID, "", nil, strings.Join(spoken, "\n"), func() {
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(8).Children(func() {
			ui.Row(c).Gap(6).Children(func() {
				avatar(c, botAvatarOrSystem(bot), 18, false)
				ui.Text(c, name).Grow(1).Shrink(1).MinWidth(0).FontSize(12.5).FontWeight(600).SingleLine()
				ui.Text(c, model.Stamp(time.Unix(int64(brief.UpdatedAt), 0))).FontSize(11).TextColor(p.Label2)
			})
			ui.Column(c).Gap(4).Children(func() {
				for _, l := range lines {
					ui.Row(c).Gap(8).AlignItems(ui.Start).Children(func() {
						ui.Text(c, l.key).Width(keyWidth).FontSize(11.5).LineHeight(1.4).TextColor(p.Label2)
						ui.Text(c, l.text).Grow(1).Shrink(1).MinWidth(0).FontSize(12).LineHeight(1.35).MaxLines(4)
					})
				}
			})
			attentionActions(c, func() {
				if pushButton(c.Key("dismiss"), L("Dismiss"), pushOptions{Small: true}).Clicked() {
					store.DismissBrief(brief.CoordinatorBotID, brief.Revision, m.attentionFailed)
				}
			})
		})
	})
	return row
}

// attentionItem is the category's symbol, the title, the next action, and "Review · Launch copy".
func (m *mainWindow) attentionItem(c *ui.Context, item model.AttentionItem) ui.Element {
	p := colors(c)
	category, symbolName := attentionCategory(item.Category)
	tint, status, statusColor := p.Accent, category, p.Label2
	if item.Urgent {
		tint, status, statusColor = p.Red, L("Urgent"), p.Red
	}
	chats := attentionChats(item)
	titles := make([]string, len(chats))
	for i, chat := range chats {
		titles[i] = store.Title(chat)
	}
	first := ""
	if len(chats) > 0 {
		first = chats[0].ID
	}
	review := attentionReview(item)
	message := attentionMessage(item, first)
	if review != nil {
		first = review.Origin.ChatID
		message = review.Origin.MessageID
	}
	row := m.attentionRow(c, first, message, review, strings.Join([]string{status, item.Title, item.NextAction, item.Summary}, ". "), func() {
		ui.Row(c).Width(18).Padding(1, 0, 0, 0).Justify(ui.Center).TextColor(tint).Children(func() { symbol(c, symbolName, 15, 1.9) })
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(3).Children(func() {
			ui.Text(c, item.Title).FontSize(12.5).FontWeight(600).LineHeight(1.3).MaxLines(2)
			ui.Text(c, item.NextAction).FontSize(12).LineHeight(1.35).MaxLines(3)
			ui.Row(c).Margin(1, 0, 0, 0).MinWidth(0).Children(func() {
				ui.Text(c, status).FontSize(11).TextColor(statusColor).SingleLine()
				if len(titles) > 0 {
					ui.Text(c, " · "+strings.Join(titles, ", ")).Shrink(1).MinWidth(0).FontSize(11).TextColor(p.Label2).SingleLine()
				}
			})
			attentionActions(c, func() {
				if pushButton(c.Key("resolve"), L("Mark as Resolved"), pushOptions{Small: true}).Clicked() {
					m.resolveAttention(item)
				}
				// The row opens the first chat; another source chat has its own button.
				if len(chats) > 1 {
					for _, chat := range chats {
						if pushButton(c.Key("open-"+chat.ID), L("Open “%@”", store.Title(chat)), pushOptions{Small: true}).Clicked() {
							m.showAttentionChat(chat.ID, attentionMessage(item, chat.ID))
						}
					}
				}
			})
		})
	})
	return row
}

// attentionActions is a row's buttons, under its text, wrapping onto more lines as they need.
func attentionActions(c *ui.Context, buttons func()) {
	ui.Row(c).Wrap().Gap(6).Margin(4, 0, 0, 0).Children(buttons)
}

func attentionCategory(category string) (title, symbol string) {
	switch category {
	case "review":
		return L("Review"), "doc.text.magnifyingglass"
	case "blocker":
		return L("Blocker"), "hand.raised"
	case "commitment":
		return L("Commitment"), "calendar"
	}
	return L("Change"), "arrow.triangle.2.circlepath"
}

// attentionMessage is the message in `chatID` the item cites first, or "".
func attentionMessage(item model.AttentionItem, chatID string) string {
	for _, source := range item.Sources {
		if source.ChatID == chatID && source.MessageID != "" {
			return source.MessageID
		}
	}
	return ""
}

// showAttentionChat closes the flyout and opens `chatID`, then brings `messageID` into view and
// pulses it, as a quote does, loading older pages until it is there. One that is gone leaves the
// chat at its end.
func (m *mainWindow) showAttentionChat(chatID, messageID string) {
	m.closeAttention()
	m.selectChat(chatID)
	if messageID == "" {
		return
	}
	store.LoadMessage(chatID, messageID, func(err error) {
		if err != nil || m.selection.ChatID != chatID {
			return
		}
		chat := m.chatStateFor(chatID)
		chat.rows = buildRows(store.Chat(chatID), store.WorkingBots(chatID), chat.stoppedNotice)
		m.revealMessage(chat, shownMessage(chat.rows, store.Chat(chatID), messageID))
	})
}

// shownMessage is `messageID`, or, for a message the transcript draws no row for (a tool call
// that needs nobody), the nearest one before it that has a row, else the nearest after it.
func shownMessage(rows []chatRow, chat *model.Chat, messageID string) string {
	shown := map[string]bool{}
	for _, row := range rows {
		if row.kind == rowMessage {
			shown[row.message.ID] = true
		}
	}
	if shown[messageID] || chat == nil {
		return messageID
	}
	at := slices.IndexFunc(chat.Messages, func(m *model.Message) bool { return m.ID == messageID })
	if at < 0 {
		return messageID
	}
	for i := at - 1; i >= 0; i-- {
		if shown[chat.Messages[i].ID] {
			return chat.Messages[i].ID
		}
	}
	for _, m := range chat.Messages[at+1:] {
		if shown[m.ID] {
			return m.ID
		}
	}
	return messageID
}

// attentionChats are the chats an item came from, once each, that still exist.
// attentionReview is the review the item waits on while it is open and decided in a sheet, not a
// draft card in the chat.
func attentionReview(item model.AttentionItem) *model.ReviewItem {
	for _, source := range item.Sources {
		if source.ReviewID == "" {
			continue
		}
		if review := store.Review(source.ReviewID); review != nil && review.Open() && !review.IsMessage && store.Chat(review.Origin.ChatID) != nil {
			return review
		}
	}
	return nil
}

func attentionChats(item model.AttentionItem) []*model.Chat {
	var chats []*model.Chat
	seen := map[string]bool{}
	for _, source := range item.Sources {
		if chat := store.Chat(source.ChatID); chat != nil && !seen[chat.ID] {
			seen[chat.ID] = true
			chats = append(chats, chat)
		}
	}
	return chats
}

// attentionOptions is the ⋯ menu: Notifications (Briefs, Urgent Items) and the Coordinator
// (Automatic: a group's owner, a direct chat's bot; or one bot for every chat without an owner).
func (m *mainWindow) attentionOptions(menu *ui.Menu) {
	preferences := store.Attention.Preferences
	menu.Item(L("Notifications")).Disabled(true)
	if menu.Item(L("Briefs")).Checked(preferences.Summaries).Chosen() {
		m.setAttentionPreference("summaries", !preferences.Summaries)
	}
	if menu.Item(L("Urgent Items")).Checked(preferences.UrgentDirect).Chosen() {
		m.setAttentionPreference("urgent_direct", !preferences.UrgentDirect)
	}
	menu.Separator()
	menu.Item(L("Coordinator")).Disabled(true)
	if menu.Item(L("Automatic")).Checked(preferences.DefaultCoordinatorBotID == nil).Chosen() {
		m.setAttentionPreference("default_coordinator_bot_id", nil)
	}
	for _, bot := range store.Bots {
		chosen := preferences.DefaultCoordinatorBotID != nil && *preferences.DefaultCoordinatorBotID == bot.ID
		if menu.Item(bot.Name).Checked(chosen).Chosen() {
			m.setAttentionPreference("default_coordinator_bot_id", bot.ID)
		}
	}
}

func (m *mainWindow) resolveAttention(item model.AttentionItem) {
	store.ResolveAttention(item.ID, item.Revision, m.attentionFailed)
}

func (m *mainWindow) setAttentionPreference(key string, value any) {
	store.SetAttentionPreference(key, value, m.attentionFailed)
}

// attentionFailed says what went wrong in an alert, as the Mac's does.
func (m *mainWindow) attentionFailed(err error) {
	if err == nil {
		return
	}
	m.showAlert(alertOptions{Message: L("Couldn’t update Attention"), Informative: model.ErrorText(err)}, nil)
}
