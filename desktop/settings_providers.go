package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// providers is the account's provider credentials: connected on any Device, used by every Runner.
// The built-in providers come first, then the custom ones in the order they were added, and a row
// to add one. After their footnote, each connected provider's review model.
func (s settingsPane) providers(c *ui.Context) {
	p := colors(c)
	s.frame(c, string(model.PaneProviders), func() {
		s.section(c, L("Credentials"), nil, func(k *card) {
			if len(store.Providers) == 0 {
				keyValueRow(c, k, L("Waiting for the CLI"), "", false, nil)
				return
			}
			for _, credential := range store.Providers {
				kind := credential.Kind
				// A subscription disconnects right here; an API key or a custom provider opens its sheet.
				disconnects := credential.IsConnected && !model.UsesAPIKey(kind) && !model.IsCustomKind(kind)
				action := L("Connect…")
				if credential.IsConnected {
					action = L("Edit…")
					if disconnects {
						action = L("Disconnect")
					}
				}
				o := statusRowOptions{
					Symbol:      model.ProviderSymbol(kind),
					Title:       model.ProviderName(kind, store.Providers),
					Subtitle:    model.ProviderSubtitle(kind) + " · " + credential.Detail,
					StateColor:  &p.Green,
					ActionTitle: action,
					Destructive: disconnects,
				}
				if credential.IsConnected {
					o.State = L("Connected")
				}
				ui.Column(c.Key(kind)).Children(func() {
					rowElement, row := statusRow(c, k, o)
					s.mark(c, rowElement, o.Title)
					if !row.Action {
						return
					}
					switch {
					case model.IsCustomKind(kind):
						s.w.presentCustomProvider(kind, nil, nil)
					case disconnects:
						store.DisconnectProvider(kind, func(error) {})
					default:
						s.w.presentConnectProvider(kind, credential.BaseURL, nil)
					}
				})
			}
			addElement, _ := actionRow(c, k, L("Custom"), actionRowOptions{Tint: &p.Label2, Action: L("Add Provider…"), Menu: s.w.addProviderMenu})
			s.mark(c, addElement, L("Custom"))
		})
		s.footnote(c, L("Credentials belong to your account. They reach your paired Devices encrypted with the account key, so a bot uses them on whichever Runner it is assigned to; the relay stores ciphertext."))
		s.reviewModels(c)
	})
}

// reviewModels picks the model Auto-review runs on each connected provider, decision providers
// included: its default, the catalog's small one or a custom provider's first, or any of its
// models, decision models too. A picked model the provider no longer lists still shows, by its id.
// Hidden while no provider is connected.
func (s settingsPane) reviewModels(c *ui.Context) {
	kinds := store.ReviewProviderKinds()
	if len(kinds) == 0 {
		return
	}
	catalog := model.WithCustomModels(store.Models, store.Providers)
	s.section(c, reviewModelsEntry().row, nil, func(k *card) {
		for _, kind := range kinds {
			models := model.ReviewModels(catalog, kind)
			defaultTitle := L("Default")
			if credential := store.Credential(kind); credential != nil && credential.ReviewModel != "" {
				name := credential.ReviewModel
				for _, each := range models {
					if each.ID == credential.ReviewModel {
						name = each.Label
					}
				}
				defaultTitle = L("Default (%@)", name)
			}
			picked := store.ReviewModel(kind)
			options := []popUpOption{{Value: "", Label: defaultTitle}}
			listed := picked == ""
			for i, each := range models {
				options = append(options, popUpOption{Value: each.ID, Label: each.Label, Separated: i == 0})
				listed = listed || each.ID == picked
			}
			if !listed {
				options = append(options, popUpOption{Value: picked, Label: picked, Separated: len(models) == 0})
			}
			title := model.ProviderName(kind, store.Providers)
			ui.Column(c.Key("review-" + kind)).Children(func() {
				accessoryRow(c, k, title, "", func() {
					if id, changed, _ := popUpButton(c, popUp{Options: options, Value: picked, Style: popUpSettings, Label: title}); changed {
						store.SetReviewModel(id, kind)
					}
				})
			})
		}
		noteRow(c, k, L("Auto-review runs the review model of the bot's provider, or of the provider picked in Auto-review. A decision model writes no rule, so a card it pauses offers Allow once and Deny."), nil)
	})
}
