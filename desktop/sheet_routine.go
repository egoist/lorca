package main

import (
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
	}, func() {
		if routine == nil {
			return
		}
		state, tint := L("Paused"), p.Label2
		switch {
		case routine.IsRunning:
			state, tint = L("Running…"), p.Accent
		case routine.IsEnabled:
			state, tint = L("On"), p.Green
		case routine.PausedReason == "away":
			state = L("Paused while you were away")
		}
		section(c, L("Schedule"), sectionCaption, nil, func(k *card) {
			keyValueRow(c, k, L("State"), state, false, &tint)
			keyValueRow(c, k, L("Schedule"), routine.ScheduleText, false, nil).Tooltip(routine.Schedule)
			next, nextLabel := "—", L("Next run")
			if routine.HasCheck {
				nextLabel = L("Next check")
			}
			if !routine.NextRunAt.IsZero() {
				next = model.Upcoming(routine.NextRunAt)
			}
			keyValueRow(c, k, nextLabel, next, false, nil)
			keyValueRow(c, k, L("Last run"), routine.LastRunSummary(), false, nil)
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
		ui.Row(c).Gap(8).Children(func() {
			tooltip := L("Runs on the bot's Runner now")
			if runner := store.Device(bot.RunnerID); runner != nil {
				tooltip = L("Runs on %@ now", runner.Name)
			}
			if pushButton(c, L("Run Now"), pushOptions{Disabled: routine.IsRunning, Tooltip: tooltip}).Clicked() {
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
	})
	if result.Confirmed || result.Cancelled {
		s.dismiss()
	}
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
