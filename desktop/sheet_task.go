package main

import (
	"encoding/json"
	"net/url"
	"reflect"
	"slices"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A task sheet owns plain persistent draft values. No element/context survives a build pass.
// Its opened revision stays fixed until Reload; newer events never overwrite typing.
type taskSheet struct {
	w                                                               *appWindow
	opened                                                          *model.DurableTask
	creationID, chatID, runIn                                       string
	goal, criteria, next, reason, result, owner, links, evidenceURL string
	state                                                           model.TaskState
	chats, dependencies                                             []string
	evidence                                                        []model.TaskEvidence
	error                                                           string
	busy, closed                                                    bool
	requestID, requestBody, runID, runBody                          string
}

func (w *appWindow) presentDurableTask(chatID string, task *model.DurableTask) *taskSheet {
	st := &taskSheet{w: w, chatID: chatID, creationID: model.TaskID("task-")}
	st.load(task)
	w.present(st.view, func() { st.closed = true })
	return st
}

func (st *taskSheet) load(task *model.DurableTask) {
	st.opened = nil
	st.chats = []string{st.chatID}
	st.state = model.TaskQueued
	st.goal, st.criteria, st.next, st.reason, st.result, st.owner, st.links = "", "", "", "", "", "", ""
	st.dependencies = nil
	st.evidence = nil
	st.error = ""
	st.runID, st.runBody = "", ""
	st.runIn = ""
	if task != nil {
		copy := task.Clone()
		st.opened = &copy
		st.goal, st.criteria, st.next = copy.Goal, strings.Join(copy.AcceptanceCriteria, "\n"), copy.NextAction
		st.reason, st.result, st.owner = copy.ReasonText(), copy.ResultText(), copy.OwnerBotID
		st.chats, st.dependencies = slices.Clone(copy.ChatIDs), slices.Clone(copy.Dependencies)
		st.evidence = copy.Clone().Evidence
		st.state = copy.State
		var links []string
		for _, link := range copy.Links {
			links = append(links, link.URL)
		}
		st.links = strings.Join(links, "\n")
		for _, id := range st.runChats() {
			if st.runIn == "" || id == st.chatID {
				st.runIn = id
			}
		}
	} else if bots := st.owners(); len(bots) > 0 {
		st.owner = bots[0].ID
	}
}

func (st *taskSheet) active() bool { return st.opened != nil && st.opened.ActiveRun != nil }

func (st *taskSheet) savedOwner() *model.Bot {
	if st.opened == nil {
		return nil
	}
	if current := store.DurableTask(st.opened.ID); current != nil && (current.OwnerBotID != st.opened.OwnerBotID || current.RunnerID != st.opened.RunnerID) {
		return nil
	}
	bot := store.Bot(st.opened.OwnerBotID)
	if bot == nil || bot.RunnerID != st.opened.RunnerID {
		return nil
	}
	return bot
}
func (st *taskSheet) owners() []*model.Bot {
	var bots []*model.Bot
	for _, bot := range store.Bots {
		if slices.ContainsFunc(st.chats, func(id string) bool {
			chat := store.Chat(id)
			return chat != nil && slices.Contains(chat.BotIDs, bot.ID)
		}) {
			bots = append(bots, bot)
		}
	}
	return bots
}
func taskChatTitle(id string) string {
	if chat := store.Chat(id); chat != nil {
		return store.Title(chat)
	}
	return id
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
func taskNullable(value string) any {
	if strings.TrimSpace(value) == "" {
		return nil
	}
	return value
}
func taskHTTPS(value string) bool {
	u, err := url.Parse(value)
	return err == nil && u.Scheme == "https" && u.Hostname() != "" && u.User == nil
}

func taskField(c *ui.Context, label string, content func()) {
	ui.Column(c).Gap(5).Children(func() { ui.Text(c, label).FontSize(12).FontWeight(600); content() })
}

func (st *taskSheet) view(c *ui.Context, s *sheet) {
	confirm := L("Save")
	title := L("Task")
	if st.opened == nil {
		title = L("New task")
	}
	result := sheetFrame(c, sheetOptions{Title: title, Subtitle: L("Keep ownership, progress, and evidence across turns."), Width: 590, Confirm: confirm, ConfirmDisabled: st.busy || len(st.owners()) == 0, ReturnInContent: true, Status: st.error,
		Leading: func() {
			if taskBudgetOpener != nil && st.opened != nil {
				bot := st.savedOwner()
				if pushButton(c.Key("budget"), L("Budget…"), pushOptions{Disabled: st.busy || bot == nil || st.runChatID() == ""}).Clicked() {
					taskBudgetOpener(st.w, bot, st.runChatID(), st.opened.ID)
				}
			}
			if pushButton(c.Key("reload"), L("Reload"), pushOptions{Disabled: st.opened == nil || st.busy}).Clicked() {
				st.reload()
			}
			canRun := st.opened != nil && st.opened.State.CanRun() && st.opened.ActiveRun == nil && st.savedOwner() != nil && st.runChatID() != ""
			if pushButton(c.Key("run"), L("Start saved task"), pushOptions{Disabled: st.busy || !canRun}).Clicked() {
				st.run(s)
			}
		},
	}, func() {
		if st.opened != nil {
			ui.Text(c, st.opened.ID).FontSize(10).TextColor(colors(c).Label3).Selectable()
		}
		taskField(c.Key("goal"), L("Goal"), func() { textField(c, &st.goal, fieldOptions{Label: L("Task goal"), Disabled: st.busy}) })
		taskField(c.Key("owner"), L("Owning bot"), func() {
			var options []popUpOption
			for _, bot := range st.owners() {
				runner := bot.RunnerID
				if device := store.Device(runner); device != nil {
					runner = device.Name
				}
				options = append(options, popUpOption{Value: bot.ID, Label: bot.Name + " · " + runner})
			}
			if selected, ok, _ := popUpButton(c, popUp{Value: st.owner, Options: options, Width: 540, Label: L("Task owner"), Disabled: st.busy || st.active()}); ok {
				st.owner = selected
			}
		})
		if bot := store.Bot(st.owner); bot != nil {
			runner := bot.RunnerID
			if st.opened != nil && st.owner == st.opened.OwnerBotID {
				runner = st.opened.RunnerID
			}
			if device := store.Device(runner); device != nil {
				runner = device.Name
			}
			ui.Text(c, L("Assigned Runner: %@", runner)).FontSize(11).TextColor(colors(c).Label2)
		}
		if st.opened != nil && st.savedOwner() == nil {
			ui.Text(c, L("Resolve the owner's Runner assignment before running or changing the task budget.")).FontSize(11).TextColor(colors(c).Orange)
		}
		taskField(c.Key("state"), L("State"), func() {
			var options []popUpOption
			for _, state := range model.TaskStates() {
				options = append(options, popUpOption{Value: string(state), Label: state.Title()})
			}
			if selected, ok, _ := popUpButton(c, popUp{Value: string(st.state), Options: options, Width: 540, Label: L("Task state"), Disabled: st.opened == nil || st.busy}); ok {
				st.state = model.TaskState(selected)
			}
		})
		st.chatFields(c.Key("chats"))
		if st.opened != nil {
			taskField(c.Key("run-in"), L("Saved task runs in"), func() {
				options := []popUpOption{{Value: "", Label: L("Choose a chat…")}}
				for _, id := range st.runChats() {
					options = append(options, popUpOption{Value: id, Label: taskChatTitle(id)})
				}
				if id, ok, _ := popUpButton(c, popUp{Options: options, Value: st.runChatID(), Width: 540, Label: L("Primary run chat"), Disabled: st.busy || st.active()}); ok {
					st.runIn = id
				}
			})
		}
		taskField(c.Key("criteria"), L("Acceptance criteria · one per line"), func() {
			textArea(c, &st.criteria, 0, fieldOptions{Label: L("Acceptance criteria"), Disabled: st.busy || st.active()}).Height(66)
		})
		taskField(c.Key("next"), L("Next action"), func() {
			textArea(c, &st.next, 0, fieldOptions{Label: L("Task next action"), Disabled: st.busy}).Height(52)
		})
		if st.opened != nil {
			taskField(c.Key("reason"), L("Reason · required when blocked or cancelled"), func() {
				textArea(c, &st.reason, 0, fieldOptions{Label: L("Task reason"), Disabled: st.busy}).Height(52)
			})
			taskField(c.Key("result"), L("Result · required for completion"), func() {
				textArea(c, &st.result, 0, fieldOptions{Label: L("Task result"), Disabled: st.busy}).Height(80)
			})
		}
		st.dependencyFields(c.Key("dependencies"))
		taskField(c.Key("links"), L("External links · one HTTPS URL per line"), func() {
			textArea(c, &st.links, 0, fieldOptions{Label: L("External links"), Disabled: st.busy}).Height(52)
		})
		if st.opened != nil {
			st.evidenceFields(c.Key("evidence"))
		} else {
			ui.Text(c, L("Save the task to record progress and supporting evidence.")).FontSize(11).TextColor(colors(c).Label2)
		}
	})
	if result.Cancelled {
		s.dismiss()
	} else if result.Confirmed {
		st.save(s)
	}
}

func (st *taskSheet) chatFields(c *ui.Context) {
	taskField(c, L("Linked chats"), func() {
		for _, id := range slices.Clone(st.chats) {
			ui.Row(c.Key(id)).Gap(8).Children(func() {
				ui.Text(c, taskChatTitle(id)).Grow(1).FontSize(12)
				if linkButton(c, L("Remove"), st.busy || st.active()).Label(L("Remove chat %@", taskChatTitle(id))).Clicked() {
					st.chats = slices.DeleteFunc(st.chats, func(value string) bool { return value == id })
				}
			})
		}
		options := []popUpOption{{Value: "", Label: L("Choose a chat…")}}
		for _, chat := range store.Chats {
			if !slices.Contains(st.chats, chat.ID) {
				options = append(options, popUpOption{Value: chat.ID, Label: store.Title(chat)})
			}
		}
		if id, ok, _ := popUpButton(c.Key("add"), popUp{Options: options, Value: "", Width: 540, Label: L("Add linked chat"), Disabled: st.busy || st.active()}); ok {
			st.chats = append(st.chats, id)
		}
	})
}
func (st *taskSheet) dependencyFields(c *ui.Context) {
	taskField(c, L("Dependencies"), func() {
		for _, id := range append([]string{}, st.dependencies...) {
			goal := id
			if task := store.DurableTask(id); task != nil {
				goal = task.Goal
			}
			ui.Row(c.Key(id)).Gap(8).Children(func() {
				ui.Text(c, goal).Grow(1).FontSize(12)
				if linkButton(c, L("Remove"), st.busy || st.active()).Label(L("Remove dependency %@", goal)).Clicked() {
					st.dependencies = slices.DeleteFunc(st.dependencies, func(value string) bool { return value == id })
				}
			})
		}
		options := []popUpOption{{Value: "", Label: L("Choose a task…")}}
		for _, task := range store.DurableTasks {
			if (st.opened == nil || task.ID != st.opened.ID) && !slices.Contains(st.dependencies, task.ID) {
				options = append(options, popUpOption{Value: task.ID, Label: task.Goal})
			}
		}
		if id, ok, _ := popUpButton(c.Key("add"), popUp{Options: options, Value: "", Width: 540, Label: L("Add dependency"), Disabled: st.busy || st.active()}); ok {
			st.dependencies = append(st.dependencies, id)
		}
	})
}
func (st *taskSheet) evidenceFields(c *ui.Context) {
	taskField(c, L("Supporting evidence"), func() {
		for i, evidence := range append([]model.TaskEvidence{}, st.evidence...) {
			ui.Row(c.Key(i)).Gap(8).Children(func() {
				ui.Column(c).Grow(1).MinWidth(0).Children(func() {
					ui.Text(c, evidence.Label).FontSize(12)
					if evidence.URL != nil {
						ui.Text(c, *evidence.URL).FontSize(11).TextColor(colors(c).Label2).Selectable()
					} else {
						var details []string
						if evidence.ChatID != nil {
							details = append(details, taskChatTitle(*evidence.ChatID))
						}
						if evidence.OutputID != nil && evidence.Version != nil {
							details = append(details, L("Output version %d", *evidence.Version))
						}
						if evidence.MessageID != nil {
							details = append(details, *evidence.MessageID)
						}
						if len(details) > 0 {
							ui.Text(c, strings.Join(details, " · ")).FontSize(11).TextColor(colors(c).Label2).Selectable()
						}
					}
				})
				if linkButton(c, L("Remove"), st.busy).Label(L("Remove evidence %@", evidence.Label)).Clicked() {
					st.evidence = slices.Delete(st.evidence, i, i+1)
				}
			})
		}
		options := []popUpOption{{Value: "", Label: L("Choose a message…")}}
		references := map[string]model.TaskEvidence{}
		for _, chatID := range st.chats {
			if chat := store.Chat(chatID); chat != nil {
				for _, message := range chat.Messages {
					if message.CanBeQuoted() {
						label := model.MessageText(message)
						if len(label) > 100 {
							label = string([]rune(label)[:min(100, len([]rune(label)))])
						}
						id := message.ID
						scope := chatID
						key := chatID + ":" + id
						options = append(options, popUpOption{Value: key, Label: label})
						references[key] = model.TaskEvidence{Kind: "message", Label: label, ChatID: &scope, MessageID: &id}
					}
				}
			}
		}
		if id, ok, _ := popUpButton(c.Key("message"), popUp{Options: options, Value: "", Width: 540, Label: L("Add a chat message as evidence"), Disabled: st.busy}); ok {
			st.evidence = append(st.evidence, references[id])
		}
		ui.Row(c.Key("url")).Gap(8).Children(func() {
			textField(c, &st.evidenceURL, fieldOptions{Label: L("Evidence URL"), Placeholder: L("HTTPS link to supporting evidence"), Disabled: st.busy}).Grow(1)
			if pushButton(c, L("Add link"), pushOptions{Disabled: st.busy}).Clicked() {
				value := strings.TrimSpace(st.evidenceURL)
				if !taskHTTPS(value) {
					st.error = L("Enter an HTTPS link.")
				} else {
					st.evidence = append(st.evidence, model.TaskEvidence{Kind: "url", Label: value, URL: &value})
					st.evidenceURL = ""
					st.error = ""
				}
			}
		})
	})
}

func (st *taskSheet) parameters() map[string]any {
	p := map[string]any{}
	if st.opened == nil {
		p["id"] = st.creationID
		p["owner_bot_id"] = st.owner
		p["goal"] = st.goal
		p["acceptance_criteria"] = taskLines(st.criteria)
		p["next_action"] = st.next
		p["chat_ids"] = slices.Clone(st.chats)
		p["dependencies"] = append([]string{}, st.dependencies...)
		p["links"] = st.linkValues()
		return p
	}
	old := st.opened
	p["id"], p["expected_revision"] = old.ID, old.Revision
	if st.goal != old.Goal {
		p["goal"] = st.goal
	}
	if st.next != old.NextAction {
		p["next_action"] = st.next
	}
	if st.owner != old.OwnerBotID {
		p["owner_bot_id"] = st.owner
		if bot := store.Bot(st.owner); bot != nil && bot.RunnerID != old.RunnerID {
			p["runner_id"] = bot.RunnerID
		}
	}
	if !slices.Equal(taskLines(st.criteria), old.AcceptanceCriteria) {
		p["acceptance_criteria"] = taskLines(st.criteria)
	}
	if !slices.Equal(st.dependencies, old.Dependencies) {
		p["dependencies"] = append([]string{}, st.dependencies...)
	}
	if !slices.Equal(st.chats, old.ChatIDs) {
		p["chat_ids"] = slices.Clone(st.chats)
	}
	if st.state != old.State {
		p["state"] = st.state
	}
	if st.reason != old.ReasonText() {
		p["reason"] = taskNullable(st.reason)
	}
	if st.result != old.ResultText() {
		p["result"] = taskNullable(st.result)
	}
	if !reflect.DeepEqual(st.evidence, old.Evidence) {
		p["evidence"] = append([]model.TaskEvidence{}, st.evidence...)
	}
	var oldURLs []string
	for _, link := range old.Links {
		oldURLs = append(oldURLs, link.URL)
	}
	if !slices.Equal(taskLines(st.links), oldURLs) {
		p["links"] = st.linkValues()
	}
	return p
}
func (st *taskSheet) linkValues() []model.TaskLink {
	out := []model.TaskLink{}
	for _, value := range taskLines(st.links) {
		out = append(out, model.TaskLink{Label: value, URL: value})
	}
	return out
}
func taskRequestKey(params map[string]any, id, body *string) {
	bytes, _ := json.Marshal(params)
	if *body != string(bytes) {
		*body = string(bytes)
		*id = model.TaskID("task-request-")
	}
	params["request_id"] = *id
}

func (st *taskSheet) save(s *sheet) {
	if st.busy {
		return
	}
	params := st.parameters()
	taskRequestKey(params, &st.requestID, &st.requestBody)
	method := "tasks.update"
	if st.opened == nil {
		method = "tasks.create"
	}
	st.busy = true
	st.error = ""
	store.TaskRequest(method, params, func(_ model.DurableTask, err error) {
		if st.closed {
			return
		}
		st.busy = false
		if err != nil {
			st.error = model.ErrorText(err)
		} else {
			s.dismiss()
		}
	})
}
func (st *taskSheet) reload() {
	if st.busy || st.opened == nil {
		return
	}
	st.busy = true
	store.TaskRequest("tasks.get", map[string]any{"id": st.opened.ID, "refresh": true}, func(task model.DurableTask, err error) {
		if st.closed {
			return
		}
		st.busy = false
		if err != nil {
			st.error = model.ErrorText(err)
		} else {
			st.load(&task)
		}
	})
}
func (st *taskSheet) run(s *sheet) {
	if st.busy || st.opened == nil {
		return
	}
	if st.savedOwner() == nil {
		st.error = L("Resolve the owner's Runner assignment before running or changing the task budget.")
		return
	}
	chatID := st.runChatID()
	if chatID == "" {
		st.error = L("The task owner needs a linked chat to run in.")
		return
	}
	params := map[string]any{"id": st.opened.ID, "expected_revision": st.opened.Revision, "chat_id": chatID}
	taskRequestKey(params, &st.runID, &st.runBody)
	st.busy = true
	st.error = ""
	store.TaskRequest("tasks.run", params, func(_ model.DurableTask, err error) {
		if st.closed {
			return
		}
		st.busy = false
		if err != nil {
			st.error = model.ErrorText(err)
		} else {
			s.dismiss()
		}
	})
}

func (st *taskSheet) runChatID() string {
	if slices.Contains(st.runChats(), st.runIn) {
		return st.runIn
	}
	return ""
}

func (st *taskSheet) runChats() []string {
	if st.opened == nil {
		return nil
	}
	var selected []string
	for _, id := range st.opened.ChatIDs {
		if chat := store.Chat(id); chat != nil && slices.Contains(chat.BotIDs, st.opened.OwnerBotID) {
			selected = append(selected, id)
		}
	}
	return selected
}
