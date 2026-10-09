package main

import (
	"encoding/json"
	"net/url"
	"slices"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// taskSheet is one durable task, after the Mac's task sheet. A card on top says where it stands,
// why when it is blocked or cancelled, its result and what supports it, and offers the one step
// that fits: Start, Resume, Mark Complete, or Reopen. Below it the goal, owner, next step, and
// what done looks like are a form that Save writes back, then the tasks it waits for and its
// links, when it has them.
//
// Edits start from the version the sheet showed. A change from elsewhere replaces the form while
// the user has not touched it. When Save finds the task changed underneath, the sheet takes the
// newer version under the user's edits and says so; the next Save writes them. A new task is the
// same form, and once created the sheet shows it.
type taskSheet struct {
	w      *appWindow
	chatID string
	// base is the version the form's edits start from; nil until a new task is created.
	base       *model.DurableTask
	creationID string
	goal, next string
	criteria   string
	owner      string
	// message says what went wrong, in red when failed.
	message string
	failed  bool
	busy    bool
	closed  bool
	// sent is the last request and its id: a retry after an unclear failure sends the same id, so
	// the CLI answers it once.
	sentBody, sentID string
}

func (w *appWindow) presentDurableTask(chatID string, task *model.DurableTask) *taskSheet {
	st := &taskSheet{w: w, chatID: chatID, creationID: model.TaskID("task-")}
	if task != nil {
		st.populate(task)
	} else if chat := store.Chat(chatID); chat != nil {
		st.owner = chat.Owner()
		if owners := st.owners(); st.owner == "" && len(owners) > 0 {
			st.owner = owners[0].ID
		}
	}
	w.present(st.view, func() { st.closed = true })
	return st
}

// taskForm is what the form holds, trimmed, for comparing with a version of the task.
type taskForm struct {
	goal, next string
	criteria   []string
	owner      string
}

func (f taskForm) equal(g taskForm) bool {
	return f.goal == g.goal && f.next == g.next && slices.Equal(f.criteria, g.criteria) && f.owner == g.owner
}

func (st *taskSheet) form() taskForm {
	return taskForm{goal: strings.TrimSpace(st.goal), next: strings.TrimSpace(st.next), criteria: taskLines(st.criteria), owner: st.owner}
}

func formOf(task *model.DurableTask) taskForm {
	return taskForm{goal: strings.TrimSpace(task.Goal), next: strings.TrimSpace(task.NextAction), criteria: task.AcceptanceCriteria, owner: task.OwnerBotID}
}

func taskLines(text string) []string {
	out := []string{}
	for _, line := range strings.Split(text, "\n") {
		if line = strings.TrimSpace(line); line != "" {
			out = append(out, line)
		}
	}
	return out
}

func (st *taskSheet) populate(task *model.DurableTask) {
	copy := task.Clone()
	st.base = &copy
	st.goal, st.next, st.criteria, st.owner = task.Goal, task.NextAction, strings.Join(task.AcceptanceCriteria, "\n"), task.OwnerBotID
}

func (st *taskSheet) hasEdits() bool { return st.base != nil && !st.form().equal(formOf(st.base)) }

func (st *taskSheet) canSave() bool {
	f := st.form()
	return !st.busy && f.owner != "" && f.goal != "" && f.next != "" && len(f.criteria) > 0
}

// rebase takes `latest` under the user's edits: a field they changed keeps their words, the
// others take the newer version's.
func (st *taskSheet) rebase(latest *model.DurableTask) {
	if st.base == nil {
		return
	}
	before, now := formOf(st.base), st.form()
	if now.goal == before.goal {
		st.goal = latest.Goal
	}
	if now.next == before.next {
		st.next = latest.NextAction
	}
	if slices.Equal(now.criteria, before.criteria) {
		st.criteria = strings.Join(latest.AcceptanceCriteria, "\n")
	}
	if now.owner == before.owner {
		st.owner = latest.OwnerBotID
	}
	copy := latest.Clone()
	st.base = &copy
}

// latest is the newest version the store has, which the status and the sections show.
func (st *taskSheet) latest() *model.DurableTask {
	if st.base == nil {
		return nil
	}
	if task := store.DurableTask(st.base.ID); task != nil {
		return task
	}
	return st.base
}

// owners are the bots of the task's chats, which the owner is one of.
func (st *taskSheet) owners() []*model.Bot {
	chats := []string{st.chatID}
	if st.base != nil {
		chats = st.base.ChatIDs
	}
	var bots []*model.Bot
	for _, bot := range store.Bots {
		if slices.ContainsFunc(chats, func(id string) bool {
			chat := store.Chat(id)
			return chat != nil && slices.Contains(chat.BotIDs, bot.ID)
		}) {
			bots = append(bots, bot)
		}
	}
	return bots
}

func (st *taskSheet) view(c *ui.Context, s *sheet) {
	p := colors(c)
	// A change from elsewhere replaces an untouched form.
	if st.base != nil && !st.busy {
		if newer := store.DurableTask(st.base.ID); newer != nil && newer.Revision > st.base.Revision && !st.hasEdits() {
			st.populate(newer)
		}
	}
	task := st.latest()
	title, subtitle, confirm := L("Task"), "", L("Save")
	if task == nil {
		title, subtitle, confirm = L("New Task"), L("A task stays with its bot across turns, until it’s done."), L("Create Task")
	}
	// A running task keeps its owner and its definition of done until the run ends.
	running := task != nil && task.ActiveRun != nil
	const width = 520
	fill := float32(width - 40 - formLabelWidth - 10)
	result := sheetFrame(c, sheetOptions{
		Title: title, Subtitle: subtitle, Width: width, Confirm: confirm, ConfirmDisabled: !st.canSave(),
		Leading: func() {
			if task != nil && !task.State.Finished() {
				if pushButton(c.Key("cancel-task"), L("Cancel Task…"), pushOptions{Kind: buttonDestructive, Disabled: st.busy}).Clicked() {
					st.confirmCancel(s)
				}
			}
		},
	}, func() {
		if task != nil {
			st.statusCard(c.Key("status"), s, task).Margin(0, 0, 4, 0)
		}
		formRow(c.Key("goal"), L("Goal"), false, func() {
			textField(c, &st.goal, fieldOptions{Placeholder: L("What should get done"), Label: L("Goal")}).Grow(1).MinWidth(0)
		})
		if owners := st.owners(); len(owners) > 1 {
			formRow(c.Key("owner"), L("Owner"), false, func() {
				var options []popUpOption
				for _, bot := range owners {
					options = append(options, popUpOption{Value: bot.ID, Label: bot.Name})
				}
				if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: st.owner, Width: fill, Label: L("Owner"), Disabled: running || st.busy}); changed {
					st.owner = picked
				}
			})
		}
		formRow(c.Key("next"), L("Next step"), false, func() {
			textField(c, &st.next, fieldOptions{Placeholder: L("What the bot does first"), Label: L("Next step")}).Grow(1).MinWidth(0)
		})
		// Return starts a new line here, one check per line.
		formRow(c.Key("criteria"), L("Done when"), true, func() {
			textArea(c, &st.criteria, 0, fieldOptions{Placeholder: L("One check per line"), Label: L("Done when"), Disabled: running}).
				Lines(3, 10).Grow(1).MinWidth(0).MinHeight(54)
		})
		if task != nil && len(task.Dependencies) > 0 {
			section(c.Key("waits-for"), L("Waits For"), sectionCaption, nil, func(k *card) {
				for _, id := range task.Dependencies {
					o := statusRowOptions{Symbol: "circle", Title: L("A task on another Device")}
					tint := p.Label3
					if dependency := store.DurableTask(id); dependency != nil {
						o.Symbol, o.Title, o.Subtitle = dependency.State.Symbol(), dependency.Goal, dependency.State.Title()
						tint = taskTint(p, dependency.State)
					}
					o.SymbolColor = &tint
					ui.Box(c.Key(id)).Children(func() { statusRow(c, k, o) })
				}
			}).Margin(4, 0, 0, 0)
		}
		if task != nil && len(task.Links) > 0 {
			section(c.Key("links"), L("Links"), sectionCaption, nil, func(k *card) {
				for i, link := range task.Links {
					o := statusRowOptions{Symbol: "link", Title: link.Label, Tooltip: link.URL, Clickable: true}
					if u, err := url.Parse(link.URL); err == nil && link.Label != link.URL {
						o.Subtitle = u.Hostname()
					}
					href := link.URL
					ui.Box(c.Key(i)).Children(func() {
						if _, row := statusRow(c, k, o); row.Clicked {
							openLink(href)
						}
					})
				}
			}).Margin(4, 0, 0, 0)
		}
		if st.message != "" {
			tint := p.Label2
			if st.failed {
				tint = p.Red
			}
			ui.Text(c.Key("message"), st.message).FontSize(12).TextColor(tint).LineHeight(1.4).Selectable()
		}
	})
	switch {
	case result.Cancelled:
		s.dismiss()
	case result.Confirmed:
		st.save(s)
	}
}

// statusCard is the state, why, the result and what supports it, and the step the state allows.
func (st *taskSheet) statusCard(c *ui.Context, s *sheet, task *model.DurableTask) ui.Element {
	p := colors(c)
	detail, action := "", ""
	switch task.State {
	case model.TaskQueued:
		action = L("Start")
	case model.TaskBlocked:
		detail, action = task.ReasonText(), L("Resume")
	case model.TaskWorking:
		if bot := store.Bot(task.OwnerBotID); task.ActiveRun != nil && bot != nil {
			detail = L("%@ is working on it.", bot.Name)
		}
	case model.TaskAwaitingReview:
		if task.ResultText() != "" && len(task.Evidence) > 0 {
			action = L("Mark Complete")
		}
	case model.TaskCompleted:
		action = L("Reopen")
	case model.TaskCancelled:
		if task.ReasonText() != model.CancelledByUser {
			detail = task.ReasonText()
		}
		action = L("Reopen")
	}
	if task.CanStart() && slices.ContainsFunc(task.Dependencies, func(id string) bool {
		dependency := store.DurableTask(id)
		return dependency == nil || dependency.State != model.TaskCompleted
	}) {
		detail, action = L("It starts once the tasks it waits for are completed."), ""
	}
	if st.busy {
		action = ""
	}
	tint := taskTint(p, task.State)
	return section(c, "", sectionCaption, nil, func(k *card) {
		if _, row := statusRow(c.Key("state"), k, statusRowOptions{Symbol: task.State.Symbol(), SymbolColor: &tint, Title: task.State.Title(), Subtitle: detail, ActionTitle: action}); row.Action {
			st.statusAction(s, task)
		}
		if result := task.ResultText(); result != "" {
			k.row(ui.Row(c.Key("result")).Padding(10, 12, 10, 40)).Children(func() {
				ui.Text(c, result).FontSize(12).LineHeight(1.4).MaxLines(8).Selectable().Grow(1).Shrink(1).MinWidth(0)
			})
		}
		const limit = 5
		evidence := task.Evidence
		if len(evidence) > limit {
			evidence = evidence[:limit-1]
		}
		for i, item := range evidence {
			ui.Box(c.Key(i)).Children(func() { st.evidenceRow(c, k, item) })
		}
		if len(task.Evidence) > limit {
			noteRow(c.Key("more"), k, L("%d more", len(task.Evidence)-(limit-1)), nil)
		}
	})
}

// evidenceRow is a message by who wrote it and when, an output by its name and version (a click
// opens that version), a link by where it goes, or the CLI's label.
func (st *taskSheet) evidenceRow(c *ui.Context, k *card, item model.TaskEvidence) {
	o := statusRowOptions{Symbol: "bubble.left", Title: item.Label}
	var output *model.Message
	switch item.Kind {
	case "url":
		o.Symbol = "link"
	case "file":
		o.Symbol = "doc.text"
	case "output":
		o.Symbol = "doc.richtext"
		if item.Version != nil {
			o.Subtitle = L("Version %d", int(*item.Version))
		}
		if item.ChatID != nil && item.MessageID != nil {
			if chat := store.Chat(*item.ChatID); chat != nil {
				for _, message := range chat.Messages {
					if message.ID == *item.MessageID && message.Output != nil {
						output = message
						o.Tooltip, o.Clickable = L("Open %@", item.Label), true
					}
				}
			}
		}
	case "review":
		o.Symbol = "checkmark.circle"
	case "message":
		o.Title = L("Message")
		if item.ChatID != nil && item.MessageID != nil {
			if chat := store.Chat(*item.ChatID); chat != nil {
				for _, message := range chat.Messages {
					if message.ID != *item.MessageID {
						continue
					}
					switch message.Author.Kind {
					case model.AuthorYou:
						o.Title = L("Your message")
					case model.AuthorBot:
						name := L("a bot")
						if bot := store.Bot(message.Author.BotID); bot != nil {
							name = bot.Name
						}
						o.Title = L("Message from %@", name)
					}
					o.Subtitle = model.DaySeparator(message.CreatedAt)
				}
			}
		}
	}
	if item.URL != nil {
		if u, err := url.Parse(*item.URL); err == nil {
			o.Subtitle = u.Hostname()
		}
		o.Tooltip, o.Clickable = *item.URL, true
	}
	if _, row := statusRow(c, k, o); row.Clicked {
		switch {
		case output != nil:
			st.w.openOutput(output)
		case item.URL != nil:
			openLink(*item.URL)
		}
	}
}

// changes are the fields the user changed from `task`.
func (st *taskSheet) changes(task *model.DurableTask) map[string]any {
	before, now := formOf(task), st.form()
	params := map[string]any{}
	if now.goal != before.goal {
		params["goal"] = now.goal
	}
	if now.next != before.next {
		params["next_action"] = now.next
	}
	if !slices.Equal(now.criteria, before.criteria) {
		params["acceptance_criteria"] = now.criteria
	}
	if now.owner != "" && now.owner != before.owner {
		params["owner_bot_id"] = now.owner
	}
	return params
}

func (st *taskSheet) save(s *sheet) {
	if !st.canSave() {
		return
	}
	if st.base == nil {
		f := st.form()
		st.send("tasks.create", map[string]any{
			"id": st.creationID, "owner_bot_id": f.owner, "goal": f.goal, "next_action": f.next,
			"acceptance_criteria": f.criteria, "chat_ids": []string{st.chatID},
		}, func(task model.DurableTask) { st.populate(&task) })
		return
	}
	if len(st.changes(st.base)) == 0 {
		s.dismiss()
		return
	}
	st.update(nil, func(model.DurableTask) { s.dismiss() })
}

// update writes the form's edits, and `params` with them, over the version they start from.
func (st *taskSheet) update(params map[string]any, done func(model.DurableTask)) {
	if st.base == nil {
		return
	}
	edits := st.changes(st.base)
	for key, value := range params {
		edits[key] = value
	}
	edits["id"], edits["expected_revision"] = st.base.ID, st.base.Revision
	st.send("tasks.update", edits, done)
}

func (st *taskSheet) statusAction(s *sheet, task *model.DurableTask) {
	switch task.State {
	case model.TaskQueued, model.TaskBlocked, model.TaskWorking:
		st.start(s)
	case model.TaskAwaitingReview:
		st.update(map[string]any{"state": string(model.TaskCompleted)}, func(model.DurableTask) { s.dismiss() })
	case model.TaskCompleted, model.TaskCancelled:
		st.update(map[string]any{"state": string(model.TaskQueued)}, nil)
	}
}

// start saves what the user changed, then starts a run in this chat when the owner is in it, or
// in the first of the task's chats it is in, and closes on the chat where it works.
func (st *taskSheet) start(s *sheet) {
	run := func(task model.DurableTask) {
		params := map[string]any{"id": task.ID, "expected_revision": task.Revision}
		if chat := store.Chat(st.chatID); chat != nil && slices.Contains(chat.BotIDs, task.OwnerBotID) {
			params["chat_id"] = st.chatID
		}
		st.send("tasks.run", params, func(model.DurableTask) { s.dismiss() })
	}
	if st.base == nil {
		return
	}
	if len(st.changes(st.base)) == 0 {
		run(*st.base)
	} else {
		st.update(nil, run)
	}
}

func (st *taskSheet) confirmCancel(s *sheet) {
	task := st.latest()
	if task == nil {
		return
	}
	informative := L("You can reopen it later.")
	if bot := store.Bot(task.OwnerBotID); task.ActiveRun != nil && bot != nil {
		informative = L("%@ stops working on it. You can reopen it later.", bot.Name)
	}
	st.w.showAlert(alertOptions{
		Message:     L("Cancel this task?"),
		Informative: informative,
		Buttons:     []alertButton{{Title: L("Cancel Task"), Destructive: true}, {Title: L("Keep Task")}},
		Escape:      1,
	}, func(answer int) {
		if answer == 0 {
			st.update(map[string]any{"state": string(model.TaskCancelled), "reason": model.CancelledByUser}, func(model.DurableTask) { s.dismiss() })
		}
	})
}

func (st *taskSheet) send(method string, params map[string]any, done func(model.DurableTask)) {
	if st.busy {
		return
	}
	body, _ := json.Marshal(map[string]any{"method": method, "params": params})
	if string(body) != st.sentBody {
		st.sentBody, st.sentID = string(body), model.TaskID("")
	}
	params["request_id"] = st.sentID
	st.busy, st.message = true, ""
	store.TaskRequest(method, params, func(task model.DurableTask, err error) {
		if st.closed {
			return
		}
		st.busy = false
		if err != nil {
			st.fail(err)
			return
		}
		st.sentBody, st.sentID = "", ""
		if done != nil {
			done(task)
		}
	})
}

// fail takes the newer version under the user's edits when theirs was stale, and says what the
// CLI said otherwise.
func (st *taskSheet) fail(err error) {
	if !strings.HasPrefix(err.Error(), "Task revision conflict") || st.base == nil {
		st.message, st.failed = model.ErrorText(err), true
		return
	}
	st.message, st.failed = L("This task changed since you opened it. Your edits are still here; save again to keep them."), false
	st.busy = true
	store.TaskRequest("tasks.get", map[string]any{"id": st.base.ID, "refresh": true}, func(task model.DurableTask, err error) {
		if st.closed {
			return
		}
		st.busy = false
		if err == nil {
			st.rebase(&task)
		}
	})
}
