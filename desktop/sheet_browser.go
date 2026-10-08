package main

import (
	"slices"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Persistent field state belongs to this sheet. Elements and contexts stay in
// their build pass; Runner replies land through the model's ordered main queue.
type browserSheet struct {
	source                       *model.Store
	botID, runnerID, chatID      string
	botName, runnerName          string
	account, profile, selectedID string
	list                         model.BrowserSessionList
	ready, loading, closed       bool
	loads, epoch                 uint64
	pending                      model.BrowserOperation
	status, loadError            string
	timer                        *time.Timer
}

func (w *appWindow) presentBrowserSessions(botID, chatID string) *browserSheet {
	bot := store.Bot(botID)
	if bot == nil {
		return nil
	}
	runner := store.Device(bot.RunnerID)
	if runner == nil || !runner.IsRunner() {
		return nil
	}
	st := &browserSheet{source: store, botID: botID, runnerID: runner.ID, chatID: chatID, botName: bot.Name, runnerName: runner.Name, account: L("Default"), profile: L("Browser")}
	w.present(st.view, st.close)
	st.load()
	st.schedule()
	return st
}

func (s *browserSheet) close() {
	s.closed = true
	if s.timer != nil {
		s.timer.Stop()
		s.timer = nil
	}
}

func (s *browserSheet) currentOwner() bool {
	bot := s.source.Bot(s.botID)
	return bot != nil && bot.RunnerID == s.runnerID
}

func (s *browserSheet) selected() (model.BrowserSession, bool) {
	i := slices.IndexFunc(s.list.Sessions, func(each model.BrowserSession) bool { return each.ID == s.selectedID })
	if i < 0 {
		return model.BrowserSession{}, false
	}
	return s.list.Sessions[i], true
}

func (s *browserSheet) apply(list model.BrowserSessionList) bool {
	for _, session := range list.Sessions {
		if session.BotID != s.botID || session.RunnerID != s.runnerID {
			s.loadError = L("This browser session belongs to another bot or Runner.")
			s.ready = false
			return false
		}
	}
	s.list, s.ready, s.loadError = list, true, ""
	if _, found := s.selected(); !found {
		s.selectedID = ""
		for _, session := range list.Sessions {
			if session.Selected {
				s.selectedID = session.ID
				break
			}
		}
		if s.selectedID == "" && len(list.Sessions) > 0 {
			s.selectedID = list.Sessions[0].ID
		}
	}
	return true
}

func (s *browserSheet) load() {
	if s.closed || s.loading {
		return
	}
	if !s.currentOwner() {
		s.ready, s.loadError = false, L("This browser session belongs to another bot or Runner.")
		return
	}
	s.loading = true
	s.loads++
	load, epoch := s.loads, s.epoch
	s.source.BrowserSessions(s.botID, func(list model.BrowserSessionList, err error) {
		if s.closed || load != s.loads || epoch != s.epoch {
			return
		}
		s.loading = false
		if !s.currentOwner() {
			s.ready = false
			return
		}
		if err != nil {
			s.loadError = model.ErrorText(err)
			return
		}
		s.apply(list)
	})
}

func (s *browserSheet) schedule() {
	if s.closed || s.timer != nil || s.source.IsMock {
		return
	}
	s.timer = s.source.BrowserRefreshAfter(func() {
		s.timer = nil
		if s.closed {
			return
		}
		s.load()
		s.schedule()
	})
}

func (s *browserSheet) allowed(op model.BrowserOperation) bool {
	if s.closed || !s.ready || !s.currentOwner() || !s.source.IsConnected {
		return false
	}
	if op == model.BrowserCreate {
		return s.pending == "" && strings.TrimSpace(s.account) != "" && strings.TrimSpace(s.profile) != ""
	}
	session, exists := s.selected()
	if !exists {
		return false
	}
	if op == model.BrowserStop {
		return s.list.Capabilities.Stop && s.pending != model.BrowserStop && (session.State != model.BrowserStopped || s.pending == model.BrowserOpen)
	}
	if s.pending != "" {
		return false
	}
	active := session.State == model.BrowserBotControl || session.State == model.BrowserHuman
	switch op {
	case model.BrowserOpen:
		return s.list.Capabilities.VisibleOpen && (active || session.State == model.BrowserStopped)
	case model.BrowserTakeover:
		return s.list.Capabilities.Pause && session.State == model.BrowserBotControl
	case model.BrowserResume:
		return s.list.Capabilities.Resume && session.State == model.BrowserHuman
	case model.BrowserScreenshot:
		return s.list.Capabilities.Screenshot && active
	}
	return false
}

func (s *browserSheet) perform(op model.BrowserOperation) {
	if !s.allowed(op) {
		return
	}
	session, _ := s.selected()
	s.epoch++
	epoch := s.epoch
	// Invalidate a read begun before this action. A later Stop also invalidates
	// the pending takeover/open callback, so it cannot grant control in the UI.
	s.loads++
	s.loading = false
	s.pending = op
	s.status = L("Working…")
	if op == model.BrowserTakeover {
		s.status = L("Waiting for current input to finish…")
	}
	s.source.BrowserAction(op, s.botID, s.chatID, session, s.account, s.profile, func(reply model.BrowserReply, err error) {
		if s.closed || epoch != s.epoch {
			return
		}
		// A poll started while this mutation waited can describe the previous
		// control state. Invalidate it before applying the mutation's result.
		s.loads++
		s.loading = false
		s.pending = ""
		if !s.currentOwner() {
			s.ready = false
			s.status = L("This browser session belongs to another bot or Runner.")
			return
		}
		if err != nil {
			s.status = model.ErrorText(err)
			s.load()
			return
		}
		s.status = ""
		if op == model.BrowserScreenshot {
			s.status = L("Screenshot attached in chat.")
		}
		if reply.Session != nil {
			session := *reply.Session
			if session.BotID != s.botID || session.RunnerID != s.runnerID {
				s.ready, s.status = false, L("This browser session belongs to another bot or Runner.")
				return
			}
			if op == model.BrowserCreate {
				s.selectedID = session.ID
			}
			i := slices.IndexFunc(s.list.Sessions, func(each model.BrowserSession) bool { return each.ID == session.ID })
			if i < 0 {
				s.list.Sessions = append(s.list.Sessions, session)
			} else if s.list.Sessions[i].Revision <= session.Revision {
				s.list.Sessions[i] = session
			}
		}
		s.load()
	})
}

func (s *browserSheet) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	result := sheetFrame(c, sheetOptions{Title: L("Browser Sessions"), Subtitle: L("%@ owns these profiles on %@. Sign in in the visible browser, then explicitly return control to the bot.", s.botName, s.runnerName), Width: 570, Confirm: L("Done"), NoCancel: true}, func() {
		for _, field := range []struct {
			key, label string
			value      *string
		}{{"account", L("Account"), &s.account}, {"profile", L("Profile"), &s.profile}} {
			ui.Row(c.Key(field.key)).Gap(12).AlignItems(ui.Center).Children(func() {
				ui.Text(c, field.label).Width(70).FontSize(12).TextColor(p.Label2)
				textField(c.Key("input"), field.value, fieldOptions{Label: field.label, Disabled: s.pending != ""}).Grow(1)
			})
		}
		if pushButton(c.Key("create"), L("Create Profile"), pushOptions{Disabled: !s.allowed(model.BrowserCreate)}).Clicked() {
			s.perform(model.BrowserCreate)
		}
		ui.Text(c, L("Each profile has separate browser data. For stronger separation, assign the bot to a dedicated Runner.")).FontSize(textCaption).TextColor(p.Label2).LineHeight(1.4)
		if len(s.list.Sessions) > 0 {
			options := make([]popUpOption, 0, len(s.list.Sessions))
			for _, session := range s.list.Sessions {
				label := session.Account + " · " + session.Profile
				if session.Selected {
					label += " · " + L("Selected")
				}
				options = append(options, popUpOption{Value: session.ID, Label: label})
			}
			if picked, changed, _ := popUpButton(c.Key("session"), popUp{Options: options, Value: s.selectedID, Width: 510, Label: L("Browser Sessions"), Disabled: s.pending != ""}); changed {
				s.selectedID = picked
			}
		}
		session, exists := s.selected()
		if !exists {
			ui.Text(c, L("Create a profile to start a persistent browser session.")).FontSize(textCaption).TextColor(p.Label2)
		} else {
			state := L("Unknown")
			switch session.State {
			case model.BrowserStopped:
				state = L("Stopped")
			case model.BrowserBotControl:
				state = L("Bot control")
			case model.BrowserTakingOver:
				state = L("Waiting for current input to finish…")
			case model.BrowserHuman:
				state = L("Human control on %@", s.runnerName)
			}
			ui.Text(c, state).FontSize(12).TextColor(p.Label2)
			ui.Text(c, session.ID).Font(monoFont).FontSize(10).TextColor(p.Label3).Selectable()
		}
		ui.Row(c).Gap(8).Children(func() {
			if pushButton(c.Key("open"), L("Open Browser"), pushOptions{Disabled: !s.allowed(model.BrowserOpen)}).Clicked() {
				s.perform(model.BrowserOpen)
			}
			title, action := L("Take Over"), model.BrowserTakeover
			if !s.list.Capabilities.LocalInput {
				title = L("Pause on Runner")
			}
			if session.State == model.BrowserHuman {
				title, action = L("Return to Bot"), model.BrowserResume
			}
			if pushButton(c.Key("control"), title, pushOptions{Disabled: !s.allowed(action)}).Clicked() {
				s.perform(action)
			}
			if pushButton(c.Key("stop"), L("Stop Browser"), pushOptions{Disabled: !s.allowed(model.BrowserStop)}).Clicked() {
				s.perform(model.BrowserStop)
			}
		})
		if pushButton(c.Key("screenshot"), L("Attach Screenshot"), pushOptions{Disabled: !s.allowed(model.BrowserScreenshot)}).Clicked() {
			s.perform(model.BrowserScreenshot)
		}
		note := L("This Device can pause, return control, stop, and attach screenshots. Sign-in and interactive input happen on %@; live remote viewing and input are unavailable.", s.runnerName)
		if !s.ready {
			note = L("Loading…")
		} else if s.list.Capabilities.LocalInput {
			note = L("Use the browser on this Runner while you have control. Bot tool calls wait until Return to Bot. Screenshots attach encrypted evidence to this chat.")
		}
		ui.Text(c, note).FontSize(textCaption).TextColor(p.Label2).LineHeight(1.4)
		if s.loadError != "" {
			ui.Text(c, s.loadError).FontSize(textCaption).TextColor(p.Orange).LineHeight(1.4)
		}
		if s.status != "" {
			ui.Text(c, s.status).FontSize(textCaption).TextColor(p.Label2).LineHeight(1.4)
		}
	})
	if result.Confirmed || result.Cancelled {
		sh.dismiss()
	}
}
