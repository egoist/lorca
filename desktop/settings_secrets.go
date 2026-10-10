package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// settingsSecretsState is the picked Runner's secrets as it last answered, and which Runner that
// was. The list is asked when the pane shows a Runner and after each change made here.
type settingsSecretsState struct {
	deviceID string
	list     []model.SavedSecret
	loaded   bool
	failure  string
	// loads counts the asks, so an answer that an ask after it overtook is dropped.
	loads int
}

func (s *settingsSecretsState) load(runnerID string) {
	s.loads++
	load := s.loads
	store.Secrets(runnerID, func(list []model.SavedSecret, err error) {
		if load != s.loads {
			return
		}
		if err != nil {
			s.failure = model.ErrorText(err)
			return
		}
		s.list, s.loaded, s.failure = list, true, ""
	})
}

// secretPlace is where a secret goes: the site it is typed into, or the bot's commands.
func secretPlace(secret model.SavedSecret) string {
	if secret.Use == model.SecretBrowser {
		return firstNonEmpty(secret.Site, L("Browser"))
	}
	return L("Commands")
}

// secrets is the picked Runner's secrets for its bots, after the macOS app's
// SecretsSettingsViewController: what each is, whose it is, and where it goes, never its value. A
// row opens it to type a new value or delete it; its menu does either.
func (s settingsPane) secrets(c *ui.Context, m *mainWindow) {
	device := store.Device(m.settingsDeviceID)
	title := L("Secrets")
	if device != nil {
		title = L("Secrets on %@", device.Name)
	}
	state := &s.page.secrets
	if device != nil && device.IsRunner() && state.deviceID != device.ID {
		state.deviceID, state.list, state.loaded, state.failure = device.ID, nil, false, ""
		state.load(device.ID)
	}
	s.frame(c, string(model.PaneSecrets)+"|"+m.settingsDeviceID, func() {
		s.section(c, title, nil, func(k *card) {
			settingsRunnerRows(c, k, device, func() {
				switch {
				case !state.loaded && state.failure != "":
					noteRow(c, k, state.failure, nil)
				case !state.loaded:
					keyValueRow(c, k, L("Loading…"), "", false, nil)
				case len(state.list) == 0:
					noteRow(c, k, L("No secrets on %@ yet.", device.Name), nil)
				}
				for _, secret := range state.list {
					bot := store.Bot(secret.BotID)
					owner := L("A deleted bot")
					shown := &model.Bot{ID: secret.BotID, Name: owner, SymbolName: "lock", Accent: "indigo"}
					if bot != nil {
						owner, shown = bot.Name, bot
					}
					row, result := botRow(c, k, shown, botRowOptions{Title: secret.Label, Detail: owner + " · " + secretPlace(secret), Clickable: true})
					row.ContextMenu(func(menu *ui.Menu) {
						if menu.Item(L("Replace…")).Chosen() {
							s.w.presentSecret(secret, device, state)
						}
						menu.Separator()
						if menu.Item(L("Delete…")).Chosen() {
							s.w.confirmDeleteSecret(secret, device, state, nil)
						}
					})
					if result.Clicked {
						s.w.presentSecret(secret, device, state)
					}
				}
			})
		})
		s.footnote(c, L("A bot asks in the chat when it needs a password or a key. What you save stays encrypted on its Runner, and the bot uses it by name without ever seeing it."))
	})
}

// secretOwner is the bot a secret is for, by name.
func secretOwner(secret model.SavedSecret) string {
	if bot := store.Bot(secret.BotID); bot != nil {
		return bot.Name
	}
	return L("The bot")
}

// presentSecret is one of a Runner's secrets, after the macOS app's SecretViewController: what it
// is and where its bot uses it, a field for a new value, and Delete apart from Replace. The value
// is never shown; a new one goes sealed to the Runner.
func (w *appWindow) presentSecret(secret model.SavedSecret, device *model.Device, list *settingsSecretsState) {
	var value, failure string
	busy := false
	who := secretOwner(secret)
	subtitle := L("%@'s commands use it.", who)
	if secret.Use == model.SecretBrowser {
		subtitle = L("%@ signs in to %@ with it.", who, firstNonEmpty(secret.Site, L("its sign-in page")))
	}
	w.present(func(c *ui.Context, s *sheet) {
		p := colors(c)
		result := sheetFrame(c, sheetOptions{
			Title: secret.Label, Subtitle: subtitle, Confirm: L("Replace"), ConfirmDisabled: busy || value == "",
			Leading: func() {
				if pushButton(c, L("Delete…"), pushOptions{Kind: buttonDestructive, Disabled: busy}).Clicked() {
					w.confirmDeleteSecret(secret, device, list, s.dismiss)
				}
			},
		}, func() {
			field := textField(c.Key("value"), &value, fieldOptions{Secure: true, Placeholder: L("New value"), Label: L("New value for %@", secret.Label), AutoFocus: true, Disabled: busy})
			if field.Changed() {
				failure = ""
			}
			if failure != "" {
				ui.Text(c, failure).FontSize(textCaption).TextColor(p.Red)
			}
		})
		if result.Cancelled {
			s.dismiss()
		}
		if result.Confirmed && !busy && value != "" {
			busy = true
			store.ReplaceSecret(device.ID, secret.ID, value, func(err error) {
				busy = false
				if err != nil {
					failure = model.ErrorText(err)
					return
				}
				list.load(device.ID)
				s.dismiss()
			})
		}
	}, nil)
}

// confirmDeleteSecret asks before deleting a secret from its Runner, then deletes it; a failure
// says why.
func (w *appWindow) confirmDeleteSecret(secret model.SavedSecret, device *model.Device, list *settingsSecretsState, done func()) {
	w.showAlert(alertOptions{
		Message:     L("Delete “%@”?", secret.Label),
		Informative: L("%@ asks for it again the next time it needs it.", secretOwner(secret)),
		Buttons:     []alertButton{{Title: L("Delete"), Destructive: true}, {Title: L("Cancel")}},
	}, func(index int) {
		if index != 0 {
			return
		}
		store.DeleteSecret(device.ID, secret.ID, func(err error) {
			if err != nil {
				w.showAlert(alertOptions{Message: L("Couldn't delete “%@”", secret.Label), Informative: model.ErrorText(err)}, nil)
				return
			}
			list.load(device.ID)
			if done != nil {
				done()
			}
		})
	})
}
