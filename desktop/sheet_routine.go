package main

import (
	"strings"
	"unicode"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// One routine's details, after the macOS app's RoutineViewController: how it stands, with what
// happened and how to fix it when something went wrong; the schedule, its next run, what happens
// to runs its Runner missed, and its last check and run; the task, the check when it has one, and
// the actions on it. Run Now starts it on the bot's Runner; Pause and Resume flip the switch the
// inspector shows; Edit in Chat hands the bot the change to make, since the bot owns its routines
// (its timezone and missed-run policy too); Delete asks first.

// presentRoutine is a routine's details; onEditInChat puts a text in the chat's composer.
func (w *appWindow) presentRoutine(routineID string, bot *model.Bot, onEditInChat func(text string)) {
	if bot == nil {
		return
	}
	title := L("Routine")
	if routine := store.Routine(routineID); routine != nil {
		title = routine.Name
	}
	w.present(func(c *ui.Context, s *sheet) { w.routineView(c, s, title, routineID, bot, onEditInChat) }, nil)
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

func (w *appWindow) routineView(c *ui.Context, s *sheet, title, routineID string, bot *model.Bot, onEditInChat func(text string)) {
	p := colors(c)
	routine := routineOf(bot.ID, routineID)
	// The sheet closes when the routine is gone.
	if routine == nil {
		s.dismiss()
	}
	result := sheetFrame(c, sheetOptions{
		Title:    title,
		Subtitle: L("A task %@ runs on its own in this chat. %@ set it up and can change it: ask in chat.", bot.Name, bot.Name),
		Width:    520,
		Confirm:  L("Done"),
		NoCancel: true,
		Footer: func() {
			if routine != nil {
				w.routineActions(c, s, routine, bot, onEditInChat)
			}
		},
	}, func() {
		if routine == nil {
			return
		}
		problem := routine.Problem()
		state, tint := L("Paused"), p.Label2
		switch {
		case routine.IsRunning:
			state, tint = L("Running…"), p.Accent
		case problem != model.ProblemNone:
			state = problem.Text()
			if problem.NeedsUser() {
				tint = p.Orange
			}
		case routine.IsEnabled:
			state, tint = L("On"), p.Green
		case routine.PausedReason == "away":
			state = L("Paused while you were away")
		}
		runner := L("its Runner")
		if device := store.Device(bot.RunnerID); device != nil {
			runner = device.Name
		}
		section(c, L("Schedule"), sectionCaption, nil, func(k *card) {
			keyValueRow(c, k, L("State"), state, false, &tint)
			if problem != model.ProblemNone {
				noteRow(c.Key("problem"), k, problem.Explanation(bot.Name, runner), nil)
			}
			scheduleTooltip := routine.Schedule
			if !strings.HasPrefix(routine.Schedule, "every ") {
				scheduleTooltip += " · " + routine.Timezone
			}
			keyValueRow(c.Key("schedule"), k, L("Schedule"), routine.ScheduleSummary(), false, nil).Tooltip(scheduleTooltip)
			next, nextLabel := "—", L("Next run")
			if routine.HasCheck {
				nextLabel = L("Next check")
			}
			if !routine.NextRunAt.IsZero() {
				// "Tomorrow 9:00 AM" on a line of its own, as the last check and run read.
				runes := []rune(model.Upcoming(routine.NextRunAt))
				runes[0] = unicode.ToUpper(runes[0])
				next = string(runes)
			}
			keyValueRow(c.Key("next"), k, nextLabel, next, false, nil)
			missed, missedTooltip := L("Run once"), L("When %@ was off at a scheduled time, the routine runs once when it’s back.", runner)
			if routine.MissedRunPolicy == "skip" {
				missed, missedTooltip = L("Skip"), L("When %@ was off at a scheduled time, the routine waits for the next one.", runner)
			}
			keyValueRow(c.Key("missed"), k, L("Missed runs"), missed, false, nil).Tooltip(missedTooltip)
			if lastCheck := routine.LastCheckSummary(); lastCheck != "" {
				keyValueRow(c.Key("last-check"), k, L("Last check"), lastCheck, false, nil)
				// Only a failing check has a success to tell apart from it.
				if routine.Health.Status == "failed" || routine.Health.Status == "blocked" {
					success := L("Never")
					if !routine.Health.LastSuccessAt.IsZero() {
						success = model.DaySeparator(routine.Health.LastSuccessAt)
					}
					keyValueRow(c.Key("last-success"), k, L("Last successful check"), success, false, nil)
				}
			}
			keyValueRow(c.Key("last-run"), k, L("Last run"), routine.LastRunSummary(), false, nil)
		})
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

// routineActions are the sheet's actions on the routine: Run Now, Pause or Resume, Edit in Chat,
// and Delete apart from them.
func (w *appWindow) routineActions(c *ui.Context, s *sheet, routine *model.Routine, bot *model.Bot, onEditInChat func(text string)) {
	ui.Row(c).Grow(1).Gap(8).Children(func() {
		tooltip := L("Runs on the bot's Runner now")
		if runner := store.Device(bot.RunnerID); runner != nil {
			tooltip = L("Runs on %@ now", runner.Name)
		}
		// A run needs its Runner online, and a routine paused by failed sign-ins needs Resume.
		blocked := routine.IsRunning || routine.State == "waiting_for_runner" || routine.PausedReason == "authentication"
		if pushButton(c, L("Run Now"), pushOptions{Disabled: blocked, Tooltip: tooltip}).Clicked() {
			store.RunRoutine(routine.ID)
		}
		toggle := L("Pause")
		if !routine.IsEnabled {
			toggle = L("Resume")
		}
		if pushButton(c, toggle, pushOptions{}).Clicked() {
			store.SetRoutineEnabled(routine.ID, !routine.IsEnabled)
		}
		if pushButton(c, L("Edit in Chat…"), pushOptions{}).Clicked() {
			if onEditInChat != nil {
				onEditInChat(L("Edit my routine \"%@\": ", routine.Name))
			}
			s.dismiss()
		}
		ui.Spacer(c)
		if pushButton(c, L("Delete…"), pushOptions{}).Clicked() {
			w.confirmDeleteRoutine(s, routine.ID)
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
