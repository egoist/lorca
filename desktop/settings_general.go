package main

import (
	"strconv"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// General and Advanced: this computer's own settings, which work without an account, so the small
// Settings window shows them too.

// The updater's answers General shows; a test stands in for them.
var (
	settingsUpdatesEnabled = updatesEnabled
	settingsUpdaterState   = currentUpdaterState
)

// settingsSwitch reports a user change after applying bound input.
func settingsSwitch(c *ui.Context, on *bool, label string, disabled bool) bool {
	changed := false
	holder := ui.Row(c)
	if disabled {
		holder.Disabled(true)
	}
	holder.Children(func() {
		changed = toggleSwitch(c, on, true).Label(label).Changed()
	})
	return changed
}

func (s settingsPane) general(c *ui.Context) {
	p := colors(c)
	saved := prefs.get()
	s.frame(c, string(model.PaneGeneral), func() {
		s.section(c, L("Chats"), nil, func(k *card) {
			sendOnReturn, timestamps := sendOnReturnEntry().row, timestampsEntry().row
			s.mark(c, accessoryRow(c, k, sendOnReturn, "", func() {
				on := saved.SendOnReturn
				if settingsSwitch(c, &on, sendOnReturn, false) {
					setPrefs(PreferencesPatch{SendOnReturn: &on})
				}
			}), sendOnReturn)
			s.mark(c, accessoryRow(c, k, timestamps, "", func() {
				on := saved.ShowTimestamps
				if settingsSwitch(c, &on, timestamps, false) {
					setPrefs(PreferencesPatch{ShowTimestamps: &on})
				}
			}), timestamps)
		})
		s.section(c, L("Appearance"), nil, func(k *card) {
			appearance, appLanguage := appearanceEntry().row, appLanguageEntry().row
			s.mark(c, accessoryRow(c, k, appearance, "", func() {
				value := saved.Appearance
				if value != "light" && value != "dark" {
					value = ""
				}
				picked, changed, _ := popUpButton(c, popUp{
					Options: []popUpOption{{Value: "", Label: L("System")}, {Value: "light", Label: L("Light")}, {Value: "dark", Label: L("Dark")}},
					Value:   value,
					Style:   popUpSettings,
					Label:   appearance,
				})
				if changed {
					setPrefs(PreferencesPatch{Appearance: &picked})
				}
			}), appearance)
			// The app's own language. Each one is named in itself, so it reads whatever is showing.
			s.mark(c, accessoryRow(c, k, appLanguage, "", func() {
				options := []popUpOption{{Value: "", Label: L("System")}}
				for i, language := range l10n.Supported {
					options = append(options, popUpOption{Value: string(language.Code), Label: language.Name, Separated: i == 0})
				}
				picked, changed, _ := popUpButton(c, popUp{Options: options, Value: string(l10n.Chosen()), Style: popUpSettings, Label: appLanguage})
				if changed {
					setPrefs(PreferencesPatch{AppLanguage: &picked})
				}
			}), appLanguage)
		})
		if settingsUpdatesEnabled() {
			updater := settingsUpdaterState()
			s.section(c, L("Updates"), nil, func(k *card) {
				lastCheck := L("Never checked")
				if !updater.LastCheck.IsZero() {
					lastCheck = L("Last checked %@", model.DateTime(updater.LastCheck))
				}
				version := versionEntry().row
				rowElement, row := actionRow(c, k, version, actionRowOptions{Value: appVersion() + " · " + lastCheck, Tint: &p.Label2, Action: L("Check for Updates…")})
				s.mark(c, rowElement, version)
				if row.Action {
					checkForUpdates()
				}
				checks, downloads := automaticChecksEntry().row, automaticDownloadsEntry().row
				s.mark(c, accessoryRow(c, k, checks, "", func() {
					on := updater.AutomaticChecks
					if settingsSwitch(c, &on, checks, false) {
						setAutomaticUpdates(on, updater.AutomaticDownloads)
					}
				}), checks)
				s.mark(c, accessoryRow(c, k, downloads, "", func() {
					on := updater.AutomaticDownloads
					if settingsSwitch(c, &on, downloads, !updater.AutomaticChecks) {
						setAutomaticUpdates(updater.AutomaticChecks, on)
					}
				}), downloads)
			})
		}
		s.footnote(c, L("Lorca talks only to the CLI on this computer. Nothing here is synced; each Device keeps its own settings."))
	})
}

func (s settingsPane) advanced(c *ui.Context) {
	p := colors(c)
	saved := prefs.get()
	s.frame(c, string(model.PaneAdvanced), func() {
		s.section(c, L("Connection"), nil, func(k *card) {
			relay, port := relayURLEntry().row, cliPortEntry().row
			relayValue := store.RelayURL
			if relayValue == "" {
				relayValue = saved.RelayURL
			}
			s.markBox(c, relay, func() {
				if committed, ok := editableRow(c, k, relay, relayValue, productionRelayURL, true, false); ok && committed != store.RelayURL {
					setPrefs(PreferencesPatch{RelayURL: &committed})
					store.SetRelayURL(committed)
				}
			})
			// A port out of range leaves the row showing the port in force again. The app looks
			// for the CLI on a new one.
			s.markBox(c, port, func() {
				committed, ok := editableRow(c, k, port, strconv.Itoa(saved.CLIPort), strconv.Itoa(defaultCLIPort()), true, false)
				if number, err := strconv.Atoi(committed); ok && err == nil && number > 0 && number < 65536 && number != saved.CLIPort {
					setPrefs(PreferencesPatch{CLIPort: &number})
				}
			})
		})
		s.footnote(c, L("Self-hosting the relay is a URL change: clients sign their requests and upload ciphertext, so the relay has nothing to trust. Leave it empty to use Lorca’s relay."))
		s.section(c, L("Setup"), nil, func(k *card) {
			label := onboardingEntry().row
			rowElement, row := actionRow(c, k, label, actionRowOptions{Tint: &p.Label2, Action: L("Show Onboarding Again")})
			s.mark(c, rowElement, label)
			// Onboarding closes the window this row is in, so the click returns first.
			if row.Action {
				post(app.showOnboarding)
			}
		})
		if store.HasIdentity != nil && *store.HasIdentity {
			s.section(c, L("Account"), nil, func(k *card) {
				label := deleteAccountEntry().row
				rowElement, row := actionRow(c, k, label, actionRowOptions{Tint: &p.Label2, Action: L("Delete Account…")})
				s.mark(c, rowElement, label)
				if row.Action {
					s.w.confirmDeleteAccount()
				}
			})
			s.footnote(c, L("Deletes the account from the relay and from every paired Device: chats, attachments, bots, and provider credentials."))
		}
	})
}

// confirmDeleteAccount asks before deleting the account, then deletes it everywhere.
func (w *appWindow) confirmDeleteAccount() {
	w.showAlert(alertOptions{
		Message:     L("Delete this account?"),
		Informative: L("The relay deletes everything it holds for the account, and every paired Device, this one included, forgets its keys and chats. This can’t be undone."),
		Style:       alertCritical,
		Buttons:     []alertButton{{Title: L("Delete Account"), Destructive: true}, {Title: L("Cancel")}},
	}, func(answer int) {
		if answer != 0 {
			return
		}
		store.DeleteAccount(func(err error) {
			if err != nil {
				w.showAlert(alertOptions{Message: L("Couldn’t delete the account"), Informative: model.ErrorText(err)}, nil)
			}
		})
	})
}
