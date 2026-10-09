package main

import (
	"fmt"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The account's attention, after the macOS app's AttentionViewController: a popover under the
// header's Attention button with each coordinator's latest brief, then what waits on the user,
// urgent first. A row opens its chat; its menu opens another source chat or marks it resolved. The
// ⋯ menu holds the notification switches and the coordinator. Bots keep the list; the CLI sends
// it whole (attention.changed), and the popover follows it while it shows.

// attentionWidth is the popover's, as the Mac's.
const attentionWidth = 380

type attentionState struct {
	open bool
}

// attentionButton is shown while anything waits on the user, or while its popover is open, with
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
		s.open = !s.open
	}
	ui.PopoverBase(c, button, &s.open, func(panel ui.Element) {
		popoverPanel(c, panel).Width(attentionWidth)
		m.attentionList(c)
	})
}

// toggleAttention is View › Attention: the popover opens over the button, which shows for it even
// with nothing to list.
func (m *mainWindow) toggleAttention() {
	if m.isSettings() {
		return
	}
	m.attention.open = !m.attention.open
	m.invalidate()
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

// attentionRow is a row lit under the pointer, as a menu item is, that opens `chatID`.
func (m *mainWindow) attentionRow(c *ui.Context, chatID, label string, children func()) ui.Element {
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
		m.attention.open = false
		m.selectChat(chatID)
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
	return m.attentionRow(c, brief.ChatID, strings.Join(spoken, "\n"), func() {
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
		})
	})
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
	row := m.attentionRow(c, first, strings.Join([]string{status, item.Title, item.NextAction, item.Summary}, ". "), func() {
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
		})
	})
	row.Tooltip(item.Summary)
	row.ContextMenu(func(menu *ui.Menu) {
		for _, chat := range chats {
			if menu.Item(L("Open “%@”", store.Title(chat))).Chosen() {
				m.attention.open = false
				m.selectChat(chat.ID)
			}
		}
		if len(chats) > 0 {
			menu.Separator()
		}
		if menu.Item(L("Mark as Resolved")).Chosen() {
			m.resolveAttention(item)
		}
	})
	return row
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

// attentionChats are the chats an item came from, once each, that still exist.
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
