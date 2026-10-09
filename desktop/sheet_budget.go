package main

import (
	"errors"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// budgetSheet is what a DM's turns, a task's runs, or a routine's runs may use on the bot's
// Runner, after the macOS app's BudgetViewController: a form of limits with what the work used
// beside each, and a card with Resume when the work stopped. The fields are the user's until the
// sheet closes.
type budgetSheet struct {
	w         *appWindow
	bot       *model.Bot
	chatID    string
	routineID string
	taskID    string
	fields    model.BudgetFields
	// invalid is the field that isn't a number.
	invalid   string
	errorText string
	busy      bool
	closed    bool
}

// presentBudget opens a DM's limits for each new turn, with its newest turn when that one
// stopped; with a routine, the routine's limits, which all of its runs count toward.
func (w *appWindow) presentBudget(bot *model.Bot, chatID, routineID string) {
	if bot == nil {
		return
	}
	s := &budgetSheet{w: w, bot: bot, chatID: chatID, routineID: routineID}
	s.open()
}

// presentTaskLimits opens a task's limits, which all of its runs count toward; `bot` is its owner.
func (w *appWindow) presentTaskLimits(bot *model.Bot, task *model.DurableTask) {
	if bot == nil || task == nil {
		return
	}
	s := &budgetSheet{w: w, bot: bot, taskID: task.ID}
	if len(task.ChatIDs) > 0 {
		s.chatID = task.ChatIDs[0]
	}
	s.open()
}

func (s *budgetSheet) open() {
	if configured := s.configured(); configured != nil {
		s.fields = model.BudgetFieldsFor(configured.Limits)
	}
	s.w.present(s.view, func() { s.closed = true })
}

// configured is the allowance the form edits: the task's or routine's, or the DM's for new turns.
func (s *budgetSheet) configured() *model.BudgetState {
	scope := s.scope()
	return store.Budget(scope.Kind, scope.ID, s.bot.RunnerID)
}

// stopped is what stopped and waits for Resume: the task or routine, or the DM's newest turn.
func (s *budgetSheet) stopped() *model.BudgetState {
	if s.scope().Kind != "chat" {
		if b := s.configured(); b != nil && b.IsStopped() {
			return b
		}
		return nil
	}
	return store.StoppedTurn(s.chatID, s.bot.RunnerID)
}

// usage is whose use the form shows beside the limits.
func (s *budgetSheet) usage() *model.BudgetUsage {
	b := s.stopped()
	if s.scope().Kind != "chat" {
		b = s.configured()
	}
	if b == nil {
		return nil
	}
	return &b.Usage
}

func (s *budgetSheet) limits() (model.BudgetLimits, bool) {
	limits, err := s.fields.Limits()
	if err != nil {
		var field model.BudgetFieldError
		if errors.As(err, &field) {
			s.invalid = field.Field
		}
		s.errorText = model.ErrorText(err)
		return limits, false
	}
	s.invalid = ""
	return limits, true
}

func (s *budgetSheet) scope() model.BudgetTarget {
	if s.taskID != "" {
		return model.BudgetTarget{Kind: "task", ID: s.taskID}
	}
	if s.routineID != "" {
		return model.BudgetTarget{Kind: "routine", ID: s.routineID}
	}
	return model.BudgetTarget{Kind: "chat", ID: s.chatID}
}

// change sends a change to the Runner with the buttons off, and closes the sheet once it's done.
func (s *budgetSheet) change(sh *sheet, change model.BudgetChange) {
	s.busy, s.errorText = true, ""
	store.ChangeBudget(s.bot, s.chatID, change, func(err error) {
		if s.closed {
			return
		}
		s.busy = false
		if err != nil {
			s.errorText = model.ErrorText(err)
			return
		}
		sh.dismiss()
	})
}

// resume goes on where the work stopped. The stopped turn takes the limits in the form too, so
// raising one lets it go on. When the limit it reached didn't go up, resuming means using the
// limits in full again, so that asks first.
func (s *budgetSheet) resume(sh *sheet, stopped model.BudgetState) {
	limits, ok := s.limits()
	if !ok {
		return
	}
	change := model.BudgetChange{Saves: []model.BudgetTarget{s.scope()}, Limits: limits, Resume: &model.BudgetTarget{Kind: stopped.Kind, ID: stopped.ID}}
	if stopped.Kind == "job" {
		change.Saves = append(change.Saves, model.BudgetTarget{Kind: "job", ID: stopped.ID})
	}
	if stopped.Fits(limits) {
		s.change(sh, change)
		return
	}
	message := L("Resume this turn with fresh limits?")
	switch s.scope().Kind {
	case "routine":
		message = L("Resume this routine with fresh limits?")
	case "task":
		message = L("Resume this task with fresh limits?")
	}
	s.w.showAlert(alertOptions{Message: message,
		Informative: L("It already used its limits. Resuming lets it use them again in full."),
		Buttons:     []alertButton{{Title: L("Resume")}, {Title: L("Cancel")}},
	}, func(index int) {
		if index == 0 && !s.closed {
			change.Fresh = true
			s.change(sh, change)
		}
	})
}

func (s *budgetSheet) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	subtitle := L("Each turn with %@ stops when it reaches one of these.", s.bot.Name)
	if s.taskID != "" {
		subtitle = L("All runs of this task count toward these. When it reaches one, the task waits until you resume it.")
	} else if s.routineID != "" {
		name := L("Routine")
		if routine := store.Routine(s.routineID); routine != nil {
			name = routine.Name
		}
		subtitle = L("All runs of %@ count toward these. When it reaches one, it waits until you resume it.", name)
	}
	stopped := s.stopped()
	usage := s.usage()
	result := sheetFrame(c, sheetOptions{Title: L("Limits"), Subtitle: subtitle, Width: 460, Confirm: L("Save"), ConfirmDisabled: s.busy}, func() {
		if stopped != nil {
			ui.Row(c.Key("stopped")).Gap(10).Padding(10, 12).AlignItems(ui.Center).Background(p.BotBubble).Radius(9).Border(1, p.BotBubbleBorder).Children(func() {
				ui.Row(c).TextColor(p.Orange).Children(func() { symbol(c, "exclamationmark.circle.fill", 17, 2) })
				ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(2).Children(func() {
					ui.Text(c, stopped.StoppedTitle()).FontSize(13).FontWeight(600)
					ui.Text(c, stopped.StoppedDetail()).FontSize(12).TextColor(p.Label2).LineHeight(1.35)
				})
				if pushButton(c, L("Resume"), pushOptions{Disabled: s.busy}).Clicked() {
					s.resume(sh, *stopped)
				}
			})
		}
		ui.Column(c.Key("limits")).Gap(8).Children(func() {
			for _, field := range []struct {
				key, label, unit string
				value            *string
			}{
				{"usd", L("Spending"), "USD", &s.fields.USD},
				{"tokens", L("Tokens"), "", &s.fields.Tokens},
				{"runtime", L("Run time"), L("minutes"), &s.fields.Minutes},
				{"retries", L("Retries"), "", &s.fields.Retries},
				{"connector_calls", L("Plugin calls"), "", &s.fields.ConnectorCalls},
			} {
				ui.Row(c.Key(field.key)).Gap(10).AlignItems(ui.Center).Children(func() {
					ui.Text(c, field.label).Width(formLabelWidth).FontSize(12).TextColor(p.Label2).SingleLine()
					input := textField(c.Key("field"), field.value, fieldOptions{Label: field.label, Placeholder: L("No limit"), Disabled: s.busy}).Width(100).Shrink(0).TextAlign(ui.End)
					if s.invalid == field.key && input.Changed() {
						s.invalid, s.errorText = "", ""
					}
					ui.Text(c, field.unit).FontSize(12).TextColor(p.Label2).SingleLine()
					used, tint := "", p.Label2
					if usage != nil {
						used = budgetUsed(field.key, *usage)
					}
					if stopped != nil && stopped.Reached == field.key {
						tint = p.Orange
					}
					ui.Text(c, used).Grow(1).MinWidth(0).TextAlign(ui.End).FontSize(12).TextColor(tint).SingleLine()
				})
			}
		})
		if s.errorText != "" {
			ui.Text(c.Key("error"), s.errorText).FontSize(12).TextColor(p.Red).LineHeight(1.35)
		}
	})
	if result.Cancelled {
		sh.dismiss()
	}
	if result.Confirmed && !s.busy {
		if limits, ok := s.limits(); ok {
			s.change(sh, model.BudgetChange{Saves: []model.BudgetTarget{s.scope()}, Limits: limits})
		}
	}
}

// budgetUsed is what the work used of one limit: "$0.21 used", "100,412 used", or nothing.
func budgetUsed(field string, u model.BudgetUsage) string {
	switch field {
	case "usd":
		if u.APICostUSD > 0 || u.SubscriptionEstimateUSD > 0 {
			return L("%@ used", model.Spend(u.APICostUSD, u.SubscriptionEstimateUSD))
		}
		if u.UnknownPriceCalls > 0 {
			return L("Price unknown")
		}
	case "tokens":
		if u.Tokens > 0 {
			return L("%@ used", model.Count(u.Tokens))
		}
	case "runtime":
		if u.RuntimeSecs >= 1 {
			return L("%@ used", model.Minutes(int(u.RuntimeSecs)))
		}
	case "retries":
		if u.Retries > 0 {
			return L("%@ used", model.Count(u.Retries))
		}
	case "connector_calls":
		if u.ConnectorCalls > 0 {
			return L("%@ used", model.Count(u.ConnectorCalls))
		}
	}
	return ""
}
