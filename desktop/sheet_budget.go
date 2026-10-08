package main

import (
	"encoding/json"
	"fmt"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

type budgetChoice struct {
	target model.BudgetTarget
	label  string
}

// presentTaskBudget is the canonical task-card integration hook. The task module supplies
// its existing ID/owner; this sheet owns no task records or execution claims.
func (w *appWindow) presentTaskBudget(bot *model.Bot, chatID, taskID string) {
	w.presentBudget(bot, chatID, "", taskID)
}

func (w *appWindow) presentBudget(bot *model.Bot, chatID, routineID, taskID string) {
	if bot == nil {
		return
	}
	s := &budgetSheet{w: w, botName: bot.Name, drafts: map[string]model.BudgetFields{}}
	target := model.BudgetTarget{RunnerID: bot.RunnerID, BotID: bot.ID, ChatID: chatID}
	switch {
	case routineID != "":
		target.Kind, target.ID = "routine", routineID
		name := L("Routine")
		if routine := store.Routine(routineID); routine != nil {
			name = routine.Name
		}
		s.choices = []budgetChoice{{target, name}}
	case taskID != "":
		target.Kind, target.ID = "task", taskID
		s.choices = []budgetChoice{{target, L("Task allowance")}}
	default:
		target.Kind, target.ID = "chat", chatID
		s.choices = []budgetChoice{{target, L("Allowance for new tasks in this chat")}}
		for _, budget := range store.BudgetsFor(chatID, bot.RunnerID) {
			if budget.Kind != "job" && budget.Kind != "task" {
				continue
			}
			item := target
			item.Kind, item.ID = budget.Kind, budget.ID
			id := budget.ID
			if len(id) > 8 {
				id = id[len(id)-8:]
			}
			s.choices = append(s.choices, budgetChoice{item, budget.StateLabel() + " · " + id})
			if s.selected == "" && budget.NeedsRecovery() {
				s.selected = budget.Key()
			}
			if len(s.choices) >= 13 {
				break
			}
		}
	}
	if s.selected == "" {
		s.selected = s.choices[0].target.Kind + ":" + s.choices[0].target.ID
	}
	s.load()
	w.present(s.view, func() { s.closed = true })
}

// All persistent sheet state is data. MyGo contexts/elements remain in the build pass.
type budgetSheet struct {
	w                                 *appWindow
	botName                           string
	choices                           []budgetChoice
	selected                          string
	fields                            model.BudgetFields
	loaded                            *model.BudgetState
	loading, readable, saving, closed bool
	loads                             int
	errorText, recoveryNote           string
	requestID, attempt                string
	drafts                            map[string]model.BudgetFields
}

func (s *budgetSheet) target() model.BudgetTarget {
	for _, item := range s.choices {
		if item.target.Kind+":"+item.target.ID == s.selected {
			return item.target
		}
	}
	return s.choices[0].target
}

func (s *budgetSheet) current() *model.BudgetState {
	if s.loaded == nil {
		return nil
	}
	target := s.target()
	if live := store.Budget(target.Kind, target.ID, target.RunnerID); live != nil && live.UpdatedAt > s.loaded.UpdatedAt {
		return live
	}
	return s.loaded
}

func (s *budgetSheet) selectScope(value string) {
	if s.saving || value == s.selected {
		return
	}
	if s.readable {
		s.drafts[s.selected] = s.fields
	}
	s.selected = value
	s.fields = model.BudgetFields{}
	s.loaded = nil
	s.requestID, s.attempt, s.recoveryNote = "", "", ""
	s.load()
}

func (s *budgetSheet) load() {
	s.loads++
	load := s.loads
	target := s.target()
	s.loading, s.readable, s.errorText = true, false, ""
	store.ListBudgets(target.RunnerID, func(values []model.BudgetState, err error) {
		if s.closed || load != s.loads {
			return
		}
		s.loading = false
		if err != nil {
			s.errorText = model.ErrorText(err)
			return
		}
		s.loaded = nil
		for _, budget := range values {
			if budget.Kind == target.Kind && budget.ID == target.ID && budget.RunnerID == target.RunnerID {
				value := budget
				s.loaded = &value
				break
			}
		}
		limits := model.BudgetLimits{}
		if s.loaded != nil {
			limits = s.loaded.Limits
		}
		s.fields = model.BudgetFieldsFor(limits)
		if draft, ok := s.drafts[s.selected]; ok {
			s.fields = draft
		}
		s.readable = true
	})
}

func (s *budgetSheet) change(sh *sheet, resume, renew bool) {
	if s.loading || !s.readable || s.saving {
		return
	}
	limits, err := s.fields.Limits()
	if err != nil {
		s.errorText = model.ErrorText(err)
		return
	}
	target := s.target()
	owned := target.Kind == "task"
	if current := s.current(); current != nil {
		owned = owned || current.OwnedAdmission()
	}
	encoded, _ := json.Marshal(limits)
	attempt := s.selected + string(encoded) + fmt.Sprint(resume, renew, owned)
	if s.requestID == "" || s.attempt != attempt {
		s.requestID, s.attempt = model.NewBudgetRequestID(), attempt
	}
	s.saving, s.errorText, s.recoveryNote = true, "", ""
	store.ChangeBudget(target, limits, resume, renew, owned, s.requestID, func(value model.BudgetState, err error) {
		if s.closed {
			return
		}
		s.saving = false
		if err != nil {
			s.errorText = model.ErrorText(err)
			return
		}
		s.loaded = &value
		s.requestID, s.attempt = "", ""
		if resume && owned {
			s.recoveryNote = L("Allowance recovered. Retry the delivery in Events or run the task again in Tasks so its ownership and inbox admission are checked.")
			return
		}
		sh.dismiss()
	})
}

func (s *budgetSheet) confirmRenew(sh *sheet) {
	s.w.showAlert(alertOptions{Message: L("Renew this allowance?"),
		Informative: L("This grants the full configured allowance again and resumes from the existing transcript. Check completed effects before resuming interrupted work."),
		Buttons:     []alertButton{{Title: L("Renew and resume")}, {Title: L("Cancel")}},
	}, func(index int) {
		if index == 0 && !s.closed {
			s.change(sh, true, true)
		}
	})
}

func (s *budgetSheet) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	current := s.current()
	var nextScope string
	disabled := s.loading || !s.readable || s.saving
	canResume := !disabled && s.target().Kind != "chat" && (current == nil || current.State != "running")
	result := sheetFrame(c, sheetOptions{Title: L("Budget limits"),
		Subtitle: L("Limits are enforced on %@'s Runner. Leave a field empty for unlimited. Runtime includes checks, retries, review, and waiting.", s.botName),
		Width:    560, Confirm: L("Save"), ConfirmDisabled: disabled}, func() {
		options := make([]popUpOption, 0, len(s.choices))
		for _, item := range s.choices {
			options = append(options, popUpOption{Value: item.target.Kind + ":" + item.target.ID, Label: item.label})
		}
		if value, changed, _ := popUpButton(c.Key("budget-scope"), popUp{Options: options, Value: s.selected, Label: L("Allowance"), Disabled: s.saving}); changed {
			nextScope = value
		}
		section(c, L("Usage"), sectionCaption, nil, func(k *card) {
			if current == nil {
				state := L("No limits configured")
				if s.loading {
					state = L("Loading…")
				}
				keyValueRow(c, k, L("State"), state, false, &p.Label2)
				return
			}
			u := current.Usage
			tint := p.Label
			if current.NeedsRecovery() {
				tint = p.Orange
			}
			keyValueRow(c, k, L("State"), current.StateLabel(), false, &tint)
			keyValueRow(c, k, L("API spending"), fmt.Sprintf("$%.4f", u.APICostUSD), false, nil)
			keyValueRow(c, k, L("Subscription API-equivalent estimate"), fmt.Sprintf("$%.4f", u.SubscriptionEstimateUSD), false, nil)
			keyValueRow(c, k, L("Unknown pricing"), L("%d calls", u.UnknownPriceCalls), false, nil)
			keyValueRow(c, k, L("Tokens / runtime"), model.BudgetTokenSummary(u.Tokens)+fmt.Sprintf(" · %.0f s", u.RuntimeSecs), false, nil)
			keyValueRow(c, k, L("Retries / connector calls"), fmt.Sprintf("%d / %d", u.Retries, u.ConnectorCalls), false, nil)
		})
		section(c, L("Allowance"), sectionCaption, nil, func(k *card) {
			for _, field := range []struct {
				key, label string
				value      *string
			}{
				{"usd", L("Spending (USD)"), &s.fields.USD}, {"tokens", L("Total tokens"), &s.fields.Tokens},
				{"runtime", L("Runtime (seconds)"), &s.fields.Runtime}, {"retries", L("Retries"), &s.fields.Retries},
				{"calls", L("Connector calls"), &s.fields.ConnectorCalls},
			} {
				k.row(rowBox(c.Key(s.selected + "/" + field.key))).Children(func() {
					rowKey(c, field.label)
					ui.Spacer(c)
					textField(c.Key(s.selected+"/input/"+field.key), field.value, fieldOptions{Label: field.label, Placeholder: L("Unlimited"), Disabled: disabled}).Width(160).Shrink(0)
				})
			}
		})
		note := L("API spending and subscription API-equivalent estimates count toward the spending allowance. For unknown pricing, use a token or runtime allowance.")
		if current != nil {
			note = firstNonEmpty(current.Reason, L("Requests without reported usage use token and cost estimates. Unknown prices need a token or runtime allowance; they are never treated as free."))
		}
		providerNote(c, note, &p.Label2)
		ui.Row(c).Gap(8).Children(func() {
			if pushButton(c, L("Save and resume"), pushOptions{Disabled: !canResume}).Clicked() {
				s.change(sh, true, false)
			}
			if pushButton(c, L("Renew allowance and resume…"), pushOptions{Disabled: !canResume}).Clicked() {
				s.confirmRenew(sh)
			}
		})
		if s.loading || s.saving {
			providerStatusLine(c, &providerStatus{text: L("Loading…"), tone: model.ToneSecondary, spinning: true})
		}
		if s.errorText != "" {
			providerNote(c, s.errorText, &p.Red)
			if !s.readable && !s.loading && pushButton(c, L("Retry"), pushOptions{}).Clicked() {
				s.load()
			}
		}
		if s.recoveryNote != "" {
			providerNote(c, s.recoveryNote, &p.Green)
		}
	})
	// Build the existing bound fields before switching, so the last edit belongs
	// to its original scope. The menu choice causes MyGo to rebuild the new scope.
	if nextScope != "" {
		s.selectScope(nextScope)
	}
	if result.Cancelled {
		sh.dismiss()
	}
	if result.Confirmed {
		s.change(sh, false, false)
	}
}

func budgetStateForRoutine(routineID, runnerID string) *model.BudgetState {
	return store.Budget("routine", routineID, runnerID)
}

// A short recovery line for inspectors. Fields remain local to their editing sheet.
func budgetSummary(chatID, runnerID string) (string, bool) {
	for _, value := range store.BudgetsFor(chatID, runnerID) {
		if (value.Kind == "job" || value.Kind == "task") && value.NeedsRecovery() {
			return value.StateLabel(), true
		}
	}
	return L("Task limits"), false
}
