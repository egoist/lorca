package model

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"

	"github.com/egoist/lorca/desktop/l10n"
)

func TestBudgetFieldsKeepZeroApartFromNoLimit(t *testing.T) {
	l10n.Set("en", "en-US")
	limits, err := (BudgetFields{USD: "0", Tokens: "100,000", Minutes: "15", Retries: "3"}).Limits()
	if err != nil {
		t.Fatal(err)
	}
	raw, _ := json.Marshal(limits)
	if got := string(raw); got != `{"max_usd":0,"max_tokens":100000,"max_runtime_secs":900,"max_retries":3}` {
		t.Fatalf("limits %s", got)
	}
	if got := BudgetFieldsFor(limits); got != (BudgetFields{USD: "0.00", Tokens: "100,000", Minutes: "15", Retries: "3"}) {
		t.Fatalf("fields %+v", got)
	}
	for _, fields := range []BudgetFields{{USD: "NaN"}, {USD: "-0.1"}, {Tokens: "1.5"}, {Retries: "soon"}, {Minutes: "525601"}} {
		var field BudgetFieldError
		if _, err := fields.Limits(); !errors.As(err, &field) {
			t.Errorf("accepted %+v", fields)
		}
	}
}

func TestAStoppedTurnSaysWhichLimitAndResumesOnlyWhenThatLimitWentUp(t *testing.T) {
	l10n.Set("en", "en-US")
	var state BudgetState
	data := `{"kind":"job","id":"job-1","runner_id":"r","chat_id":"c","limits":{"max_tokens":100,"max_runtime_secs":600},"usage":{"tokens":100,"runtime_secs":20},"state":"budget_exhausted","reached":"tokens","updated_at":1}`
	if err := json.Unmarshal([]byte(data), &state); err != nil {
		t.Fatal(err)
	}
	if !state.IsStopped() || state.StoppedTitle() != "Token limit reached" || state.StoppedDetail() != "Used 100 of 100 tokens." {
		t.Fatalf("%q %q", state.StoppedTitle(), state.StoppedDetail())
	}
	n := func(v uint64) *uint64 { return &v }
	for _, c := range []struct {
		limits BudgetLimits
		fits   bool
	}{
		{state.Limits, false},
		{BudgetLimits{MaxTokens: n(100), MaxRuntimeSecs: n(1200)}, false},
		{BudgetLimits{MaxTokens: n(200), MaxRuntimeSecs: n(600)}, true},
		{BudgetLimits{MaxRuntimeSecs: n(600)}, true},
		{BudgetLimits{MaxTokens: n(200), MaxRuntimeSecs: n(10)}, false},
	} {
		if got := state.Fits(c.limits); got != c.fits {
			t.Errorf("Fits(%s) = %v", c.limits.Summary(), got)
		}
	}
}

func TestSpentKeepsAPISpendingEstimatesAndUnknownPricesApart(t *testing.T) {
	l10n.Set("en", "en-US")
	usd := 2.0
	tokens := uint64(200_000)
	for _, c := range []struct{ got, want string }{
		{ChatUsage{Turns: 3, APICostUSD: 0.42, PricingKinds: []string{"api"}}.SpendSummary(), "$0.42 · 3 turns"},
		{ChatUsage{Turns: 3, PricingKinds: []string{"subscription_estimate"}}.SpendSummary(), "$0.00 est. · 3 turns"},
		{ChatUsage{Turns: 3, APICostUSD: 0.03, SubscriptionEstimateUSD: 0.4, PricingKinds: []string{"api", "subscription_estimate"}}.SpendSummary(), "$0.03 + $0.40 est. · 3 turns"},
		{ChatUsage{Turns: 1, UnknownPriceCalls: 2, PricingKinds: []string{"unknown"}}.SpendSummary(), "Price unknown · 1 turn"},
		{ChatUsage{Turns: 3}.SpendSummary(), "Price unknown · 3 turns"},
		{ChatUsage{Turns: 3, APICostUSD: 0.1, UnknownPriceCalls: 1, PricingKinds: []string{"api", "unknown"}}.SpendSummary(), "$0.10 + unknown · 3 turns"},
		{BudgetLimits{}.Summary(), "None"},
		{BudgetLimits{MaxUSD: &usd, MaxTokens: &tokens}.Summary(), "$2.00 · 200k tokens"},
	} {
		if c.got != c.want {
			t.Errorf("%q, want %q", c.got, c.want)
		}
	}
	var limits CallLimits
	limits.Limits.MaxCalls, limits.Limits.WindowSecs = 10, 30
	if got := limits.Summary(); got != "10 every 30 s" {
		t.Errorf("%q", got)
	}
	for _, fields := range []CallLimitFields{{"20", "0", "2"}, {"10001", "60", "2"}, {"20", "60", "257"}, {"x", "60", "2"}} {
		if _, _, _, err := fields.values(); err == nil || !strings.Contains(err.Error(), "10,000") {
			t.Errorf("accepted %+v", fields)
		}
	}
}

func TestStoppedTurnIsTheChatsNewestTurn(t *testing.T) {
	s := NewStore(nil, func(fn func()) { fn() }, false)
	s.Budgets = []BudgetState{
		{Kind: "job", ID: "old", RunnerID: "r", ChatID: "c", State: "budget_exhausted", UpdatedAt: 1},
		{Kind: "job", ID: "new", RunnerID: "r", ChatID: "c", State: "running", UpdatedAt: 2},
	}
	if got := s.StoppedTurn("c", "r"); got != nil {
		t.Errorf("a newer turn replaces the stopped one: %+v", got)
	}
	s.Budgets[1].State = "interrupted"
	if got := s.StoppedTurn("c", "r"); got == nil || got.ID != "new" {
		t.Errorf("%+v", got)
	}
}
