package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A bot's settings in a sheet of their own, opened by a click on the bot in the inspector (or its
// menu in a group), a tab per page: its profile and memory, what it runs with and where its turns
// run, its skills, and the plugins on its Runner. The inspector keeps what goes on in the chat.

// presentBotSettings opens the bot's settings on Profile. Its memory is asked for again as it
// opens; the sheet closes itself if the bot goes away.
func (m *mainWindow) presentBotSettings(botID string) {
	m.inspector.refreshMemory(botID)
	tab := 0
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
			ui.Row(c).Justify(ui.Center).Children(func() {
				segmented(c.Key("tabs"), &tab, L("Bot Settings"), L("Profile"), L("Runs with"), L("Skills"), L("Plugins"))
			})
			ui.Column(c.Key(tab)).Gap(20).Padding(4, 0, 2, 0).Children(func() {
				switch tab {
				case 1:
					m.inspectorRuntime(c, bot)
					m.inspectorRouting(c, []*model.Bot{bot})
				case 2:
					m.inspectorSkillsTab(c, model.BotScope(bot.ID))
				case 3:
					m.inspectorPlugins(c, bot)
				default:
					m.inspectorProfile(c, bot)
					m.inspectorMemory(c, bot)
				}
			})
		})
		if result.Confirmed || result.Cancelled {
			s.dismiss()
		}
	}, nil)
}
