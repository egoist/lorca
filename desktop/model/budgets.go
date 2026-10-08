package model

import (
	"errors"
	"fmt"
	"math"
	"slices"
	"strconv"
	"strings"
)

// Budgets are Runner-owned projections from the CLI, never a desktop accounting store.
// Pointers preserve the distinction between unlimited and an explicit zero allowance.
type BudgetLimits struct {
	MaxUSD            *float64 `json:"max_usd,omitempty"`
	MaxTokens         *uint64  `json:"max_tokens,omitempty"`
	MaxRuntimeSecs    *uint64  `json:"max_runtime_secs,omitempty"`
	MaxRetries        *uint64  `json:"max_retries,omitempty"`
	MaxConnectorCalls *uint64  `json:"max_connector_calls,omitempty"`
}

type BudgetUsage struct {
	Tokens                  uint64  `json:"tokens"`
	APICostUSD              float64 `json:"api_cost_usd"`
	SubscriptionEstimateUSD float64 `json:"subscription_estimate_usd"`
	UnknownPriceCalls       uint64  `json:"unknown_price_calls"`
	EstimatedCalls          uint64  `json:"estimated_calls"`
	ModelCalls              uint64  `json:"model_calls"`
	RuntimeSecs             float64 `json:"runtime_secs"`
	Retries                 uint64  `json:"retries"`
	ConnectorCalls          uint64  `json:"connector_calls"`
}

type BudgetState struct {
	Kind      string       `json:"kind"`
	ID        string       `json:"id"`
	RunnerID  string       `json:"runner_id"`
	BotID     string       `json:"bot_id"`
	ChatID    string       `json:"chat_id"`
	JobKind   string       `json:"job_kind"`
	TaskID    string       `json:"task_id"`
	Limits    BudgetLimits `json:"limits"`
	Usage     BudgetUsage  `json:"usage"`
	State     string       `json:"state"`
	Reason    string       `json:"reason"`
	UpdatedAt float64      `json:"updated_at"`
}

func (b BudgetState) NeedsRecovery() bool {
	return b.State == "budget_exhausted" || b.State == "interrupted"
}
func (b BudgetState) OwnedAdmission() bool {
	return b.Kind == "task" || b.TaskID != "" || b.JobKind == "event"
}
func (b BudgetState) StateLabel() string {
	switch b.State {
	case "budget_exhausted":
		return L("Budget exhausted")
	case "interrupted":
		return L("Interrupted — resume explicitly")
	case "running":
		return L("Running…")
	case "complete":
		return L("Finished")
	default:
		return L("Ready")
	}
}
func (b BudgetState) Key() string { return b.Kind + ":" + b.ID }

// BudgetFields belong to an editor, so events and rebuilds cannot overwrite a user's draft.
type BudgetFields struct{ USD, Tokens, Runtime, Retries, ConnectorCalls string }

func BudgetFieldsFor(limits BudgetLimits) BudgetFields {
	u := func(value *uint64) string {
		if value == nil {
			return ""
		}
		return strconv.FormatUint(*value, 10)
	}
	usd := ""
	if limits.MaxUSD != nil {
		usd = strconv.FormatFloat(*limits.MaxUSD, 'f', -1, 64)
	}
	return BudgetFields{usd, u(limits.MaxTokens), u(limits.MaxRuntimeSecs), u(limits.MaxRetries), u(limits.MaxConnectorCalls)}
}

func (f BudgetFields) Limits() (BudgetLimits, error) {
	var limits BudgetLimits
	bad := errors.New(L("Use a nonnegative number for each limit, or leave it empty."))
	if text := strings.TrimSpace(f.USD); text != "" {
		usd, err := strconv.ParseFloat(text, 64)
		if err != nil || math.IsNaN(usd) || math.IsInf(usd, 0) || usd < 0 {
			return limits, bad
		}
		limits.MaxUSD = &usd
	}
	for _, field := range []struct {
		text string
		into **uint64
	}{
		{f.Tokens, &limits.MaxTokens}, {f.Runtime, &limits.MaxRuntimeSecs},
		{f.Retries, &limits.MaxRetries}, {f.ConnectorCalls, &limits.MaxConnectorCalls},
	} {
		if text := strings.TrimSpace(field.text); text != "" {
			value, err := strconv.ParseUint(text, 10, 64)
			if err != nil {
				return limits, bad
			}
			*field.into = &value
		}
	}
	if limits.MaxRuntimeSecs != nil && *limits.MaxRuntimeSecs > 31_536_000 {
		return limits, errors.New(L("A runtime allowance is at most one year; leave it unset for unlimited."))
	}
	return limits, nil
}

type BudgetTarget struct{ Kind, ID, RunnerID, BotID, ChatID string }

func (t BudgetTarget) params() map[string]any {
	return map[string]any{"kind": t.Kind, "id": t.ID, "runner_id": t.RunnerID, "bot_id": t.BotID, "chat_id": t.ChatID}
}

func (s *Store) Budget(kind, id, runnerID string) *BudgetState {
	for i := range s.Budgets {
		b := &s.Budgets[i]
		if b.Kind == kind && b.ID == id && b.RunnerID == runnerID {
			return b
		}
	}
	return nil
}

func (s *Store) BudgetsFor(chatID, runnerID string) []BudgetState {
	var out []BudgetState
	for _, b := range s.Budgets {
		if b.ChatID == chatID && b.RunnerID == runnerID {
			out = append(out, b)
		}
	}
	slices.SortStableFunc(out, func(a, b BudgetState) int {
		if a.UpdatedAt > b.UpdatedAt {
			return -1
		}
		if a.UpdatedAt < b.UpdatedAt {
			return 1
		}
		return strings.Compare(a.Key(), b.Key())
	})
	return out
}

func (s *Store) mergeBudgets(values []BudgetState) {
	for _, value := range values {
		old := s.Budget(value.Kind, value.ID, value.RunnerID)
		if old == nil {
			s.Budgets = append(slices.Clone(s.Budgets), value)
		} else if value.UpdatedAt >= old.UpdatedAt {
			*old = value
		}
	}
	s.emit(Event{Kind: EventBudgetsChanged})
}

// ListBudgets reads the assigned Runner before editing, returning through the ordered post queue.
func (s *Store) ListBudgets(runnerID string, done func([]BudgetState, error)) {
	Async(s, func() ([]BudgetState, error) {
		value, err := call[struct {
			Budgets []BudgetState `json:"budgets"`
		}](s, "budgets.list", map[string]any{"runner_id": runnerID})
		return value.Budgets, err
	}, func(values []BudgetState, err error) {
		if err == nil {
			s.mergeBudgets(values)
		}
		if done != nil {
			done(values, err)
		}
	})
}

// ChangeBudget keeps request ordering: save first, then explicit recovery. Ownership/inbox
// admission stays with tasks.run/events.retry; the UI cannot replay those saved Jobs.
func (s *Store) ChangeBudget(target BudgetTarget, limits BudgetLimits, resume, renew, owned bool, requestID string, done func(BudgetState, error)) {
	params := target.params()
	params["limits"] = limits
	Async(s, func() (BudgetState, error) {
		value, err := call[BudgetState](s, "budgets.set", params)
		if err != nil || !resume {
			return value, err
		}
		recovery := target.params()
		recovery["renew"], recovery["run"], recovery["request_id"] = renew, !(owned || target.Kind == "task"), requestID
		return call[BudgetState](s, "budgets.resume", recovery)
	}, func(value BudgetState, err error) {
		if err == nil {
			s.mergeBudgets([]BudgetState{value})
		}
		if done != nil {
			done(value, err)
		}
	})
}

func NewBudgetRequestID() string { return "budget-" + uuid() }

type ConnectorLimits struct {
	MaxCalls       uint64 `json:"max_calls"`
	WindowSecs     uint64 `json:"window_secs"`
	MaxConcurrency uint64 `json:"max_concurrency"`
}
type ConnectorState struct {
	Limits      ConnectorLimits `json:"limits"`
	ActiveCalls uint64          `json:"active_calls"`
	RetryAt     *float64        `json:"retry_at"`
}
type ConnectorFields struct{ Calls, Window, Concurrency string }

func ConnectorFieldsFor(limits ConnectorLimits) ConnectorFields {
	return ConnectorFields{strconv.FormatUint(limits.MaxCalls, 10), strconv.FormatUint(limits.WindowSecs, 10), strconv.FormatUint(limits.MaxConcurrency, 10)}
}
func (f ConnectorFields) Limits() (ConnectorLimits, error) {
	var limits ConnectorLimits
	bad := errors.New(L("Use whole numbers: up to 1,000,000 calls, a window of 1–86,400 seconds, and up to 256 concurrent calls."))
	for _, field := range []struct {
		text string
		into *uint64
	}{{f.Calls, &limits.MaxCalls}, {f.Window, &limits.WindowSecs}, {f.Concurrency, &limits.MaxConcurrency}} {
		value, err := strconv.ParseUint(strings.TrimSpace(field.text), 10, 64)
		if err != nil {
			return limits, bad
		}
		*field.into = value
	}
	if limits.MaxCalls > 1_000_000 || limits.WindowSecs < 1 || limits.WindowSecs > 86_400 || limits.MaxConcurrency > 256 {
		return limits, bad
	}
	return limits, nil
}
func (s *Store) GetConnectorLimits(pluginID, runnerID, scope string, done func(ConnectorState, error)) {
	params := map[string]any{"plugin_id": pluginID, "runner_id": runnerID, "scope": scope}
	Async(s, func() (ConnectorState, error) { return call[ConnectorState](s, "connector_limits.get", params) }, done)
}
func (s *Store) SetConnectorLimits(pluginID, runnerID, scope string, limits ConnectorLimits, done func(ConnectorState, error)) {
	params := map[string]any{"plugin_id": pluginID, "runner_id": runnerID, "scope": scope, "limits": limits}
	Async(s, func() (ConnectorState, error) { return call[ConnectorState](s, "connector_limits.set", params) }, done)
}

func BudgetTokenSummary(tokens uint64) string {
	if tokens > uint64(math.MaxInt) {
		return fmt.Sprint(tokens)
	}
	return Tokens(int(tokens))
}
