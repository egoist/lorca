package model

import (
	"encoding/json"
	"errors"
	"slices"
	"strings"
	"sync"
	"testing"
)

func TestBudgetFieldsPreserveZeroUnlimitedAndRejectInvalidLimits(t *testing.T) {
	limits, err := (BudgetFields{USD: "0", Tokens: "0", Runtime: "", Retries: "3", ConnectorCalls: ""}).Limits()
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(limits)
	if !strings.Contains(string(raw), `"max_usd":0`) || !strings.Contains(string(raw), `"max_tokens":0`) || strings.Contains(string(raw), "max_runtime_secs") {
		t.Fatalf("zero/unlimited lost: %s", raw)
	}
	for _, fields := range []BudgetFields{{USD: "NaN"}, {USD: "Inf"}, {USD: "-0.1"}, {Tokens: "-1"}, {Tokens: "1.5"}, {Runtime: "31536001"}} {
		if _, err := fields.Limits(); err == nil {
			t.Fatalf("accepted %+v", fields)
		}
	}
	if got := BudgetFieldsFor(limits); got.USD != "0" || got.Tokens != "0" || got.Runtime != "" {
		t.Fatalf("round trip %+v", got)
	}
	for _, fields := range []ConnectorFields{{"20", "0", "2"}, {"20", "86401", "2"}, {"1000001", "60", "2"}, {"20", "60", "257"}, {"NaN", "60", "2"}} {
		if _, err := fields.Limits(); err == nil {
			t.Fatalf("accepted connector %+v", fields)
		}
	}
	if got, err := (ConnectorFields{"0", "60", "0"}).Limits(); err != nil || got.MaxConcurrency != 0 {
		t.Fatalf("cannot pause: %+v %v", got, err)
	}
}

func TestBudgetProjectionAndPriceLabelsFromWire(t *testing.T) {
	s := NewStore(nil, func(fn func()) { fn() }, false)
	snapshot := decodeJSON[WireSnapshot](t, `{"budgets":[{"kind":"task","id":"task-1","runner_id":"runner","chat_id":"chat","job_kind":"event","limits":{"max_tokens":0},"usage":{"tokens":15,"unknown_price_calls":2},"state":"budget_exhausted","reason":"Increase the allowance"}]}`)
	s.apply(snapshot)
	b := s.Budget("task", "task-1", "runner")
	if b == nil || !b.NeedsRecovery() || !b.OwnedAdmission() || b.Limits.MaxTokens == nil || *b.Limits.MaxTokens != 0 {
		t.Fatalf("projection %+v", b)
	}
	s.handle("budgets.changed", json.RawMessage(`{"budgets":[{"kind":"routine","id":"rt-1","runner_id":"runner","limits":{},"usage":{},"state":"interrupted"}]}`))
	if s.Budget("task", "task-1", "runner") != nil || !s.Budget("routine", "rt-1", "runner").NeedsRecovery() {
		t.Fatalf("event did not replace projection: %+v", s.Budgets)
	}
	s.handle("budgets.changed", json.RawMessage(`{"budgets":[]}`))
	if len(s.Budgets) != 0 {
		t.Fatal("clear projection ignored")
	}
	unknown := ToUsage(decodeJSON[WireChatUsage](t, `{"cost_usd":0,"turns":1,"priced_calls":1,"pricing_kinds":["unknown"],"unknown_price_calls":1}`))
	if !strings.Contains(unknown.SpendSummary(), "Pricing unknown") || strings.Contains(unknown.SpendSummary(), "$0") {
		t.Fatal(unknown.SpendSummary())
	}
	legacy := ToUsage(WireChatUsage{CostUSD: 1, Turns: 3})
	if !strings.Contains(legacy.SpendSummary(), "Pricing unknown") {
		t.Fatal(legacy.SpendSummary())
	}
	sub := ToUsage(decodeJSON[WireChatUsage](t, `{"turns":1,"priced_calls":1,"pricing_kinds":["subscription_estimate"],"subscription_estimate_usd":0}`))
	if !strings.Contains(sub.SpendSummary(), "API-equivalent estimate $0.00") {
		t.Fatal(sub.SpendSummary())
	}
	api := ToUsage(decodeJSON[WireChatUsage](t, `{"turns":2,"priced_calls":1,"pricing_kinds":["api"],"api_cost_usd":0.004}`))
	if !strings.Contains(api.SpendSummary(), "API <$0.01") {
		t.Fatal(api.SpendSummary())
	}
}

type budgetModelTransport struct {
	mu      sync.Mutex
	methods []string
	params  []map[string]any
	fail    string
}

func (t *budgetModelTransport) Reconnect() {}
func (t *budgetModelTransport) Request(method string, params any) (json.RawMessage, error) {
	raw, _ := json.Marshal(params)
	var value map[string]any
	_ = json.Unmarshal(raw, &value)
	t.mu.Lock()
	defer t.mu.Unlock()
	t.methods = append(t.methods, method)
	t.params = append(t.params, value)
	if method == t.fail {
		return nil, errors.New("Runner is offline")
	}
	return json.RawMessage(`{"kind":"task","id":"task-1","runner_id":"runner","limits":{},"usage":{"tokens":20},"state":"ready"}`), nil
}

func TestBudgetMutationUsesOrderedMainThreadReplyAndOwnedAdmission(t *testing.T) {
	transport := &budgetModelTransport{}
	posted := make(chan func(), 1)
	s := NewStore(transport, func(fn func()) { posted <- fn }, false)
	called := false
	s.ChangeBudget(BudgetTarget{Kind: "task", ID: "task-1", RunnerID: "runner", BotID: "bot", ChatID: "chat"}, BudgetLimits{}, true, true, true, "stable-request", func(value BudgetState, err error) {
		called = true
		if err != nil || value.Usage.Tokens != 20 {
			t.Errorf("reply %+v %v", value, err)
		}
	})
	fn := <-posted
	if called || len(s.Budgets) != 0 {
		t.Fatal("worker touched main-thread state")
	}
	transport.mu.Lock()
	if !slices.Equal(transport.methods, []string{"budgets.set", "budgets.resume"}) {
		t.Errorf("methods %v", transport.methods)
	}
	if transport.params[1]["run"] != false || transport.params[1]["request_id"] != "stable-request" || transport.params[1]["runner_id"] != "runner" {
		t.Errorf("ownership or routing bypass: %+v", transport.params[1])
	}
	transport.mu.Unlock()
	fn()
	if !called || len(s.Budgets) != 1 {
		t.Fatal("reply not applied on main queue")
	}
	transport.fail = "budgets.set"
	s.ChangeBudget(BudgetTarget{Kind: "task", ID: "task-1", RunnerID: "runner"}, BudgetLimits{}, true, false, true, "retry", func(_ BudgetState, err error) {
		if err == nil {
			t.Error("error swallowed")
		}
	})
	(<-posted)()
	transport.mu.Lock()
	defer transport.mu.Unlock()
	if len(transport.methods) != 3 {
		t.Errorf("started recovery after failed save: %v", transport.methods)
	}
}
