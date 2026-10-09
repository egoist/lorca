package main

import (
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// sharedLinks is the bots the account shares as links, from the roster, after the macOS app's
// SharedLinksSettingsViewController: a row opens its bot's Share sheet, where Update Link keeps the
// address; its menu copies the link or revokes it.
func (s settingsPane) sharedLinks(c *ui.Context) {
	s.frame(c, string(model.PaneSharedLinks), func() {
		s.section(c, L("Shared Links"), nil, func(k *card) {
			if len(store.SharedLinks) == 0 {
				noteRow(c, k, L("No shared links yet. Share a bot from its chat's menu or File › Share as Template…"), nil)
				return
			}
			for i := len(store.SharedLinks) - 1; i >= 0; i-- {
				link := store.SharedLinks[i]
				bot := store.Bot(link.BotID)
				shown := bot
				if shown == nil {
					// A deleted bot's link still works until it is revoked.
					shown = &model.Bot{ID: link.BotID, Name: link.Name, SymbolName: "link", Accent: "indigo"}
				}
				updated := model.Stamp(time.Unix(int64(link.UpdatedAt), 0))
				row, result := botRow(c, k, shown, botRowOptions{Detail: L("Updated %@", updated), Clickable: bot != nil})
				row.Tooltip(link.URL)
				row.ContextMenu(func(menu *ui.Menu) {
					if menu.Item(L("Copy Link")).Chosen() {
						copyText(link.URL)
					}
					menu.Separator()
					if menu.Item(L("Revoke Link…")).Chosen() {
						s.w.revokeLink(link)
					}
				})
				if result.Clicked {
					s.w.presentTemplateShare(bot.ID)
				}
			}
		})
		s.footnote(c, L("Anyone with a link can add their own copy of the bot. Revoking a link stops it working; copies already added stay."))
	})
}

// revokeLink asks before it takes the link down.
func (w *appWindow) revokeLink(link model.SharedLink) {
	w.showAlert(alertOptions{
		Message:     L("Revoke the link to “%@”?", link.Name),
		Informative: L("Whoever opens it sees that it no longer works. Bots already added from it stay."),
		Buttons:     []alertButton{{Title: L("Revoke"), Destructive: true}, {Title: L("Cancel")}},
	}, func(index int) {
		if index != 0 {
			return
		}
		store.RevokeLink(link.ID, func(err error) {
			if err != nil {
				w.showNote(L("Couldn't revoke the link"), model.ErrorText(err))
			}
		})
	})
}
