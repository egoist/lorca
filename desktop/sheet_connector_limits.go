package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
	"time"
)

type connectorLimitSheet struct {
	w                                     *appWindow
	pluginID, runnerID, runnerName, scope string
	fields                                model.ConnectorFields
	state                                 model.ConnectorState
	loading, readable, saving, closed     bool
	loads                                 int
	errorText                             string
	drafts                                map[string]model.ConnectorFields
}

func (w *appWindow) presentConnectorLimits(pluginID string, runner *model.Device) {
	if runner == nil {
		return
	}
	s := &connectorLimitSheet{w: w, pluginID: pluginID, runnerID: runner.ID, runnerName: runner.Name, scope: "account", drafts: map[string]model.ConnectorFields{}}
	s.load()
	w.present(s.view, func() { s.closed = true })
}

func (s *connectorLimitSheet) selectScope(scope string) {
	if s.saving || scope == s.scope {
		return
	}
	if s.readable {
		s.drafts[s.scope] = s.fields
	}
	s.scope = scope
	s.fields = model.ConnectorFields{}
	s.load()
}
func (s *connectorLimitSheet) load() {
	s.loads++
	generation := s.loads
	scope := s.scope
	s.loading, s.readable, s.errorText = true, false, ""
	store.GetConnectorLimits(s.pluginID, s.runnerID, scope, func(value model.ConnectorState, err error) {
		if s.closed || generation != s.loads {
			return
		}
		s.loading = false
		if err != nil {
			s.errorText = model.ErrorText(err)
			return
		}
		s.state = value
		s.readable = true
		s.fields = model.ConnectorFieldsFor(value.Limits)
		if draft, ok := s.drafts[scope]; ok {
			s.fields = draft
		}
	})
}
func (s *connectorLimitSheet) save(sh *sheet) {
	if !s.readable || s.loading || s.saving {
		return
	}
	limits, err := s.fields.Limits()
	if err != nil {
		s.errorText = model.ErrorText(err)
		return
	}
	s.saving, s.errorText = true, ""
	store.SetConnectorLimits(s.pluginID, s.runnerID, s.scope, limits, func(_ model.ConnectorState, err error) {
		if s.closed {
			return
		}
		s.saving = false
		if err != nil {
			s.errorText = model.ErrorText(err)
			return
		}
		sh.dismiss()
	})
}
func (s *connectorLimitSheet) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	disabled := s.loading || !s.readable || s.saving
	var nextScope string
	result := sheetFrame(c, sheetOptions{Title: L("Shared connector limits"),
		Subtitle: L("All bots on %@ share these call rates and concurrency limits. Service limits apply across its accounts. Service retry guidance still applies.", s.runnerName),
		Width:    520, Confirm: L("Save"), ConfirmDisabled: disabled}, func() {
		if scope, changed, _ := popUpButton(c.Key("connector-scope"), popUp{Value: s.scope, Options: []popUpOption{{Value: "account", Label: L("This account")}, {Value: "service", Label: L("All accounts for this service")}}, Label: L("Account and service"), Disabled: s.saving}); changed {
			nextScope = scope
		}
		section(c, L("Call limits"), sectionCaption, nil, func(k *card) {
			for _, field := range []struct {
				key, label string
				value      *string
			}{{"rate", L("Calls per window"), &s.fields.Calls}, {"window", L("Window (seconds)"), &s.fields.Window}, {"concurrency", L("Concurrent calls"), &s.fields.Concurrency}} {
				k.row(rowBox(c.Key(s.scope + "/" + field.key))).Children(func() {
					rowKey(c, field.label)
					ui.Spacer(c)
					textField(c.Key(s.scope+"/input/"+field.key), field.value, fieldOptions{Label: field.label, Disabled: disabled}).Width(130).Shrink(0)
				})
			}
		})
		if s.loading || s.saving {
			providerStatusLine(c, &providerStatus{text: L("Loading…"), tone: model.ToneSecondary, spinning: true})
		} else if s.readable {
			text := L("%d calls active. Zero call or concurrency capacity pauses calls.", s.state.ActiveCalls)
			if s.state.RetryAt != nil {
				text = L("Service cooldown until %@", model.Upcoming(time.UnixMilli(int64(*s.state.RetryAt*1000))))
			}
			providerNote(c, text, &p.Label2)
		}
		if s.errorText != "" {
			providerNote(c, s.errorText, &p.Red)
			if !s.readable && !s.loading && pushButton(c, L("Retry"), pushOptions{}).Clicked() {
				s.load()
			}
		}
	})
	// Keep pending bound edits in the scope whose controls built this frame.
	if nextScope != "" {
		s.selectScope(nextScope)
	}
	if result.Cancelled {
		sh.dismiss()
	}
	if result.Confirmed {
		s.save(sh)
	}
}
