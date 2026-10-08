package main

import (
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// One routine's details, after the macOS app's RoutineViewController: the schedule and its next and
// last runs, the task, the check when it has one, and the actions on it. Run Now starts it on the
// bot's Runner; Pause and Resume flip the switch the inspector shows; Edit in Chat hands the bot the
// change to make, since the bot owns its routines; Delete asks first.

// presentRoutine is a routine's details; onEditInChat puts a text in the chat's composer.
func (w *appWindow) presentRoutine(routineID string, bot *model.Bot, onEditInChat func(text string)) {
	if bot == nil {
		return
	}
	title := L("Routine")
	if routine := store.Routine(routineID); routine != nil {
		title = routine.Name
	}
	draft := &routineSheetState{}
	if routine := store.Routine(routineID); routine != nil {
		draft.policy = routine.MissedPolicy()
	}
	w.present(func(c *ui.Context, s *sheet) { w.routineView(c, s, title, routineID, bot, onEditInChat, draft) }, func() { draft.closed = true })
}

// routineOf is the routine as the bot's list has it, with its running state; nil once it is gone.
func routineOf(botID, routineID string) *model.Routine {
	for _, routine := range store.RoutinesFor(botID) {
		if routine.ID == routineID {
			return &routine
		}
	}
	return nil
}

func (w *appWindow) routineView(c *ui.Context, s *sheet, title, routineID string, bot *model.Bot, onEditInChat func(text string), draft *routineSheetState) {
	p := colors(c)
	routine := routineOf(bot.ID, routineID)
	// The sheet closes when the routine is gone.
	if routine == nil {
		s.dismiss()
	}
	if routine != nil && !draft.saving {
		draft.policy = routine.MissedPolicy()
	}
	result := sheetFrame(c, sheetOptions{
		Title:    title,
		Subtitle: L("A task %@ runs on its own in this chat. %@ set it up and can change it: ask in chat.", bot.Name, bot.Name),
		Width:    520,
		Confirm:  L("Done"),
		NoCancel: true,
		Footer:   func() { w.routineActions(c, s, routineID, bot, onEditInChat) },
	}, func() {
		if routine == nil {
			return
		}
		state, tint := routine.StateText(), p.Label2
		if routine.IsRunning {
			tint = p.Accent
		} else if routine.State == "failed" || routine.State == "blocked" || routine.PausedReason == "authentication" {
			tint = p.Orange
		}

		section(c, L("Schedule"), sectionCaption, nil, func(k *card) {
			keyValueRow(c, k, L("State"), state, false, &tint)
			keyValueRow(c, k, L("Schedule"), routine.ScheduleText, false, nil).Tooltip(routine.Schedule)
			_, timezone := actionRow(c.Key("timezone"), k, L("Timezone"), actionRowOptions{Value: routine.SchedulingTimezone(), Action: L("Change…")})
			if timezone.Action {
				w.presentRoutineTimezone(routineID)
			}
			w.routinePolicyRow(c, k, routineID, draft)
			if draft.policy == "skip" {
				noteRow(c, k, L("Skip occurrences more than a minute late. The next occurrence keeps the chosen timezone."), nil)
			} else {
				noteRow(c, k, L("After an outage, run once with current data. Missed occurrences never queue a burst of runs."), nil)
			}
			if draft.failure != "" {
				noteRow(c, k, draft.failure, &p.Orange)
			}
			next, nextLabel := routine.NextSummary(), L("Next run")
			if routine.HasCheck {
				nextLabel = L("Next check")
			}
			keyValueRow(c, k, nextLabel, next, false, nil)
			keyValueRow(c, k, L("Last run"), routine.LastRunSummary(), false, nil)
		})
		w.routineHealth(c, routine, bot)
		section(c, L("Task"), sectionCaption, nil, func(k *card) {
			k.row(ui.Scroll(c).Height(96).Padding(8, 12).Children(func() {
				ui.Text(c, routine.Prompt).FontSize(12).LineHeight(1.4).Selectable()
			}))
		})
		if routine.HasCheck {
			section(c, L("Check"), sectionCaption, nil, func(k *card) {
				k.row(ui.ScrollBoth(c).Height(120).Padding(8, 12).Children(func() {
					ui.Text(c, routine.Check).Font(monoFont).FontSize(11).LineHeight(1.45).NoWrap().Selectable()
				}))
			})
		}
	})
	if result.Confirmed || result.Cancelled {
		s.dismiss()
	}
}

type routineSheetState struct {
	policy  string
	saving  bool
	failure string
	closed  bool
}

func (w *appWindow) routineActions(c *ui.Context, s *sheet, routineID string, bot *model.Bot, onEditInChat func(string)) {
	routine := routineOf(bot.ID, routineID)
	if routine == nil {
		return
	}

	ui.Row(c).Grow(1).Gap(8).Children(func() {
		tooltip := L("Runs on the bot's Runner now")
		if runner := store.Device(bot.RunnerID); runner != nil {
			tooltip = L("Runs on %@ now", runner.Name)
		}
		if pushButton(c, L("Run Now"), pushOptions{Disabled: !routine.CanRunNow(), Tooltip: tooltip}).Clicked() {
			store.RunRoutine(routineID)
		}
		toggle := L("Pause")
		if !routine.IsEnabled {
			toggle = L("Resume")
		}
		if pushButton(c, toggle, pushOptions{}).Clicked() {
			store.SetRoutineEnabled(routineID, !routine.IsEnabled)
		}
		if pushButton(c, L("Edit in Chat…"), pushOptions{}).Clicked() {
			if onEditInChat != nil {
				onEditInChat(L("Edit my routine \"%@\": ", routine.Name))
			}
			s.dismiss()
		}
		ui.Spacer(c)
		if pushButton(c, L("Delete…"), pushOptions{}).Clicked() {
			w.confirmDeleteRoutine(s, routineID)
		}
	})
}

// confirmDeleteRoutine asks before deleting a routine, which can't be undone.
func (w *appWindow) confirmDeleteRoutine(s *sheet, routineID string) {
	routine := store.Routine(routineID)
	if routine == nil {
		return
	}
	w.showAlert(alertOptions{
		Message:     L("Delete “%@”?", routine.Name),
		Informative: L("This deletes the routine and stops its future runs. This can't be undone."),
		Style:       alertWarning,
		Buttons:     []alertButton{{Title: L("Delete routine")}, {Title: L("Cancel")}},
	}, func(index int) {
		if index != 0 {
			return
		}
		store.DeleteRoutine(routineID)
		s.dismiss()
	})
}

func (w *appWindow) routinePolicyRow(c *ui.Context, k *card, id string, draft *routineSheetState) {
	choice := L("Run once")
	if draft.policy == "skip" {
		choice = L("Skip")
	}
	k.row(rowBox(c.Key("missed-runs"))).Children(func() {
		rowKey(c, L("Missed runs"))
		ui.Spacer(c)
		ui.Select(c.Key("policy-choice"), &choice, []string{L("Run once"), L("Skip")}).Width(150).Label(L("Missed runs")).Disabled(draft.saving).OnChange(func() {
			if choice == L("Skip") {
				draft.policy = "skip"
			} else {
				draft.policy = "coalesce"
			}
			policy := draft.policy
			draft.saving, draft.failure = true, ""
			store.SetRoutinePolicy(id, model.RoutinePolicy{MissedRunPolicy: &policy}, func(err error) {
				draft.saving = false
				if draft.closed {
					return
				}
				if err != nil {
					draft.failure = model.ErrorText(err)
					if current := store.Routine(id); current != nil {
						draft.policy = current.MissedPolicy()
					}
				}
			})
		})
	})
}

func (w *appWindow) presentRoutineTimezone(id string) {
	routine := store.Routine(id)
	if routine == nil {
		return
	}
	timezone := routine.SchedulingTimezone()
	busy, closed, failure := false, false, ""
	w.present(func(c *ui.Context, s *sheet) {
		result := sheetFrame(c, sheetOptions{Title: L("Routine timezone"), Subtitle: L("Use an IANA timezone such as America/New_York, Asia/Singapore, or UTC. Cron follows its daylight-saving changes; intervals count elapsed time."), Width: 520, Confirm: L("Save"), ConfirmDisabled: busy || strings.TrimSpace(timezone) == ""}, func() {
			textField(c.Key("routine-timezone"), &timezone, fieldOptions{AutoFocus: true, Label: L("Timezone"), Disabled: busy})
			if failure != "" {
				ui.Text(c, failure).FontSize(12).TextColor(colors(c).Orange)
			}
		})
		if result.Cancelled {
			s.dismiss()
		}
		if result.Confirmed && !busy {
			busy, failure = true, ""
			next := strings.TrimSpace(timezone)
			store.SetRoutinePolicy(id, model.RoutinePolicy{Timezone: &next}, func(err error) {
				busy = false
				if closed {
					return
				}
				if err != nil {
					failure = model.ErrorText(err)
				} else {
					s.dismiss()
				}
			})
		}
	}, func() { closed = true })
}

func (w *appWindow) routineHealth(c *ui.Context, routine *model.Routine, bot *model.Bot) {
	section(c.Key("routine-health"), L("Availability and checks"), sectionCaption, nil, func(k *card) {
		runner := store.Device(bot.RunnerID)
		name := bot.RunnerID
		if runner != nil {
			name = runner.Name
		}
		keyValueRow(c, k, L("Runner"), name, false, nil)
		availability := L("Available")
		if !routine.RunnerAvailable && routine.HasRunnerAvailability {
			availability = L("Waiting for Runner")
		}
		keyValueRow(c, k, L("Availability"), availability, false, nil)
		if routine.HasCheck {
			checked, success := L("Never"), L("Never")
			if !routine.LastCheckAt.IsZero() {
				checked = model.DaySeparator(routine.LastCheckAt)
			}
			if !routine.LastSuccessfulCheckAt.IsZero() {
				success = model.DaySeparator(routine.LastSuccessfulCheckAt)
			}
			keyValueRow(c, k, L("Last check"), checked, false, nil)
			keyValueRow(c, k, L("Last successful check"), success, false, nil)
		}
		if !routine.RetryAt.IsZero() && routine.IsEnabled {
			keyValueRow(c, k, L("Retry after"), model.DaySeparator(routine.RetryAt), false, nil)
		}
		if routine.RecoveryAction != "" {
			noteRow(c, k, routine.RecoveryAction, nil)
		}
	})
}
