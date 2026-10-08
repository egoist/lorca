package main

import (
	"strings"

	"github.com/egoist/mygo/ui"
)

// A description edited in a sheet of its own, after the macOS app's DescriptionViewController, opened
// from a compact row in the inspector: a bot's (what it does and how it should work) or a group's
// (what it is for). Saving publishes it, so the next turn and every paired Device see it.

// presentDescription edits `initial` under `subtitle`; save gets the trimmed text.
func (w *appWindow) presentDescription(subtitle, initial string, save func(description string)) {
	text := initial
	w.present(func(c *ui.Context, s *sheet) {
		result := sheetFrame(c, sheetOptions{
			Title:           L("Description"),
			Subtitle:        subtitle,
			Width:           520,
			Confirm:         L("Save"),
			ReturnInContent: true,
		}, func() {
			textArea(c, &text, 0, fieldOptions{AutoFocus: true, Label: L("Description")}).Height(280)
		})
		switch {
		case result.Confirmed:
			save(strings.TrimSpace(text))
			s.dismiss()
		case result.Cancelled:
			s.dismiss()
		}
	}, nil)
}

// presentBotDescription edits what a bot does and how it should work.
func (w *appWindow) presentBotDescription(botID string) {
	initial := ""
	if bot := store.Bot(botID); bot != nil {
		initial = bot.Description
	}
	w.presentDescription(L("What it does and how it should work"), initial, func(description string) {
		if bot := store.Bot(botID); bot != nil && description != bot.Description {
			store.UpdateBotProfile(bot.ID, bot.Name, &description)
		}
	})
}

// presentGroupDescription edits what a group is for, which every bot in it reads.
func (w *appWindow) presentGroupDescription(chatID string) {
	initial := ""
	if chat := store.Chat(chatID); chat != nil {
		initial = chat.GroupDescription
	}
	w.presentDescription(L("What this group is for. Every bot in it reads this."), initial, func(description string) {
		store.SetDescription(description, chatID)
	})
}
