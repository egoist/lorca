package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// callLimitSheet is how often, and how many at once, all bots on a Runner may call one plugin
// account, after the macOS app's ConnectorLimitsViewController. An account of a service with
// several accounts can also set the limit they share.
type callLimitSheet struct {
	w         *appWindow
	pluginID  string
	name      string
	runner    *model.Device
	service   bool
	fields    model.CallLimitFields
	limits    *model.CallLimits
	errorText string
	loads     int
	busy      bool
	closed    bool
	onSaved   func()
}

func (w *appWindow) presentCallLimit(pluginID, name string, runner *model.Device, onSaved func()) {
	if runner == nil {
		return
	}
	s := &callLimitSheet{w: w, pluginID: pluginID, name: name, runner: runner, onSaved: onSaved}
	s.load()
	w.present(s.view, func() { s.closed = true })
}

// load reads the selected limit from the Runner; Save waits for it. Only the newest answer
// counts, so a late one for the other scope never fills the fields.
func (s *callLimitSheet) load() {
	s.loads++
	load := s.loads
	s.limits, s.errorText = nil, ""
	store.CallLimits(s.pluginID, s.runner.ID, s.service, func(limits model.CallLimits, err error) {
		if s.closed || load != s.loads {
			return
		}
		if err != nil {
			s.errorText = model.ErrorText(err)
			return
		}
		s.limits, s.fields = &limits, model.CallLimitFieldsFor(limits)
	})
}

func (s *callLimitSheet) save(sh *sheet) {
	s.busy, s.errorText = true, ""
	store.SetCallLimits(s.pluginID, s.runner.ID, s.service, s.fields, func(err error) {
		if s.closed {
			return
		}
		s.busy = false
		if err != nil {
			s.errorText = model.ErrorText(err)
			return
		}
		if s.onSaved != nil {
			s.onSaved()
		}
		sh.dismiss()
	})
}

func (s *callLimitSheet) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	disabled := s.busy || s.limits == nil
	result := sheetFrame(c, sheetOptions{Title: L("Call Limit"), Subtitle: L("All bots on %@ share this limit when they use %@.", s.runner.Name, s.name),
		Confirm: L("Save"), ConfirmDisabled: disabled}, func() {
		label := func(text string) ui.Element {
			return ui.Text(c, text).FontSize(12).TextColor(p.Label2).SingleLine()
		}
		ui.Column(c.Key("form")).Gap(8).Children(func() {
			if s.limits != nil && s.limits.SharesService() {
				formRow(c.Key("scope"), L("Applies to"), false, func() {
					current := "account"
					if s.service {
						current = "service"
					}
					options := []popUpOption{{Value: "account", Label: L("This account")}, {Value: "service", Label: L("All %@ accounts", s.name)}}
					if value, changed, _ := popUpButton(c, popUp{Options: options, Value: current, Label: L("Applies to"), Disabled: s.busy}); changed {
						s.service = value == "service"
						s.load()
					}
				})
			}
			formRow(c.Key("calls"), L("Calls"), false, func() {
				ui.Row(c).Gap(6).AlignItems(ui.Center).Children(func() {
					textField(c.Key("calls-field"), &s.fields.Calls, fieldOptions{Label: L("Calls"), Disabled: disabled}).Width(64).TextAlign(ui.End)
					label(Lc("every", "calls every n seconds"))
					textField(c.Key("window-field"), &s.fields.Window, fieldOptions{Label: L("Seconds"), Disabled: disabled}).Width(64).TextAlign(ui.End)
					label(L("seconds"))
				})
			})
			formRow(c.Key("concurrency"), L("At once"), false, func() {
				textField(c.Key("concurrency-field"), &s.fields.Concurrency, fieldOptions{Label: L("At once"), Disabled: disabled}).Width(64).TextAlign(ui.End)
			})
		})
		if s.limits != nil {
			if until, waiting := s.limits.Waiting(); waiting {
				ui.Text(c.Key("waiting"), L("%@ asked Lorca to slow down. Calls wait until %@.", s.name, model.Clock(until))).FontSize(12).TextColor(p.Label2).LineHeight(1.35)
			}
		}
		if s.errorText != "" {
			ui.Text(c.Key("error"), s.errorText).FontSize(12).TextColor(p.Red).LineHeight(1.35)
		}
	})
	if result.Cancelled {
		sh.dismiss()
	}
	if result.Confirmed && !disabled {
		s.save(sh)
	}
}
