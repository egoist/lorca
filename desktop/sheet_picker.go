package main

import (
	"slices"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Picking bots, after the macOS app's NewGroupChatViewController and BotPickerViewController: one to
// six bots for a new group chat, whose members change later, or one bot to add to a chat. Direct
// chats need no picker because every bot gets one when it is made.

// pickerBotRow is a bot to pick: a check, its avatar, its name over its provider and Runner, and
// "offline" when the Runner is. It reports a click while it is enabled.
func pickerBotRow(c *ui.Context, k *card, bot *model.Bot, selected, enabled bool) bool {
	p := colors(c)
	runner := store.Device(bot.RunnerID)
	r := k.row(rowBox(c.Key(bot.ID)).MinHeight(46).Padding(0, 12).Label(bot.Name).Role(ui.RoleCheckBox).Checked(selected))
	clicked := false
	if enabled {
		r.Cursor(ui.CursorPointer)
		if r.Hovered() {
			r.Background(p.RowHover)
		}
		clicked = r.Clicked()
	} else {
		r.Disabled(true)
	}
	r.Children(func() {
		check, tint := "circle", p.Label3
		if selected {
			check, tint = "checkmark.circle.fill", p.Accent
		}
		ui.Row(c).TextColor(tint).Children(func() { symbol(c, check, 16, 1.8) })
		avatar(c, botAvatar(bot), 28, false)
		runnerName := L("unassigned")
		if runner != nil {
			runnerName = runner.Name
		}
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			ui.Text(c, bot.Name).FontSize(12.5).FontWeight(500).SingleLine()
			ui.Text(c, model.ProviderName(bot.Provider, store.Providers)+" · "+runnerName).FontSize(textCaption).TextColor(p.Label2).SingleLine()
		})
		if runner != nil && runner.Status == model.StatusOffline {
			ui.Text(c, L("offline")).FontSize(10).FontWeight(500).TextColor(p.Orange)
		}
	})
	return clicked
}

// presentNewGroupChat picks the bots of a new group chat and its name ("" for none).
func (w *appWindow) presentNewGroupChat(onCreate func(botIDs []string, title string)) {
	bots := slices.Clone(store.Bots)
	var selected []string
	if len(bots) > 0 {
		selected = []string{bots[0].ID}
	}
	name := ""
	w.present(func(c *ui.Context, s *sheet) {
		result := sheetFrame(c, sheetOptions{
			Title:           L("New Group Chat"),
			Subtitle:        L("Pick up to six bots to talk with together."),
			Width:           440,
			Confirm:         L("Create"),
			ConfirmDisabled: len(selected) == 0,
		}, func() {
			toggled := ""
			section(c, L("Bots"), sectionCaption, nil, func(k *card) {
				for _, bot := range bots {
					picked := slices.Contains(selected, bot.ID)
					if pickerBotRow(c, k, bot, picked, picked || len(selected) < model.MaxGroupBots) {
						toggled = bot.ID
					}
				}
			})
			switch {
			case toggled == "":
			case slices.Contains(selected, toggled):
				selected = slices.DeleteFunc(slices.Clone(selected), func(id string) bool { return id == toggled })
			case len(selected) < model.MaxGroupBots:
				selected = append(slices.Clone(selected), toggled)
			default:
				beep()
			}
			textField(c, &name, fieldOptions{Placeholder: L("Group name (optional)"), Label: L("Group name (optional)")})
		})
		switch {
		case result.Confirmed:
			if len(selected) == 0 {
				return
			}
			if onCreate != nil {
				onCreate(selected, strings.TrimSpace(name))
			}
			s.dismiss()
		case result.Cancelled:
			s.dismiss()
		}
	}, nil)
}

// presentBotPicker picks one of `bots`.
func (w *appWindow) presentBotPicker(title string, bots []*model.Bot, onPick func(botID string)) {
	selected := ""
	if len(bots) > 0 {
		selected = bots[0].ID
	}
	w.present(func(c *ui.Context, s *sheet) {
		result := sheetFrame(c, sheetOptions{
			Title:           title,
			Subtitle:        L("A group chat holds up to six bots."),
			Width:           420,
			Confirm:         L("Add"),
			ConfirmDisabled: selected == "",
		}, func() {
			section(c, L("Available"), sectionCaption, nil, func(k *card) {
				for _, bot := range bots {
					if pickerBotRow(c, k, bot, bot.ID == selected, true) {
						selected = bot.ID
					}
				}
			})
		})
		switch {
		case result.Confirmed:
			if selected == "" {
				return
			}
			if onPick != nil {
				onPick(selected)
			}
			s.dismiss()
		case result.Cancelled:
			s.dismiss()
		}
	}, nil)
}
