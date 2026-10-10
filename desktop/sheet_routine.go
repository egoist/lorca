package main

import (
	"cmp"
	"strings"
	"time"
	"unicode"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// One routine's details, after the macOS app's RoutineViewController: how it stands, with what
// happened and how to fix it when something went wrong; the schedule, its next run, what happens
// to runs its Runner missed, and its last check and run; the task, the check when it has one, and
// the actions on it. Run Now starts it on the bot's Runner; Pause and Resume flip the switch the
// inspector shows; Edit in Chat hands the bot the change to make, since the bot owns its routines
// (its timezone and missed-run policy too); Delete asks first. A routine on events shows what it
// listens to and its latest event instead of a next run; one with a webhook has its URL, key, and
// header to copy, and a new key.

// routineSheet is what the sheet keeps while it is up: whether the webhook key shows, and when
// each row last copied.
type routineSheet struct {
	keyShown bool
	copiedAt map[string]time.Time
}

// presentRoutine is a routine's details; onEditInChat puts a text in the chat's composer.
func (w *appWindow) presentRoutine(routineID string, bot *model.Bot, onEditInChat func(text string)) {
	if bot == nil {
		return
	}
	title := L("Routine")
	if routine := store.Routine(routineID); routine != nil {
		title = routine.Name
	}
	st := &routineSheet{copiedAt: map[string]time.Time{}}
	w.present(func(c *ui.Context, s *sheet) { w.routineView(c, s, st, title, routineID, bot, onEditInChat) }, nil)
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

func (w *appWindow) routineView(c *ui.Context, s *sheet, st *routineSheet, title, routineID string, bot *model.Bot, onEditInChat func(text string)) {
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
		// A routine stopped at its limits runs again only once the user resumes it in Limits.
		budget := store.Budget("routine", routine.ID, bot.RunnerID)
		stopped := budget != nil && budget.IsStopped()
		switch {
		case stopped:
			state, tint = budget.StoppedLabel(), p.Orange
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
			if stopped {
				noteRow(c.Key("problem"), k, budget.StoppedDetail(), nil)
			} else if problem != model.ProblemNone {
				subject := ""
				if routine.Events != nil {
					subject = routine.Events.Subject
				}
				noteRow(c.Key("problem"), k, problem.Explanation(bot.Name, runner, subject), nil)
			}
			scheduleTooltip := routine.Schedule
			if !strings.HasPrefix(routine.Schedule, "every ") {
				scheduleTooltip += " · " + routine.Timezone
			}
			keyValueRow(c.Key("schedule"), k, L("Schedule"), routine.ScheduleSummary(), false, nil).Tooltip(scheduleTooltip)
			if events := routine.Events; events != nil {
				w.routineEventRows(c, k, events, bot)
			}
			// The calendar names its account.
			if events := routine.Calendar; events != nil && events.Account != "" {
				keyValueRow(c.Key("calendar"), k, L("Calendar"), events.Account, false, nil)
			}
			// A routine on events has no next run: its events start it, held on the relay while
			// its Runner is off, so it has no missed runs either.
			if routine.Events == nil {
				routineNextRows(c, k, routine, runner)
			}
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
			// What its runs may use; the Limits sheet is where a stopped routine resumes.
			limits := Lc("None", "limits")
			if budget != nil {
				limits = budget.Limits.Summary()
			}
			if disclosureRow(c.Key("limits"), k, L("Limits"), limits, nil) {
				w.presentBudget(bot, store.DM(bot.ID), routine.ID)
			}
		})
		if events := routine.Events; events != nil && events.Endpoint != "" {
			w.routineWebhook(c, st, routine.ID, events)
		}
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

// routineNextRows are a scheduled routine's next run (its next check, when it looks first) and
// what happens to runs its Runner missed.
func routineNextRows(c *ui.Context, k *card, routine *model.Routine, runner string) {
	next, nextLabel := "—", L("Next run")
	if routine.LooksFirst() {
		nextLabel = L("Next check")
	}
	if routine.Calendar != nil && routine.IsEnabled {
		next = L("None in the next day")
	}
	if !routine.NextRunAt.IsZero() {
		// "Tomorrow 9:00 AM" on a line of its own, as the last check and run read; a
		// routine around events names the event it runs for.
		runes := []rune(model.Upcoming(routine.NextRunAt))
		runes[0] = unicode.ToUpper(runes[0])
		next = string(runes)
		if routine.Calendar != nil && routine.Calendar.NextEventTitle != "" {
			next += " · " + routine.Calendar.NextEventTitle
		}
	}
	keyValueRow(c.Key("next"), k, nextLabel, next, false, nil)
	// A one-time routine runs once its Runner is back, whatever the policy.
	if routine.OnceAt.IsZero() {
		missed, missedTooltip := L("Run once"), L("When %@ was off at a scheduled time, the routine runs once when it’s back.", runner)
		if routine.MissedRunPolicy == "skip" {
			missed, missedTooltip = L("Skip"), L("When %@ was off at a scheduled time, the routine waits for the next one.", runner)
		}
		keyValueRow(c.Key("missed"), k, L("Missed runs"), missed, false, nil).Tooltip(missedTooltip)
	}
}

// routineEventRows are what a routine on events listens to: its pull request, which opens on
// GitHub; how it hears the service, where an App that isn't installed yet installs from; and the
// latest event.
func (w *appWindow) routineEventRows(c *ui.Context, k *card, events *model.RoutineEvents, bot *model.Bot) {
	p := colors(c)
	if events.Subject != "" {
		label := events.SourceName
		if events.IsGitHub() {
			label = L("Pull request")
		}
		row := keyValueRow(c.Key("subject"), k, label, cmp.Or(events.Title, events.Subject), false, nil)
		if events.URL != "" {
			row.Tooltip(events.URL).Cursor(ui.CursorPointer)
			if row.Clicked() {
				openLink(events.URL)
			}
		}
	}
	service := cmp.Or(events.SourceName, events.Receiver)
	if events.IsGitHub() {
		service = "GitHub"
	}
	switch {
	case events.Status == "needs_setup":
		action := L("Set Up…")
		if events.IsGitHub() {
			action = L("Install App…")
		}
		row := keyValueRow(c.Key("service"), k, service, action, false, &p.Orange).Tooltip(L("Opens the page that sets it up for this account")).Cursor(ui.CursorPointer)
		if row.Clicked() {
			store.ReceiverSetupURL(events.Receiver, events.Subject, func(url string, err error) {
				if err != nil {
					w.showAlert(alertOptions{Message: L("Request failed"), Informative: model.ErrorText(err)}, nil)
					return
				}
				openLink(url)
			})
		}
	case events.Status == "gateway":
		keyValueRow(c.Key("service"), k, service, L("Your gateway"), false, nil)
		noteRow(c.Key("gateway"), k, L("This relay doesn’t take %@’s events itself, so they come through a gateway you run. %@ said how to set it up in the chat.", service, bot.Name), nil)
	case events.Status == "pending" && (!events.IsWebhook() || events.Endpoint == ""):
		keyValueRow(c.Key("service"), k, service, L("Connecting…"), false, nil)
	case !events.IsWebhook():
		keyValueRow(c.Key("service"), k, service, L("Connected"), false, nil)
	}
	last := L("None yet")
	if events.LastEvent != nil {
		last = L("%@ · %@", model.DaySeparator(events.LastEvent.At), events.LastEvent.Summary)
	}
	keyValueRow(c.Key("last-event"), k, L("Last event"), last, false, nil)
}

// routineWebhook is a routine's webhook: its URL, its key (hidden until a click shows it), and
// the header a sender pastes, each copied by its row's Copy; Regenerate makes a new key.
func (w *appWindow) routineWebhook(c *ui.Context, st *routineSheet, routineID string, events *model.RoutineEvents) {
	copied := func(row string) bool {
		at, ok := st.copiedAt[row]
		if !ok || c.Now().Sub(at) >= 1500*time.Millisecond {
			return false
		}
		c.After(1500*time.Millisecond - c.Now().Sub(at))
		return true
	}
	copyRow := func(row, text string) {
		mygo.Clipboard.WriteText(text)
		st.copiedAt[row] = c.Now()
	}
	copyTitle := func(row string) string {
		if copied(row) {
			return L("Copied")
		}
		return L("Copy")
	}
	key := strings.Repeat("•", 12)
	if st.keyShown {
		key = events.Key
	}
	section(c.Key("webhook"), L("Webhook"), sectionCaption, nil, func(k *card) {
		_, url := actionRow(c.Key("webhook-url"), k, L("Webhook URL"), actionRowOptions{Value: events.Endpoint, Action: copyTitle("url"), Copied: copied("url"), Tooltip: events.Endpoint})
		if url.Action {
			copyRow("url", events.Endpoint)
		}
		tooltip := L("Click to show the key")
		if st.keyShown {
			tooltip = L("Click to hide the key")
		}
		row, keyRow := actionRow(c.Key("webhook-key"), k, L("Webhook key"), actionRowOptions{Value: key, Action: copyTitle("key"), Copied: copied("key"), Second: L("Regenerate…"), Tooltip: tooltip})
		row.Cursor(ui.CursorPointer)
		switch {
		case keyRow.Action:
			copyRow("key", events.Key)
		case keyRow.Second:
			w.confirmRegenerateKey(st, routineID)
		case row.Clicked():
			st.keyShown = !st.keyShown
		}
		_, header := actionRow(c.Key("webhook-header"), k, L("Authorization header"), actionRowOptions{Value: "Bearer " + key, Action: copyTitle("header"), Copied: copied("header"), Tooltip: L("The header line that carries the key in every request, ready to paste.")})
		if header.Action {
			copyRow("header", events.AuthorizationHeader())
		}
		noteRow(c.Key("webhook-note"), k, L("Each request to this URL runs the routine once, with what it sent. Senders include the key in an Authorization: Bearer header; share it only with the service that calls this routine."), nil)
	})
}

// confirmRegenerateKey asks before making a new webhook key, which stops the old one.
func (w *appWindow) confirmRegenerateKey(st *routineSheet, routineID string) {
	w.showAlert(alertOptions{
		Message:     L("Regenerate the webhook key?"),
		Informative: L("Services that send the current key stop reaching this routine until you give them the new one."),
		Style:       alertWarning,
		Buttons:     []alertButton{{Title: L("Regenerate Key")}, {Title: L("Cancel")}},
	}, func(index int) {
		if index != 0 {
			return
		}
		store.RegenerateRoutineKey(routineID, func(err error) {
			if err != nil {
				w.showAlert(alertOptions{Message: L("Request failed"), Informative: model.ErrorText(err)}, nil)
				return
			}
			st.keyShown = true
		})
	})
}

// routineActions are the sheet's actions on the routine: Run Now, Pause or Resume, Edit in Chat,
// and Delete apart from them.
func (w *appWindow) routineActions(c *ui.Context, s *sheet, routine *model.Routine, bot *model.Bot, onEditInChat func(text string)) {
	ui.Row(c).Grow(1).Gap(8).Children(func() {
		tooltip := L("Runs on the bot's Runner now")
		if runner := store.Device(bot.RunnerID); runner != nil {
			tooltip = L("Runs on %@ now", runner.Name)
		}
		// A run needs its Runner online, a routine paused by failed sign-ins needs Resume, and one
		// stopped at its limits resumes in Limits.
		budget := store.Budget("routine", routine.ID, bot.RunnerID)
		blocked := routine.IsRunning || routine.State == "waiting_for_runner" || routine.PausedReason == "authentication" || budget != nil && budget.IsStopped()
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
