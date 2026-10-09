package main

import (
	"encoding/json"
	"errors"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

type limitsRequest struct {
	Method string
	Params map[string]any
}

// limitsTransport answers the limits calls the sheets make and records them.
type limitsTransport struct {
	mu       sync.Mutex
	requests []limitsRequest
}

func (*limitsTransport) Reconnect() {}
func (f *limitsTransport) Request(method string, params any) (json.RawMessage, error) {
	raw, _ := json.Marshal(params)
	var p map[string]any
	_ = json.Unmarshal(raw, &p)
	f.mu.Lock()
	defer f.mu.Unlock()
	f.requests = append(f.requests, limitsRequest{method, p})
	switch method {
	case "budgets.set", "budgets.resume", "connector_limits.set":
		return json.Marshal(map[string]any{})
	case "connector_limits.get":
		return json.Marshal(map[string]any{"plugin_id": p["plugin_id"], "service_id": p["plugin_id"], "limits": map[string]any{"max_calls": 60, "window_secs": 60, "max_concurrency": 4}})
	}
	return nil, errors.New("not in this fixture")
}

func (f *limitsTransport) calls() []limitsRequest {
	f.mu.Lock()
	defer f.mu.Unlock()
	var out []limitsRequest
	for _, request := range f.requests {
		if strings.HasPrefix(request.Method, "budgets.") || strings.HasPrefix(request.Method, "connector_limits.") {
			out = append(out, request)
		}
	}
	return out
}

// limitsFixture is the demo's data over a store that sends its calls to a recording transport.
func limitsFixture(t *testing.T) (*mainWindow, *limitsTransport, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	seed := store
	f := &limitsTransport{}
	store = model.NewStore(f, func(fn func()) { pendingPosts = append(pendingPosts, fn) }, false)
	store.Devices, store.Bots, store.Chats, store.Routines, store.Budgets = seed.Devices, seed.Bots, seed.Chats, seed.Routines, seed.Budgets
	store.Providers, store.Models, store.AutoReview = seed.Providers, seed.Models, seed.AutoReview
	has := true
	store.HasIdentity, store.IsConnected, store.IsStarting = &has, true, false
	m.userWantsInspector = false
	m.selectChat("chat-scout")
	tt := ui.NewTester(m.frame(m.view), 1180, 820)
	settle(tt)
	return m, f, tt
}

// waitCalls runs frames until the transport has `n` limits calls.
func waitCalls(t *testing.T, tt *ui.Tester, f *limitsTransport, n int) []limitsRequest {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for len(f.calls()) < n {
		if time.Now().After(deadline) {
			t.Fatalf("want %d calls, have %+v", n, f.calls())
		}
		time.Sleep(5 * time.Millisecond)
		settle(tt)
	}
	settle(tt)
	return f.calls()
}

// typeInto replaces the text of the field on the row labelled `label`.
func typeInto(t *testing.T, tt *ui.Tester, label, text string) {
	t.Helper()
	row, ok := tt.Find(label)
	if !ok {
		t.Fatalf("no %q in %q", label, tt.Texts())
	}
	tt.ClickAt(row.X+formLabelWidth+10+50, row.Y+row.H/2)
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type(text)
	tt.Frame()
}

func TestLimitsResumeAfterRaisingTheLimitItReached(t *testing.T) {
	m, f, tt := limitsFixture(t)
	m.presentBudget(store.Bot("bot-scout"), "chat-scout", "")
	settleTransitions(tt)
	for _, text := range []string{L("Token limit reached"), L("Used %@ of %@ tokens.", "100,412", "100,000"), L("%@ used", "100,412")} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	typeInto(t, tt, L("Tokens"), "200000")
	if err := tt.Click(L("Resume")); err != nil {
		t.Fatal(err)
	}
	calls := waitCalls(t, tt, f, 3)
	want := []string{"budgets.set chat chat-scout", "budgets.set job job-demo-research", "budgets.resume job job-demo-research"}
	for i, call := range calls {
		if got := call.Method + " " + call.Params["kind"].(string) + " " + call.Params["id"].(string); got != want[i] {
			t.Errorf("call %d is %q, want %q", i, got, want[i])
		}
	}
	if limits := calls[1].Params["limits"].(map[string]any); limits["max_tokens"] != 200000.0 || limits["max_runtime_secs"] != 900.0 {
		t.Errorf("the stopped turn takes the form's limits: %v", limits)
	}
	if resume := calls[2].Params; resume["renew"] != false || resume["run"] != true || resume["request_id"] == "" {
		t.Errorf("resume goes on with what it used: %v", resume)
	}
}

func TestLimitsAsksBeforeGrantingTheLimitsAgain(t *testing.T) {
	m, f, tt := limitsFixture(t)
	m.presentBudget(store.Bot("bot-scout"), "chat-scout", "")
	settleTransitions(tt)
	if err := tt.Click(L("Resume")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if !tt.HasText(L("Resume this turn with fresh limits?")) || len(f.calls()) != 0 {
		t.Fatalf("no question before a fresh allowance: %q, %+v", tt.Texts(), f.calls())
	}
	// The alert's default button is its first, Resume; Return presses it.
	tt.Key(0, ui.KeyEnter)
	calls := waitCalls(t, tt, f, 3)
	if calls[2].Params["renew"] != true {
		t.Errorf("not a fresh allowance: %v", calls[2].Params)
	}
}

func TestLimitsRefusesWhatIsNotANumber(t *testing.T) {
	m, f, tt := limitsFixture(t)
	m.presentBudget(store.Bot("bot-nova"), "chat-nova", "")
	settleTransitions(tt)
	typeInto(t, tt, L("Run time"), "soon")
	if err := tt.Click(L("Save")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText(L("Enter a number, or leave it empty for no limit.")) || len(f.calls()) != 0 {
		t.Errorf("saved a bad limit: %q, %+v", tt.Texts(), f.calls())
	}
}

func TestCallLimitSaves(t *testing.T) {
	m, f, tt := limitsFixture(t)
	m.presentCallLimit("github", "GitHub", store.Device("dev-workbench"), nil)
	waitCalls(t, tt, f, 1)
	settleTransitions(tt)
	if tt.HasText(L("Applies to")) {
		t.Error("a plugin with one account has nothing to choose")
	}
	typeInto(t, tt, L("At once"), "2")
	if err := tt.Click(L("Save")); err != nil {
		t.Fatal(err)
	}
	calls := waitCalls(t, tt, f, 2)
	limits := calls[1].Params["limits"].(map[string]any)
	if calls[1].Method != "connector_limits.set" || calls[1].Params["scope"] != "account" || limits["max_concurrency"] != 2.0 || limits["max_calls"] != 60.0 {
		t.Errorf("saved %+v", calls[1])
	}
}

func TestRenderLimits(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-scout")
	tt := ui.NewTester(m.frame(m.view), 1180, 820)
	settle(tt)
	renderBoth(t, tt, "limits-inspector")
	m.presentBudget(store.Bot("bot-scout"), "chat-scout", "")
	renderBoth(t, tt, "limits-stopped-turn")
	closeSheets(m)
	m.selectChat("chat-nova")
	settle(tt)
	renderBoth(t, tt, "limits-inspector-routines")
	m.presentRoutine("rt-reviews", store.Bot("bot-nova"), m.prefill)
	renderBoth(t, tt, "limits-routine-sheet")
	closeSheets(m)
	m.presentBudget(store.Bot("bot-nova"), "chat-nova", "rt-reviews")
	renderBoth(t, tt, "limits-routine")
	closeSheets(m)
	m.presentPlugin("github", store.Device("dev-workbench"), "bot-nova", "chat-nova")
	renderBoth(t, tt, "limits-plugin")
	m.presentCallLimit("github", "GitHub", store.Device("dev-workbench"), nil)
	renderBoth(t, tt, "limits-call")
	closeSheets(m)
	m.selectChat("chat-scout")
	m.presentDurableTask("chat-scout", stoppedTask())
	renderBoth(t, tt, "limits-task")
}

// stoppedTask is a task of Researcher's its spending limit stopped, in the demo store.
func stoppedTask() *model.DurableTask {
	reason := "Stopped at the spending limit. Raise it in Limits to resume."
	task := &model.DurableTask{ID: "task-demo", Revision: 3, AuthorityRunnerID: "dev-studio", OwnerBotID: "bot-scout", RunnerID: "dev-studio",
		Goal: "Compare five setup guides", AcceptanceCriteria: []string{"A table of what each explains first"}, NextAction: "Read the guides",
		ChatIDs: []string{"chat-scout"}, State: model.TaskBlocked, Reason: &reason}
	usd := 2.0
	store.DurableTasks = append(store.DurableTasks, task)
	store.Budgets = append(store.Budgets, model.BudgetState{Kind: "task", ID: task.ID, RunnerID: "dev-studio", ChatID: "chat-scout",
		Limits: model.BudgetLimits{MaxUSD: &usd}, Usage: model.BudgetUsage{Tokens: 410_000, APICostUSD: 2.01, RuntimeSecs: 1500, ConnectorCalls: 12},
		State: "budget_exhausted", Reached: "usd"})
	return task
}

func TestATaskItsLimitsStoppedResumesInLimits(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-scout")
	tt := ui.NewTester(m.frame(m.view), 1180, 820)
	m.presentDurableTask("chat-scout", stoppedTask())
	settleTransitions(tt)
	if err := tt.Click(L("Resume")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if !tt.HasText(L("Spending limit reached")) || !tt.HasText(L("Used %@ of %@.", "$2.01", "$2.00")) {
		t.Errorf("Resume didn't open the task's Limits: %q", tt.Texts())
	}
}

func closeSheets(m *mainWindow) {
	for len(m.sheets) > 0 {
		m.sheets[len(m.sheets)-1].dismiss()
	}
}
