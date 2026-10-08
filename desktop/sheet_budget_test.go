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

type budgetUIRequest struct {
	Method string
	Params map[string]any
}
type budgetUITransport struct {
	mu            sync.Mutex
	requests      []budgetUIRequest
	budgets       []model.BudgetState
	failSet       bool
	holdScope     string
	holdBudget    bool
	hold, started chan struct{}
}

func (*budgetUITransport) Reconnect() {}
func (f *budgetUITransport) Request(method string, params any) (json.RawMessage, error) {
	raw, _ := json.Marshal(params)
	var p map[string]any
	_ = json.Unmarshal(raw, &p)
	f.mu.Lock()
	f.requests = append(f.requests, budgetUIRequest{method, p})
	holdBudget := method == "budgets.list" && f.holdBudget
	if holdBudget || method == "connector_limits.get" && p["scope"] == f.holdScope && f.hold != nil {
		f.holdBudget = false
		hold, started := f.hold, f.started
		f.mu.Unlock()
		if started != nil {
			close(started)
		}
		<-hold
		f.mu.Lock()
	}
	defer f.mu.Unlock()
	switch method {
	case "budgets.list":
		return json.Marshal(map[string]any{"budgets": f.budgets})
	case "budgets.set":
		if f.failSet {
			return nil, errors.New("The Runner is offline. Retry when it reconnects.")
		}
		for i := range f.budgets {
			b := &f.budgets[i]
			if b.Kind == p["kind"] && b.ID == p["id"] {
				r, _ := json.Marshal(p["limits"])
				var limits model.BudgetLimits
				_ = json.Unmarshal(r, &limits)
				b.Limits = limits
				b.UpdatedAt += 1
				return json.Marshal(b)
			}
		}
		return nil, errors.New("Unknown fixture budget")
	case "budgets.resume":
		for i := range f.budgets {
			b := &f.budgets[i]
			if b.Kind == p["kind"] && b.ID == p["id"] {
				b.State, b.Reason = "ready", ""
				b.UpdatedAt += 1
				return json.Marshal(b)
			}
		}
	case "connector_limits.get", "connector_limits.set":
		state := model.ConnectorState{Limits: model.ConnectorLimits{MaxCalls: 20, WindowSecs: 60, MaxConcurrency: 2}, ActiveCalls: 2}
		until := float64(time.Now().Add(5 * time.Minute).Unix())
		state.RetryAt = &until
		if p["scope"] == "service" {
			state.Limits.MaxCalls = 60
			state.Limits.MaxConcurrency = 4
			state.ActiveCalls = 3
			state.RetryAt = nil
		}
		if method == "connector_limits.set" {
			r, _ := json.Marshal(p["limits"])
			_ = json.Unmarshal(r, &state.Limits)
		}
		return json.Marshal(state)
	}
	return nil, errors.New("Read-only fixture: unsupported method")
}
func (f *budgetUITransport) calls() []budgetUIRequest {
	f.mu.Lock()
	defer f.mu.Unlock()
	var calls []budgetUIRequest
	for _, request := range f.requests {
		if strings.HasPrefix(request.Method, "budgets.") || strings.HasPrefix(request.Method, "connector_limits.") {
			calls = append(calls, request)
		}
	}
	return calls
}

type budgetUIQueue struct {
	mu      sync.Mutex
	pending []func()
}

func (q *budgetUIQueue) post(fn func()) {
	q.mu.Lock()
	defer q.mu.Unlock()
	q.pending = append(q.pending, fn)
}
func (q *budgetUIQueue) drain() {
	q.mu.Lock()
	batch := q.pending
	q.pending = nil
	q.mu.Unlock()
	for _, fn := range batch {
		fn()
	}
}

func budgetUIFixture(t *testing.T) (*mainWindow, *budgetUITransport, *budgetUIQueue) {
	t.Helper()
	m := demoWindow(t)
	seed := store
	q := &budgetUIQueue{}
	f := &budgetUITransport{}
	store = model.NewStore(f, q.post, false)
	store.Devices, store.Bots, store.Chats, store.Routines = seed.Devices, seed.Bots, seed.Chats, seed.Routines
	store.Providers, store.Models, store.AutoReview = seed.Providers, seed.Models, seed.AutoReview
	has := true
	store.HasIdentity = &has
	store.IsConnected = true
	store.IsStarting = false
	bot := store.Bot("bot-nova")
	chatID := store.DM(bot.ID)
	usd := 5.0
	tokens := uint64(10000)
	runtime := uint64(600)
	retries := uint64(3)
	calls := uint64(50)
	limits := model.BudgetLimits{MaxUSD: &usd, MaxTokens: &tokens, MaxRuntimeSecs: &runtime, MaxRetries: &retries, MaxConnectorCalls: &calls}
	base := model.BudgetState{Kind: "routine", ID: "rt-reviews", RunnerID: bot.RunnerID, BotID: bot.ID, ChatID: chatID, JobKind: "routine", Limits: limits,
		Usage: model.BudgetUsage{Tokens: 10000, APICostUSD: 2.6, SubscriptionEstimateUSD: 1, UnknownPriceCalls: 2, EstimatedCalls: 1, RuntimeSecs: 590, Retries: 3, ConnectorCalls: 40},
		State: "budget_exhausted", Reason: "Budget exhausted: increase the token allowance or renew it to resume.", UpdatedAt: 10}
	task := base
	task.Kind, task.ID, task.TaskID, task.JobKind = "task", "task-00000000-0000-4000-8000-000000000079", "task-00000000-0000-4000-8000-000000000079", "task"
	task.State, task.Reason = "interrupted", "The Runner restarted during work. Check its completed effects, then explicitly resume."
	f.budgets = []model.BudgetState{base, task}
	store.Budgets = append([]model.BudgetState(nil), f.budgets...)
	m.userWantsInspector = false
	m.selectChat(chatID)
	return m, f, q
}
func budgetWait(t *testing.T, tt *ui.Tester, q *budgetUIQueue, done func() bool) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for !done() {
		q.drain()
		tt.Frame()
		if time.Now().After(deadline) {
			t.Fatalf("fixture timeout: %q", tt.Texts())
		}
		time.Sleep(2 * time.Millisecond)
	}
	q.drain()
	tt.Frame()
}
func budgetEdit(t *testing.T, tt *ui.Tester, label, value string) {
	t.Helper()
	row, ok := tt.Find(label)
	if !ok {
		t.Fatalf("no field %q", label)
	}
	// The caption and field share an accessible label. Click the editable end of the
	// row, aligned with the sheet's trailing Save button, rather than its caption.
	save, ok := tt.Find("Save")
	if !ok {
		t.Fatal("no sheet Save button")
	}
	tt.ClickAt(save.X+save.W-40, row.Y+row.H/2)
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type(value)
	tt.Frame()
}

func TestDesktopBudgetRecoveryFieldsAndActions(t *testing.T) {
	m, f, q := budgetUIFixture(t)
	bot := store.Bot("bot-nova")
	m.presentBudget(bot, store.DM(bot.ID), "rt-reviews", "")
	tt := ui.NewTester(m.frame(m.view), 1180, 850)
	budgetWait(t, tt, q, func() bool { return tt.HasText("Budget exhausted") && tt.HasText("$2.6000") })
	for _, text := range []string{"API spending", "$2.6000", "Subscription API-equivalent estimate", "$1.0000", "Unknown pricing", "2 calls", "Save and resume", "Renew allowance and resume…"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	settleTransitions(tt)
	renderTo(t, tt, "desktop-routine-budget-exhausted")
	budgetEdit(t, tt, "Total tokens", "20000")
	for range 4 {
		tt.Frame()
	}
	if len(f.calls()) != 1 {
		t.Fatalf("draft reset by rebuild %q", tt.Texts())
	}
	if err := tt.Click("Save and resume"); err != nil {
		t.Fatal(err)
	}
	budgetWait(t, tt, q, func() bool { return !m.hasSheet() })
	reqs := f.calls()
	if len(reqs) != 3 || reqs[1].Method != "budgets.set" || reqs[2].Method != "budgets.resume" {
		t.Fatalf("actions %+v", reqs)
	}
	limits := reqs[1].Params["limits"].(map[string]any)
	if limits["max_tokens"] != float64(20000) || reqs[2].Params["run"] != true || reqs[2].Params["renew"] != false || reqs[2].Params["request_id"] == "" {
		t.Fatalf("wrong quota/recovery %+v", reqs)
	}
	if store.Budget("routine", "rt-reviews", bot.RunnerID).Usage.Tokens != 10000 {
		t.Fatal("increase erased consumption")
	}
	// The canonical Task constructor uses the existing owner/ID and refuses raw replay.
	m.presentTaskBudget(bot, store.DM(bot.ID), f.budgets[1].ID)
	budgetWait(t, tt, q, func() bool { return tt.HasText("Interrupted — resume explicitly") })
	settleTransitions(tt)
	renderTo(t, tt, "desktop-task-budget-interrupted")
	if err := tt.Click("Save and resume"); err != nil {
		t.Fatal(err)
	}
	budgetWait(t, tt, q, func() bool {
		return tt.HasText("Allowance recovered. Retry the delivery in Events or run the task again in Tasks so its ownership and inbox admission are checked.")
	})
	reqs = f.calls()
	last := reqs[len(reqs)-1]
	if last.Method != "budgets.resume" || last.Params["run"] != false || !m.hasSheet() {
		t.Fatalf("bypassed task ownership: %+v", last)
	}
}

func TestDesktopBudgetSaveFailureAndRenewConfirmation(t *testing.T) {
	m, f, q := budgetUIFixture(t)
	bot := store.Bot("bot-nova")
	m.presentBudget(bot, store.DM(bot.ID), "rt-reviews", "")
	tt := ui.NewTester(m.frame(m.view), 1180, 850)
	budgetWait(t, tt, q, func() bool { return tt.HasText("$2.6000") })
	budgetEdit(t, tt, "Spending (USD)", "NaN")
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if len(f.calls()) != 1 {
		t.Fatal("invalid allowance reached CLI")
	}
	budgetEdit(t, tt, "Spending (USD)", "5")
	f.mu.Lock()
	f.failSet = true
	f.mu.Unlock()
	if err := tt.Click("Save and resume"); err != nil {
		t.Fatal(err)
	}
	budgetWait(t, tt, q, func() bool { return tt.HasText("The Runner is offline. Retry when it reconnects.") })
	if !m.hasSheet() || len(f.calls()) != 2 {
		t.Fatal("failed save dismissed or resumed work")
	}
	f.mu.Lock()
	f.failSet = false
	f.mu.Unlock()
	if err := tt.Click("Renew allowance and resume…"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("Renew this allowance?") {
		t.Fatal(tt.Texts())
	}
	if len(f.calls()) != 2 {
		t.Fatal("renewed without confirmation")
	}
	if err := tt.Click("Renew and resume"); err != nil {
		t.Fatal(err)
	}
	budgetWait(t, tt, q, func() bool { return !m.hasSheet() })
	last := f.calls()
	if last[len(last)-1].Params["renew"] != true {
		t.Fatal("explicit renew missing")
	}
}

func TestDesktopConnectorScopeDraftsAndStaleReplies(t *testing.T) {
	m, f, q := budgetUIFixture(t)
	runner := store.Device("dev-workbench")
	m.presentConnectorLimits("github", runner)
	tt := ui.NewTester(m.frame(m.view), 1180, 850)
	budgetWait(t, tt, q, func() bool { return !tt.HasText("Loading…") && tt.HasText("Service cooldown until") })
	settleTransitions(tt)
	renderTo(t, tt, "desktop-connector-account-cooldown")
	budgetEdit(t, tt, "Calls per window", "25")
	if err := tt.Click("Account and service"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if err := tt.ChooseMenuItem("All accounts for this service"); err != nil {
		t.Fatal(err)
	}
	budgetWait(t, tt, q, func() bool {
		return tt.HasText("3 calls active. Zero call or concurrency capacity pauses calls.")
	})
	settleTransitions(tt)
	renderTo(t, tt, "desktop-connector-service")
	if err := tt.Click("Account and service"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if err := tt.ChooseMenuItem("This account"); err != nil {
		t.Fatal(err)
	}
	budgetWait(t, tt, q, func() bool { return !tt.HasText("Loading…") && tt.HasText("This account") })
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	budgetWait(t, tt, q, func() bool { return !m.hasSheet() })
	last := f.calls()
	req := last[len(last)-1]
	limits := req.Params["limits"].(map[string]any)
	if req.Method != "connector_limits.set" || req.Params["scope"] != "account" || limits["max_calls"] != float64(25) || req.Params["runner_id"] != runner.ID {
		t.Fatalf("draft/routing lost %+v", req)
	}
	// A delayed account response cannot cover a newer service response or a closed sheet.
	f.mu.Lock()
	f.holdScope = "account"
	f.hold = make(chan struct{})
	f.started = make(chan struct{})
	f.mu.Unlock()
	s := &connectorLimitSheet{w: &m.appWindow, pluginID: "github", runnerID: runner.ID, runnerName: runner.Name, scope: "account", drafts: map[string]model.ConnectorFields{}}
	s.load()
	<-f.started
	s.selectScope("service")
	budgetWait(t, tt, q, func() bool { return s.readable && s.scope == "service" })
	close(f.hold)
	time.Sleep(10 * time.Millisecond)
	q.drain()
	if s.fields.Calls != "60" {
		t.Fatalf("stale reply replaced service %v", s.fields)
	}
	s.closed = true
	s.load()
	time.Sleep(10 * time.Millisecond)
	q.drain()
	if s.fields.Calls != "60" {
		t.Fatal("closed sheet changed")
	}
}

func TestDesktopBudgetAffordances(t *testing.T) {
	m, _, q := budgetUIFixture(t)
	bot := store.Bot("bot-nova")
	m.presentRoutine("rt-reviews", bot, nil)
	tt := ui.NewTester(m.frame(m.view), 1180, 850)
	tt.Frame()
	if !tt.HasText("Budget exhausted") || !tt.HasText("BUDGET") {
		t.Fatal(tt.Texts())
	}
	// The native routine sheet preserves its schedule/task sections and adds Manage.
	if err := tt.Click("Manage…"); err != nil {
		t.Fatal(err)
	}
	budgetWait(t, tt, q, func() bool { return tt.HasText("Total tokens") && tt.HasText("$2.6000") })
	if len(m.sheets) != 2 {
		t.Fatal("routine affordance did not stack budget sheet")
	}
	m.sheets[len(m.sheets)-1].dismiss()
	m.sheets[len(m.sheets)-1].dismiss()
	m.presentPlugin("github", store.Device(bot.RunnerID))
	tt.Frame()
	if !tt.HasText("Shared connector limits") {
		t.Fatal(tt.Texts())
	}
	if err := tt.Click("Manage…"); err != nil {
		t.Fatal(err)
	}
	budgetWait(t, tt, q, func() bool { return tt.HasText("Calls per window") && !tt.HasText("Loading…") })
}

func TestDesktopBudgetScopesKeepDraftsAndIgnoreStaleReplies(t *testing.T) {
	m, f, q := budgetUIFixture(t)
	bot := store.Bot("bot-nova")
	chatID := store.DM(bot.ID)
	f.holdBudget, f.hold, f.started = true, make(chan struct{}), make(chan struct{})
	tokens := uint64(2500)
	f.budgets[1].Limits.MaxTokens = &tokens
	s := &budgetSheet{w: &m.appWindow, botName: bot.Name, selected: "routine:rt-reviews", drafts: map[string]model.BudgetFields{}, choices: []budgetChoice{
		{model.BudgetTarget{Kind: "routine", ID: "rt-reviews", RunnerID: bot.RunnerID, BotID: bot.ID, ChatID: chatID}, "Review requests"},
		{model.BudgetTarget{Kind: "task", ID: f.budgets[1].ID, RunnerID: bot.RunnerID, BotID: bot.ID, ChatID: chatID}, "Task allowance"},
	}}
	s.load()
	m.present(s.view, func() { s.closed = true })
	tt := ui.NewTester(m.frame(m.view), 1180, 850)
	<-f.started
	if s.readable || s.fields.Tokens != "" {
		t.Fatal("edits enabled before assigned Runner answers")
	}
	s.selectScope("task:" + f.budgets[1].ID)
	budgetWait(t, tt, q, func() bool { return s.readable })
	budgetEdit(t, tt, "Total tokens", "2600")
	if s.fields.Tokens != "2600" {
		t.Fatal("field did not bind to persistent state")
	}
	close(f.hold)
	budgetWait(t, tt, q, func() bool { return !s.loading && len(store.Budgets) == 2 })
	// Wait for the rejected older scope reply to reach the same ordered queue.
	time.Sleep(10 * time.Millisecond)
	q.drain()
	tt.Frame()
	if s.loaded.Kind != "task" || s.fields.Tokens != "2600" {
		t.Fatal("stale routine reply overwrote task draft")
	}
	s.selectScope("routine:rt-reviews")
	budgetWait(t, tt, q, func() bool { return s.readable })
	s.selectScope("task:" + f.budgets[1].ID)
	budgetWait(t, tt, q, func() bool { return s.readable })
	if s.fields.Tokens != "2600" {
		t.Fatal("scope change erased draft")
	}
	// Live consumption changes the display, preserving the edited allowance.
	live := *store.Budget("task", f.budgets[1].ID, bot.RunnerID)
	live.Usage.Tokens = 2601
	live.UpdatedAt = 30
	store.Budgets[1] = live
	tt.Frame()
	if s.current().Usage.Tokens != 2601 || s.fields.Tokens != "2600" {
		t.Fatal("event overwrote editor state")
	}
	s.closed = true
	s.load()
	time.Sleep(10 * time.Millisecond)
	q.drain()
	if s.fields.Tokens != "2600" {
		t.Fatal("reply mutated closed sheet")
	}
}

func TestDesktopBudgetInspectorAffordanceAndPricing(t *testing.T) {
	m, _, q := budgetUIFixture(t)
	bot := store.Bot("bot-nova")
	chat := store.Chat(store.DM(bot.ID))
	chat.Usage = &model.ChatUsage{Turns: 3, PricedCalls: 3, APICostUSD: 0.03, SubscriptionEstimateUSD: 0.40, UnknownPriceCalls: 1, PricingKinds: []string{"api", "subscription_estimate", "unknown"}}
	m.userWantsInspector = true
	tt := ui.NewTester(m.frame(m.view), 1180, 850)
	tt.Frame()
	for _, label := range []string{"API $0.03", "API-equivalent estimate $0.40", "Pricing unknown", "Budget", "Interrupted — resume explicitly"} {
		if !tt.HasText(label) {
			t.Fatalf("missing inspector %q", label)
		}
	}
	settleTransitions(tt)
	renderTo(t, tt, "desktop-inspector-pricing-recovery")
	if err := tt.Click("Manage…"); err != nil {
		t.Fatal(err)
	}
	budgetWait(t, tt, q, func() bool { return tt.HasText("Budget limits") && !tt.HasText("Loading…") })
	if !tt.HasText("Interrupted — resume explicitly") {
		t.Fatal("inspector did not select held task")
	}
}
