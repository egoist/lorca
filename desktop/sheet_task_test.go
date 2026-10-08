package main

import (
	"encoding/json"
	"reflect"
	"strings"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func desktopTaskFixture(state model.TaskState) model.DurableTask {
	reason, result := "Runner restarted during this run. Inspect its effects before requeueing.", "Demo smoke checks passed; review the linked summary."
	url := "https://example.invalid/evidence/smoke-checks"
	task := model.DurableTask{ID: "task-00000000-0000-0000-0000-000000000001", Revision: 7, AuthorityRunnerID: "dev-workbench", OwnerBotID: "bot-nova", RunnerID: "dev-workbench", Goal: "Check durable work", AcceptanceCriteria: []string{"Ownership persists", "Evidence is recorded"}, NextAction: "Review the result", ChatIDs: []string{"chat-relay", "chat-patch"}, State: state, Dependencies: []string{}, Links: []model.TaskLink{}, Evidence: []model.TaskEvidence{}, CreatedAt: 100, UpdatedAt: 107}
	if state == model.TaskBlocked {
		task.Reason = &reason
	}
	if state == model.TaskAwaitingReview || state == model.TaskCompleted {
		task.Result = &result
		task.Evidence = []model.TaskEvidence{{Kind: "url", Label: "Demo smoke-check summary", URL: &url}}
	}
	return task
}

func taskSheetTester(t *testing.T, state model.TaskState) (*mainWindow, *taskSheet, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	task := desktopTaskFixture(state)
	store.AcceptDurableTask(task)
	st := m.presentDurableTask("chat-relay", &task)
	tt := ui.NewTester(m.frame(m.view), 1180, 850)
	settle(tt)
	return m, st, tt
}

func taskInput(t *testing.T, tt *ui.Tester, label, value string) {
	t.Helper()
	if err := tt.Click(label); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type(value)
	settle(tt)
}

func TestTaskEditorKeepsDraftAcrossFramesAndOwnerChatSelection(t *testing.T) {
	_, st, tt := taskSheetTester(t, model.TaskQueued)
	taskInput(t, tt, "Task goal", "Edited persistent goal")
	for range 15 {
		tt.Frame()
	}
	if st.goal != "Edited persistent goal" {
		t.Fatalf("draft lost: %q", st.goal)
	}
	if err := tt.Click("Task owner"); err != nil {
		t.Fatal(err)
	}
	if err := tt.ChooseMenuItem("Developer · Studio"); err != nil {
		t.Fatalf("owner menu %q: %v", tt.Menu(), err)
	}
	settle(tt)
	if st.owner != "bot-patch" || st.goal != "Edited persistent goal" {
		t.Fatalf("owner change lost draft: %+v", st)
	}
	if !tt.HasText(L("Assigned Runner: %@", "Studio")) {
		t.Fatal("Runner did not follow owner")
	}
	if err := tt.Click("Remove chat Launch room"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if len(st.chats) != 1 || st.chats[0] != "chat-patch" {
		t.Fatalf("chats: %q", st.chats)
	}
	if err := tt.Click("Add linked chat"); err != nil {
		t.Fatal(err)
	}
	if err := tt.ChooseMenuItem("Launch room"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if len(st.chats) != 2 || st.goal != "Edited persistent goal" {
		t.Fatal("linked chat change lost fields")
	}
	params := st.parameters()
	if params["owner_bot_id"] != "bot-patch" || params["runner_id"] != "dev-studio" || params["expected_revision"] != uint64(7) {
		t.Fatalf("ownership params: %#v", params)
	}
	if !reflect.DeepEqual(params["chat_ids"], []string{"chat-patch", "chat-relay"}) || !reflect.DeepEqual(st.opened.ChatIDs, []string{"chat-relay", "chat-patch"}) {
		t.Fatalf("chat draft changed the opened scope or lost the edit: %#v", params)
	}
}

func TestTaskEditorStaleRevisionKeepsFormAndReloadsAuthority(t *testing.T) {
	m, st, tt := taskSheetTester(t, model.TaskQueued)
	taskInput(t, tt, "Task goal", "My unsaved goal")
	fresh := desktopTaskFixture(model.TaskQueued)
	fresh.Revision = 8
	fresh.Goal = "New authoritative goal"
	fresh.OwnerBotID = "bot-patch"
	fresh.RunnerID = "dev-studio"
	store.AcceptDurableTask(fresh)
	if st.opened.Revision != 7 || st.goal != "My unsaved goal" {
		t.Fatal("event clobbered draft")
	}
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !m.hasSheet() || !strings.Contains(st.error, "revision conflict") || st.goal != "My unsaved goal" {
		t.Fatalf("stale edit: %+v", st)
	}
	renderBoth(t, tt, "desktop-task-stale")
	if err := tt.Click("Reload"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if st.opened.Revision != 8 || st.goal != fresh.Goal || st.owner != "bot-patch" {
		t.Fatalf("reload: %+v", st)
	}
	if !tt.HasText("Developer · Studio") {
		t.Fatal("stale owner after Reload")
	}
	renderBoth(t, tt, "desktop-task-reloaded")
}

func TestTaskRequestsRetainNonceAndDoNotClearUntouchedReferences(t *testing.T) {
	_, st, _ := taskSheetTester(t, model.TaskQueued)
	output, chat, msg := "out-series", "chat-relay", "message-version"
	version := uint64(2)
	st.opened.Evidence = []model.TaskEvidence{{Kind: "output", Label: "Result", ChatID: &chat, MessageID: &msg, OutputID: &output, Version: &version}}
	st.evidence = st.opened.Clone().Evidence
	params := st.parameters()
	if _, ok := params["evidence"]; ok {
		t.Fatal("unchanged output evidence rewritten")
	}
	taskRequestKey(params, &st.requestID, &st.requestBody)
	first := st.requestID
	params = st.parameters()
	taskRequestKey(params, &st.requestID, &st.requestBody)
	if first != st.requestID {
		t.Fatal("retry changed request id")
	}
	st.next = "Another step"
	params = st.parameters()
	taskRequestKey(params, &st.requestID, &st.requestBody)
	if first == st.requestID {
		t.Fatal("different edit reused request id")
	}
	st.evidence = nil
	params = st.parameters()
	encoded, _ := json.Marshal(params)
	if !strings.Contains(string(encoded), `"evidence":[]`) {
		t.Fatalf("clearing evidence must be an array: %s", encoded)
	}
	st.dependencies = nil
	st.opened.Dependencies = []string{"dependency"}
	params = st.parameters()
	encoded, _ = json.Marshal(params)
	if !strings.Contains(string(encoded), `"dependencies":[]`) {
		t.Fatalf("clearing dependencies must be an array: %s", encoded)
	}
}

func TestTaskRunUsesSavedOwnerMemberChatAndClosedRepliesDoNotReopen(t *testing.T) {
	m, st, tt := taskSheetTester(t, model.TaskBlocked)
	task := st.opened.Clone()
	task.Revision++
	task.OwnerBotID = "bot-patch"
	task.RunnerID = "dev-studio"
	st.chatID = "chat-nova"
	task.ChatIDs = []string{"chat-nova", "chat-patch"}
	store.AcceptDurableTask(task)
	st.load(&task)
	if st.runChatID() != "chat-patch" {
		t.Fatal("evidence-only chat chosen for run")
	}
	st.goal = "unsaved goal"
	st.run(m.sheets[0])
	settle(tt)
	if m.hasSheet() {
		t.Fatal("successful run did not close sheet")
	}
	if got := store.DurableTask(st.opened.ID); got.Goal == "unsaved goal" || got.State != model.TaskWorking {
		t.Fatalf("run used draft: %+v", got)
	}
	m, st, tt = taskSheetTester(t, model.TaskQueued)
	st.reload()
	m.sheets = nil
	st.closed = true
	settle(tt)
	if !st.closed {
		t.Fatal("late reply reopened dismissed sheet")
	}
}

func TestTaskPrimaryChatSelectionKeepsEvidenceChatOutOfAdmission(t *testing.T) {
	_, st, _ := taskSheetTester(t, model.TaskQueued)
	st.opened.ChatIDs = []string{"chat-relay", "chat-nova", "chat-patch"}
	tt := ui.NewTester(st.w.frame(func(c *ui.Context) { ui.Box(c).Size(1, 1) }), 1000, 1450)
	settle(tt)
	if err := tt.Click("Primary run chat"); err != nil {
		t.Fatal(err)
	}
	if err := tt.ChooseMenuItem(taskChatTitle("chat-nova")); err != nil {
		t.Fatalf("primary chats %q: %v", tt.Menu(), err)
	}
	settle(tt)
	if st.runChatID() != "chat-nova" {
		t.Fatalf("primary chat: %q", st.runChatID())
	}
	st.runIn = "chat-patch"
	if st.runChatID() != "" {
		t.Fatal("evidence-only teammate DM admitted as the owner's primary chat")
	}
}

func TestTaskEditorEvidencePickerAndValidation(t *testing.T) {
	_, st, tt := taskSheetTester(t, model.TaskAwaitingReview)
	// Use a tall native test frame so evidence controls are visible without platform scrolling.
	tt = ui.NewTester(st.w.frame(func(c *ui.Context) { ui.Box(c).Size(1, 1) }), 1000, 1450)
	settle(tt)
	if err := tt.Click("Remove evidence Demo smoke-check summary"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if len(st.evidence) != 0 {
		t.Fatal("existing evidence not removed from the draft")
	}
	taskInput(t, tt, "Evidence URL", "https://example.invalid/new-proof")
	if err := tt.Click("Add link"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if len(st.evidence) != 1 || st.evidenceURL != "" {
		t.Fatal("link evidence not added")
	}
	taskInput(t, tt, "Evidence URL", "http://example.invalid/insecure")
	if err := tt.Click("Add link"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if len(st.evidence) != 1 || st.error == "" {
		t.Fatal("invalid link accepted")
	}
	if !reflect.DeepEqual(st.opened.Evidence, desktopTaskFixture(model.TaskAwaitingReview).Evidence) {
		t.Fatal("draft evidence mutated opened record")
	}
	renderBoth(t, tt, "desktop-task-evidence")
}

func TestTaskCreateUsesCanonicalAPIFieldsAndCompletionNeedsEvidence(t *testing.T) {
	m := demoWindow(t)
	st := m.presentDurableTask("chat-relay", nil)
	tt := ui.NewTester(m.frame(m.view), 1180, 1450)
	settle(tt)
	taskInput(t, tt, "Task goal", "A new durable goal")
	taskInput(t, tt, "Acceptance criteria", "A verified result")
	taskInput(t, tt, "Task next action", "Perform the smoke check")
	createdID := st.creationID
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	created := store.DurableTask(createdID)
	if created == nil || created.State != model.TaskQueued || created.Goal != "A new durable goal" || created.RunnerID != "dev-workbench" || m.hasSheet() {
		t.Fatalf("create: %+v, error: %q, next action: %q", created, st.error, st.next)
	}
	st = m.presentDurableTask("chat-relay", created)
	settle(tt)
	if err := tt.Click("Task state"); err != nil {
		t.Fatal(err)
	}
	if err := tt.ChooseMenuItem("Completed"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	taskInput(t, tt, "Task result", "Verified result")
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !m.hasSheet() || !strings.Contains(st.error, "evidence") {
		t.Fatal("completion without evidence accepted")
	}
	taskInput(t, tt, "Evidence URL", "https://example.invalid/proof")
	if err := tt.Click("Add link"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.DurableTask(createdID).State != model.TaskCompleted || m.hasSheet() {
		t.Fatal("completion with evidence not saved")
	}
}

func TestTaskBudgetAdapterUsesCanonicalOwnerRunnerAndPrimaryChat(t *testing.T) {
	_, st, tt := taskSheetTester(t, model.TaskQueued)
	prior := taskBudgetOpener
	defer func() { taskBudgetOpener = prior }()
	var owner, runner, chat, taskID string
	taskBudgetOpener = func(_ *appWindow, bot *model.Bot, scope, id string) {
		owner, runner, chat, taskID = bot.ID, bot.RunnerID, scope, id
	}
	settle(tt)
	if err := tt.Click("Budget…"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if owner != st.opened.OwnerBotID || runner != st.opened.RunnerID || chat != "chat-relay" || taskID != st.opened.ID {
		t.Fatalf("budget scope: %q %q %q %q", owner, runner, chat, taskID)
	}
}

func TestTaskRunnerMismatchDoesNotRetargetBudgetOrUnrelatedEdits(t *testing.T) {
	m, st, tt := taskSheetTester(t, model.TaskQueued)
	store.Bot(st.opened.OwnerBotID).RunnerID = "another-runner"
	st.goal = "A progress edit"
	if _, changed := st.parameters()["runner_id"]; changed {
		t.Fatal("unrelated edit silently moved the task Runner")
	}
	if st.savedOwner() != nil {
		t.Fatal("mismatched task admitted to the new Runner's budget")
	}
	prior := taskBudgetOpener
	defer func() { taskBudgetOpener = prior }()
	called := false
	taskBudgetOpener = func(*appWindow, *model.Bot, string, string) { called = true }
	settle(tt)
	_ = tt.Click("Budget…")
	settle(tt)
	st.run(m.sheets[0])
	if called || st.busy || !strings.Contains(st.error, "Runner assignment") {
		t.Fatalf("mismatch dispatched work or accounting: %+v", st)
	}
	store.Bot(st.opened.OwnerBotID).RunnerID = st.opened.RunnerID
	newer := st.opened.Clone()
	newer.Revision++
	newer.OwnerBotID, newer.RunnerID = "bot-patch", "dev-studio"
	store.AcceptDurableTask(newer)
	if st.savedOwner() != nil {
		t.Fatal("stale editor retargeted an allowance after task ownership changed")
	}
}

func TestRenderDurableTaskDesktopStates(t *testing.T) {
	m := demoWindow(t)
	for i, state := range model.TaskStates() {
		task := desktopTaskFixture(state)
		task.ID = model.TaskID("task-")
		task.Goal = state.Title()
		task.UpdatedAt = float64(100 + i)
		store.AcceptDurableTask(task)
	}
	tt := ui.NewTester(func(c *ui.Context) { applyTheme(c); m.inspectorDurableTasks(c, store.Chat("chat-relay")) }, 360, 600)
	settle(tt)
	for _, state := range model.TaskStates() {
		if !tt.HasText(state.Title()) {
			t.Errorf("missing state %s: %q", state, tt.Texts())
		}
	}
	renderBoth(t, tt, "desktop-task-list")
	for _, state := range []model.TaskState{model.TaskBlocked, model.TaskAwaitingReview} {
		m, _, tt = taskSheetTester(t, state)
		tt = ui.NewTester(m.frame(m.view), 1180, 1450)
		settle(tt)
		renderBoth(t, tt, "desktop-task-"+string(state))
	}
}
