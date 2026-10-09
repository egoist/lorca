package main

import (
	"slices"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func taskFixture(id string, state model.TaskState, owner, goal string, updated float64) model.DurableTask {
	runner := store.Bot(owner).RunnerID
	return model.DurableTask{
		ID: id, Revision: 3, AuthorityRunnerID: runner, OwnerBotID: owner, RunnerID: runner, Goal: goal,
		AcceptanceCriteria: []string{"Every download link returns 200", "The guide names the right file"},
		NextAction:         "Ask DevOps to upload the arm64 tarball again", ChatIDs: []string{"chat-relay"}, State: state,
		Dependencies: []string{}, Links: []model.TaskLink{}, Evidence: []model.TaskEvidence{}, CreatedAt: 1, UpdatedAt: updated,
	}
}

// launchTasks are the Launch room's tasks in every state, as the inspector and the sheet show them.
func launchTasks() []model.DurableTask {
	runPosts() // the demo's own snapshot first, so it doesn't replace what follows
	blocked := taskFixture("task-1", model.TaskBlocked, "bot-patch", "Fix the Linux download link", 90)
	reason := "The Linux download returns 404: the release bucket has no arm64 tarball."
	blocked.Reason = &reason
	blocked.Links = []model.TaskLink{{Label: "Linux download 404", URL: "https://github.com/example/site/issues/412"}}
	review := taskFixture("task-2", model.TaskAwaitingReview, "bot-scout", "Review onboarding on Mac and iPhone", 80)
	result := "Setup and pairing are clear on both. One gap: the guide should say that your Mac runs the bots while you chat from your phone."
	review.Result = &result
	notes, chat := "https://docs.example.com/launch/onboarding-notes", "chat-relay"
	var message string
	for _, m := range store.Chat("chat-relay").Messages {
		if m.Author.Kind == model.AuthorBot && m.Author.BotID == "bot-scout" {
			message = m.ID
			break
		}
	}
	// The output the run published for the task, as #87 hands it back.
	reportID, output, version := "msg-onboarding-review-2", "out-onboarding-review", uint64(2)
	relay := store.Chat("chat-relay")
	relay.Messages = append(relay.Messages, &model.Message{ID: reportID, Author: model.BotAuthor("bot-scout"), CreatedAt: time.Now(),
		Body:   model.Body{Kind: model.BodyText, Text: "[Onboarding review](https://docs.example.com/launch/onboarding-review)"},
		Output: &model.Output{ID: output, Name: "Onboarding review", Mime: "text/html", BotID: "bot-scout", Version: 2, URL: "https://docs.example.com/launch/onboarding-review"}})
	review.Evidence = []model.TaskEvidence{
		{Kind: "message", Label: "Bot run result", ChatID: &chat, MessageID: &message},
		{Kind: "output", Label: "Onboarding review", ChatID: &chat, MessageID: &reportID, OutputID: &output, Version: &version},
		{Kind: "url", Label: "Onboarding notes", URL: &notes},
	}
	working := taskFixture("task-3", model.TaskWorking, "bot-nova", "Write the launch announcement", 70)
	working.ActiveRun = &model.TaskRun{ID: "task-run-1", BotID: "bot-nova", RunnerID: working.RunnerID, ChatID: "chat-relay", StartedAt: 1}
	queued := taskFixture("task-4", model.TaskQueued, "bot-nova", "Send the go/no-go summary on Friday", 60)
	queued.Dependencies = []string{"task-1", "task-2"}
	done := taskFixture("task-5", model.TaskCompleted, "bot-patch", "Update the getting-started guide", 50)
	done.Result, done.Evidence = &result, []model.TaskEvidence{{Kind: "url", Label: "Guide", URL: &notes}}
	cancelled := taskFixture("task-6", model.TaskCancelled, "bot-nova", "Draft a press kit", 40)
	why := model.CancelledByUser
	cancelled.Reason = &why
	return []model.DurableTask{blocked, review, working, queued, done, cancelled}
}

func taskSheetTester(t *testing.T, id string) (*mainWindow, *taskSheet, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	for _, task := range launchTasks() {
		store.AcceptDurableTask(task)
	}
	m.open("chat-relay")
	st := m.presentDurableTask("chat-relay", store.DurableTask(id))
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	return m, st, tt
}

// focusField clicks the field beside a form row's label.
func focusField(t *testing.T, tt *ui.Tester, label string) {
	t.Helper()
	r, ok := tt.Find(label)
	if !ok {
		t.Fatalf("no %s: %q", label, tt.Texts())
	}
	tt.ClickAt(r.X+formLabelWidth+60, r.Y+r.H/2)
}

func taskInput(t *testing.T, tt *ui.Tester, label, value string) {
	t.Helper()
	focusField(t, tt, label)
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type(value)
	settle(tt)
}

func TestTaskSectionHidesWhileEmptyAndShowsOpenWorkFirst(t *testing.T) {
	m := demoWindow(t)
	view := func(c *ui.Context) { applyTheme(c); m.inspectorDurableTasks(c, store.Chat("chat-relay")) }
	tt := ui.NewTester(view, 300, 600)
	settle(tt)
	if tt.HasText("TASKS") {
		t.Fatal("an empty Tasks section shows")
	}
	tasks := launchTasks()
	for _, task := range slices.Backward(tasks) {
		store.AcceptDurableTask(task)
	}
	settle(tt)
	var goals []string
	for _, task := range store.TasksIn("chat-relay") {
		goals = append(goals, task.Goal)
	}
	if goals[0] != tasks[0].Goal || goals[1] != tasks[1].Goal || goals[5] != tasks[5].Goal {
		t.Fatalf("order: %q", goals)
	}
	if !tt.HasText("Show 2 More") || tt.HasText("Draft a press kit") || !tt.HasText("Blocked · Developer") {
		t.Fatalf("rows: %q", tt.Texts())
	}
	click(t, tt, "Show 2 More")
	if !tt.HasText("Draft a press kit") || tt.HasText("Show 2 More") {
		t.Fatalf("Show More: %q", tt.Texts())
	}
	renderBoth(t, tt, "desktop-task-list")
}

func TestTaskSheetSavesTheChangedFieldsOverTheOpenedRevision(t *testing.T) {
	m, st, tt := taskSheetTester(t, "task-1")
	taskInput(t, tt, "Next step", "Ask DevOps to upload it, then check the link")
	click(t, tt, "Save")
	task := store.DurableTask("task-1")
	if m.hasSheet() || task.Revision != 4 || task.NextAction != "Ask DevOps to upload it, then check the link" || task.Goal != "Fix the Linux download link" {
		t.Fatalf("save: sheet %v, %+v", m.hasSheet(), task)
	}
	if st.sentID != "" {
		t.Fatal("a finished request keeps its id")
	}
}

func TestTaskSheetStaleSaveKeepsTheUsersEditsOverTheNewerVersion(t *testing.T) {
	m, st, tt := taskSheetTester(t, "task-1")
	taskInput(t, tt, "Next step", "My next step")
	newer := store.DurableTask("task-1").Clone()
	newer.Revision, newer.Goal = 4, "Fix every download link"
	store.AcceptDurableTask(newer)
	settle(tt)
	if st.base.Revision != 3 || st.next != "My next step" {
		t.Fatal("a change from elsewhere replaced an edited form")
	}
	click(t, tt, "Save")
	if !m.hasSheet() || st.base.Revision != 4 || st.goal != "Fix every download link" || st.next != "My next step" || st.failed {
		t.Fatalf("stale save: %+v", st)
	}
	if !tt.HasText(L("This task changed since you opened it. Your edits are still here; save again to keep them.")) {
		t.Fatalf("no conflict note: %q", tt.Texts())
	}
	renderBoth(t, tt, "desktop-task-conflict")
	click(t, tt, "Save")
	if task := store.DurableTask("task-1"); m.hasSheet() || task.NextAction != "My next step" || task.Goal != "Fix every download link" {
		t.Fatalf("second save: %+v", task)
	}
}

func TestTaskSheetUntouchedFormFollowsChanges(t *testing.T) {
	_, st, tt := taskSheetTester(t, "task-1")
	newer := store.DurableTask("task-1").Clone()
	newer.Revision, newer.Goal = 4, "Fix every download link"
	store.AcceptDurableTask(newer)
	settle(tt)
	if st.base.Revision != 4 || st.goal != "Fix every download link" {
		t.Fatalf("untouched form kept the old version: %+v", st)
	}
}

func TestTaskSheetStepsFollowTheState(t *testing.T) {
	m, _, tt := taskSheetTester(t, "task-4")
	if tt.HasText("Start") || !tt.HasText(L("It starts once the tasks it waits for are completed.")) || !tt.HasText("Fix the Linux download link") {
		t.Fatalf("a task waiting on others offers Start: %q", tt.Texts())
	}
	renderBoth(t, tt, "desktop-task-queued")
	m.sheets[0].dismiss()

	m, _, tt = taskSheetTester(t, "task-1")
	renderBoth(t, tt, "desktop-task-blocked")
	click(t, tt, "Resume")
	if task := store.DurableTask("task-1"); m.hasSheet() || task.State != model.TaskWorking || task.ActiveRun == nil || task.ActiveRun.ChatID != "chat-relay" {
		t.Fatalf("Resume: %+v", task)
	}

	m, _, tt = taskSheetTester(t, "task-2")
	renderBoth(t, tt, "desktop-task-review")
	if !tt.HasText("Message from Researcher") || !tt.HasText("docs.example.com") || !tt.HasText("Version 2") {
		t.Fatalf("evidence: %q", tt.Texts())
	}
	var opened string
	outputOpenURL = func(link string) { opened = link }
	// The transcript and the inspector name the same output; its version line is the sheet's own.
	click(t, tt, "Version 2")
	if opened != "https://docs.example.com/launch/onboarding-review" || !m.hasSheet() {
		t.Fatalf("output evidence opened %q", opened)
	}
	click(t, tt, "Mark Complete")
	if task := store.DurableTask("task-2"); m.hasSheet() || task.State != model.TaskCompleted {
		t.Fatalf("Mark Complete: %+v", task)
	}

	m, _, tt = taskSheetTester(t, "task-6")
	if tt.HasText(model.CancelledByUser) || tt.HasText("Cancel Task…") {
		t.Fatalf("cancelled task: %q", tt.Texts())
	}
	click(t, tt, "Reopen")
	if task := store.DurableTask("task-6"); !m.hasSheet() || task.State != model.TaskQueued || task.Reason != nil || !tt.HasText("Start") {
		t.Fatalf("Reopen: %+v", task)
	}

	m, _, tt = taskSheetTester(t, "task-3")
	if tt.HasText("Start") || !tt.HasText("Project Manager is working on it.") {
		t.Fatalf("working: %q", tt.Texts())
	}
	click(t, tt, "Cancel Task…")
	click(t, tt, "Cancel Task")
	if task := store.DurableTask("task-3"); m.hasSheet() || task.State != model.TaskCancelled || task.ReasonText() != model.CancelledByUser {
		t.Fatalf("Cancel Task: %+v", task)
	}
}

func TestNewTaskCreatesItForTheChatAndShowsIt(t *testing.T) {
	m := demoWindow(t)
	m.open("chat-relay")
	st := m.presentDurableTask("chat-relay", nil)
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	if st.owner != "bot-nova" || !tt.HasText("Owner") {
		t.Fatalf("owner: %q", st.owner)
	}
	renderBoth(t, tt, "desktop-task-new")
	taskInput(t, tt, "Goal", "Write the release notes")
	taskInput(t, tt, "Next step", "List what changed since 0.3")
	focusField(t, tt, "Done when")
	tt.Type("Every change is listed")
	tt.Key(0, ui.KeyEnter)
	tt.Type("Links the download page")
	settle(tt)
	if st.base != nil || st.criteria != "Every change is listed\nLinks the download page" {
		t.Fatalf("Return in Done when: %q", st.criteria)
	}
	click(t, tt, "Create Task")
	if !m.hasSheet() || st.base == nil || !tt.HasText("Not started") || !tt.HasText("Start") {
		t.Fatalf("create: %+v %q", st.base, tt.Texts())
	}
	task := store.DurableTask(st.base.ID)
	if task.OwnerBotID != "bot-nova" || !slices.Equal(task.ChatIDs, []string{"chat-relay"}) || len(task.AcceptanceCriteria) != 2 {
		t.Fatalf("created: %+v", task)
	}
}
