package main

import (
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The account picker selects an installed instance explicitly. Its rows follow the selected
// Runner's public statuses; all account configuration and authorization stay in the CLI.
type integrationAccountsSheet struct {
	w                                *appWindow
	serviceID, serviceName, runnerID string
	runner                           model.Device
}

func (w *appWindow) presentPluginAccounts(serviceID, serviceName string, runner *model.Device) {
	s := &integrationAccountsSheet{w: w, serviceID: serviceID, serviceName: serviceName, runnerID: runner.ID, runner: *runner}
	w.present(s.view, nil)
}

func (s *integrationAccountsSheet) view(c *ui.Context, sh *sheet) {
	on := store.Device(s.runnerID)
	if on == nil {
		on = &s.runner
	}
	p := colors(c)
	result := sheetFrame(c, sheetOptions{Title: s.serviceName,
		Subtitle: L("Accounts on %@. Choose a name such as Work or Personal.", on.Name),
		Width:    560, Confirm: L("Done"), NoCancel: true}, func() {
		section(c, L("Accounts"), sectionCaption, nil, func(k *card) {
			accounts := store.PluginAccounts(s.serviceID, s.runnerID)
			if len(accounts) == 0 {
				keyValueRow(c, k, L("Accounts"), L("No accounts connected"), false, &p.Label2)
			}
			for _, account := range accounts {
				key := c.Key(account.ID)
				tint := p.tone(account.State.Tone())
				_, action := actionRow(key, k, firstNonEmpty(account.AccountName, account.Name),
					actionRowOptions{Value: account.Detail, Tint: &tint, Action: L("Manage…")})
				if action.Action {
					s.w.presentPlugin(account.ID, on)
				}
			}
		})
		if pushButton(c.Key("add-integration-account"), L("Add Account…"), pushOptions{}).Clicked() {
			s.w.presentIntegrationAccountName(s.serviceID, s.serviceName, on)
		}
	})
	if result.Confirmed || result.Cancelled {
		sh.dismiss()
	}
}

type integrationAccountNameSheet struct {
	w                                *appWindow
	serviceID, serviceName, runnerID string
	runner                           model.Device
	name, errorText                  string
	saving, closed                   bool
}

func (w *appWindow) presentIntegrationAccountName(serviceID, serviceName string, runner *model.Device) {
	s := &integrationAccountNameSheet{w: w, serviceID: serviceID, serviceName: serviceName, runnerID: runner.ID, runner: *runner}
	w.present(s.view, func() { s.closed = true })
}

func (s *integrationAccountNameSheet) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	result := sheetFrame(c, sheetOptions{Title: L("Add an account to %@", s.serviceName),
		Subtitle: L("Name this account so your bots can choose it explicitly."), Width: 460,
		Confirm: L("Add"), ConfirmDisabled: s.saving || strings.TrimSpace(s.name) == ""}, func() {
		textField(c.Key("integration-account-name"), &s.name, fieldOptions{
			Label: L("Account name"), Placeholder: L("Work or Personal"), AutoFocus: true}).MinHeight(28)
		if s.errorText != "" {
			providerNote(c, s.errorText, &p.Red)
		}
	})
	if result.Cancelled {
		sh.dismiss()
	}
	if result.Confirmed && !s.saving {
		s.saving, s.errorText = true, ""
		store.InstallPluginAccount(s.serviceID, s.runnerID, strings.TrimSpace(s.name), func(account model.InstalledPlugin, err error) {
			if s.closed {
				return
			}
			s.saving = false
			if err != nil {
				s.errorText = model.ErrorText(err)
				return
			}
			on := store.Device(s.runnerID)
			if on == nil {
				on = &s.runner
			}
			sh.dismiss()
			s.w.presentPlugin(account.ID, on)
		})
	}
}
