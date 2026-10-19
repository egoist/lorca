package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A bot's settings in a sheet of their own, opened by a click on the bot in the inspector (or its
// menu in a group): its profile, what it runs with, its memory, skills, the plugins on its
// Runner, and where its turns run. The inspector keeps what goes on in the chat.

// presentBotSettings opens the bot's settings. Its memory is asked for again as it opens; the sheet
// closes itself if the bot goes away.
func (m *mainWindow) presentBotSettings(botID string) {
	m.inspector.refreshMemory(botID)
	m.present(func(c *ui.Context, s *sheet) {
		bot := store.Bot(botID)
		if bot == nil {
			s.dismiss()
			return
		}
		result := sheetFrame(c, sheetOptions{
			Title:    bot.Name,
			Subtitle: L("Bot Settings"),
			Width:    480,
			Confirm:  L("Done"),
			NoCancel: true,
		}, func() {
			ui.Column(c).Gap(20).Padding(4, 0, 2, 0).Children(func() {
				m.inspectorProfile(c, bot)
				m.inspectorRuntime(c, bot)
				m.inspectorMemory(c, bot)
				m.inspectorSkills(c, model.BotScope(bot.ID))
				m.inspectorPlugins(c, bot)
				m.inspectorRouting(c, []*model.Bot{bot})
			})
		})
		if result.Confirmed || result.Cancelled {
			s.dismiss()
		}
	}, nil)
}
