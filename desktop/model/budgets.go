package model

import (
	"errors"
	"fmt"
	"math"
	"strconv"
	"strings"
	"time"
)

// BudgetLimits is what a turn, or a routine's runs, may use on the bot's Runner. A nil limit is
// no limit; zero is a limit of nothing.
type BudgetLimits struct {
	MaxUSD            *float64 `json:"max_usd,omitempty"`
	MaxTokens         *uint64  `json:"max_tokens,omitempty"`
	MaxRuntimeSecs    *uint64  `json:"max_runtime_secs,omitempty"`
	MaxRetries        *uint64  `json:"max_retries,omitempty"`
	MaxConnectorCalls *uint64  `json:"max_connector_calls,omitempty"`
}

// Summary is "$2.00 · 200k tokens", or None.
func (l BudgetLimits) Summary() string {
	var parts []string
	if l.MaxUSD != nil {
		parts = append(parts, Dollars(*l.MaxUSD))
	}
	if l.MaxTokens != nil {
		parts = append(parts, L("%@ tokens", tokenCount(*l.MaxTokens)))
	}
	if l.MaxRuntimeSecs != nil {
		parts = append(parts, Minutes(int(*l.MaxRuntimeSecs)))
	}
	if l.MaxRetries != nil {
		if *l.MaxRetries == 1 {
			parts = append(parts, L("1 retry"))
		} else {
			parts = append(parts, L("%d retries", *l.MaxRetries))
		}
	}
	if l.MaxConnectorCalls != nil {
		if *l.MaxConnectorCalls == 1 {
			parts = append(parts, L("1 plugin call"))
		} else {
			parts = append(parts, L("%d plugin calls", *l.MaxConnectorCalls))
		}
	}
	if len(parts) == 0 {
		return Lc("None", "limits")
	}
	return strings.Join(parts, " · ")
}

type BudgetUsage struct {
	Tokens                  uint64  `json:"tokens"`
	APICostUSD              float64 `json:"api_cost_usd"`
	SubscriptionEstimateUSD float64 `json:"subscription_estimate_usd"`
	UnknownPriceCalls       uint64  `json:"unknown_price_calls"`
	RuntimeSecs             float64 `json:"runtime_secs"`
	Retries                 uint64  `json:"retries"`
	ConnectorCalls          uint64  `json:"connector_calls"`
}

func (u BudgetUsage) Dollars() float64 { return u.APICostUSD + u.SubscriptionEstimateUSD }

// BudgetState is one allowance as its Runner keeps it: the limits, what the work used of them,
// and whether it stopped. `chat` is the limits each new turn in a DM starts with; `job` is one
// turn; `routine` all runs of a routine.
type BudgetState struct {
	Kind     string       `json:"kind"`
	ID       string       `json:"id"`
	RunnerID string       `json:"runner_id"`
	ChatID   string       `json:"chat_id"`
	Limits   BudgetLimits `json:"limits"`
	Usage    BudgetUsage  `json:"usage"`
	// State is ready, running, complete, budget_exhausted, or interrupted.
	State string `json:"state"`
	// Reached is the limit it stopped at: usd, tokens, runtime, retries, connector_calls, or
	// unknown_price.
	Reached   string  `json:"reached"`
	UpdatedAt float64 `json:"updated_at"`
}

func (b BudgetState) IsStopped() bool {
	return b.State == "budget_exhausted" || b.State == "interrupted"
}

// StoppedLabel is the inspector's word for a stopped allowance.
func (b BudgetState) StoppedLabel() string {
	if b.State == "interrupted" {
		return L("Interrupted")
	}
	return L("Limit reached")
}

// StoppedTitle is the status card's title: which limit, or the interruption.
func (b BudgetState) StoppedTitle() string {
	if b.State != "budget_exhausted" {
		return L("Interrupted")
	}
	switch b.Reached {
	case "usd":
		return L("Spending limit reached")
	case "tokens":
		return L("Token limit reached")
	case "runtime":
		return L("Run time limit reached")
	case "retries":
		return L("Retry limit reached")
	case "connector_calls":
		return L("Plugin call limit reached")
	case "unknown_price":
		return L("Price unknown")
	}
	return L("Limit reached")
}

// StoppedDetail is what it used of the limit it reached, or why it stopped.
func (b BudgetState) StoppedDetail() string {
	if b.State != "budget_exhausted" {
		return L("The Runner restarted while this was running. Check what it already did, then resume.")
	}
	u, l := b.Usage, b.Limits
	of := func(limit *uint64) uint64 {
		if limit == nil {
			return 0
		}
		return *limit
	}
	switch b.Reached {
	case "usd":
		limit := 0.0
		if l.MaxUSD != nil {
			limit = *l.MaxUSD
		}
		return L("Used %@ of %@.", Spend(u.APICostUSD, u.SubscriptionEstimateUSD), Dollars(limit))
	case "tokens":
		return L("Used %@ of %@ tokens.", Count(u.Tokens), Count(of(l.MaxTokens)))
	case "runtime":
		return L("Ran %@ of %@.", Minutes(int(u.RuntimeSecs)), Minutes(int(of(l.MaxRuntimeSecs))))
	case "retries":
		return L("Retried %d of %d times.", u.Retries, of(l.MaxRetries))
	case "connector_calls":
		return L("Made %d of %d plugin calls.", u.ConnectorCalls, of(l.MaxConnectorCalls))
	case "unknown_price":
		return L("A spending limit can't cover a model without a known price. Add a token or run time limit.")
	}
	return L("Raise a limit to resume.")
}

// Fits says whether new limits leave room for more work: the limit it reached went up (or away),
// and no other limit is used up. Otherwise resuming needs a fresh allowance.
func (b BudgetState) Fits(next BudgetLimits) bool {
	raisedU := func(next, old *uint64) bool { return next == nil || old == nil || *next > *old }
	room := true
	switch b.Reached {
	case "usd":
		room = next.MaxUSD == nil || b.Limits.MaxUSD == nil || *next.MaxUSD > *b.Limits.MaxUSD
	case "unknown_price":
		room = next.MaxUSD == nil || next.MaxTokens != nil || next.MaxRuntimeSecs != nil
	case "tokens":
		room = raisedU(next.MaxTokens, b.Limits.MaxTokens)
	case "runtime":
		room = raisedU(next.MaxRuntimeSecs, b.Limits.MaxRuntimeSecs)
	case "retries":
		room = raisedU(next.MaxRetries, b.Limits.MaxRetries)
	case "connector_calls":
		room = raisedU(next.MaxConnectorCalls, b.Limits.MaxConnectorCalls)
	}
	u := b.Usage
	switch {
	case next.MaxUSD != nil && u.Dollars() >= *next.MaxUSD,
		next.MaxTokens != nil && u.Tokens >= *next.MaxTokens,
		next.MaxRuntimeSecs != nil && u.RuntimeSecs >= float64(*next.MaxRuntimeSecs),
		next.MaxConnectorCalls != nil && *next.MaxConnectorCalls > 0 && u.ConnectorCalls >= *next.MaxConnectorCalls:
		return false
	}
	return room
}

// BudgetFields are the Limits sheet's fields as typed: dollars, tokens, minutes, retries, and
// plugin calls. An empty field is no limit.
type BudgetFields struct{ USD, Tokens, Minutes, Retries, ConnectorCalls string }

func BudgetFieldsFor(limits BudgetLimits) BudgetFields {
	whole := func(value *uint64) string {
		if value == nil {
			return ""
		}
		return Count(*value)
	}
	var f BudgetFields
	if limits.MaxUSD != nil {
		f.USD = fmt.Sprintf("%.2f", *limits.MaxUSD)
	}
	f.Tokens = whole(limits.MaxTokens)
	if limits.MaxRuntimeSecs != nil {
		f.Minutes = strconv.FormatFloat(math.Round(float64(*limits.MaxRuntimeSecs)/6)/10, 'f', -1, 64)
	}
	f.Retries = whole(limits.MaxRetries)
	f.ConnectorCalls = whole(limits.MaxConnectorCalls)
	return f
}

// BudgetFieldError names the field that isn't a number.
type BudgetFieldError struct {
	Field   string
	Message string
}

func (e BudgetFieldError) Error() string { return e.Message }

// Limits reads the fields. A field that isn't a number is a BudgetFieldError naming it.
func (f BudgetFields) Limits() (BudgetLimits, error) {
	var limits BudgetLimits
	parse := func(field, text string, fraction bool) (*float64, error) {
		text = strings.NewReplacer(",", "", "$", "", " ", "").Replace(strings.TrimSpace(text))
		if text == "" {
			return nil, nil
		}
		value, err := strconv.ParseFloat(text, 64)
		if err != nil || math.IsNaN(value) || math.IsInf(value, 0) || value < 0 || (!fraction && value != math.Trunc(value)) {
			return nil, BudgetFieldError{field, L("Enter a number, or leave it empty for no limit.")}
		}
		return &value, nil
	}
	whole := func(value *float64) *uint64 {
		if value == nil {
			return nil
		}
		n := uint64(*value)
		return &n
	}
	usd, err := parse("usd", f.USD, true)
	if err != nil {
		return limits, err
	}
	limits.MaxUSD = usd
	for _, item := range []struct {
		field, text string
		into        **uint64
	}{{"tokens", f.Tokens, &limits.MaxTokens}, {"retries", f.Retries, &limits.MaxRetries}, {"connector_calls", f.ConnectorCalls, &limits.MaxConnectorCalls}} {
		value, err := parse(item.field, item.text, false)
		if err != nil {
			return limits, err
		}
		*item.into = whole(value)
	}
	minutes, err := parse("runtime", f.Minutes, true)
	if err != nil {
		return limits, err
	}
	if minutes != nil {
		if *minutes > 525_600 {
			return limits, BudgetFieldError{"runtime", L("Run time can be at most a year, or leave it empty for no limit.")}
		}
		secs := uint64(math.Round(*minutes * 60))
		limits.MaxRuntimeSecs = &secs
	}
	return limits, nil
}

func (s *Store) Budget(kind, id, runnerID string) *BudgetState {
	for i := range s.Budgets {
		if b := &s.Budgets[i]; b.Kind == kind && b.ID == id && b.RunnerID == runnerID {
			return b
		}
	}
	return nil
}

// StoppedTurn is the DM's newest turn when it stopped at a limit or was interrupted: the one to
// resume.
func (s *Store) StoppedTurn(chatID, runnerID string) *BudgetState {
	var newest *BudgetState
	for i := range s.Budgets {
		b := &s.Budgets[i]
		if b.Kind == "job" && b.ChatID == chatID && b.RunnerID == runnerID && (newest == nil || b.UpdatedAt > newest.UpdatedAt) {
			newest = b
		}
	}
	if newest == nil || !newest.IsStopped() {
		return nil
	}
	return newest
}

// BudgetChange is what the Limits sheet sends to the bot's Runner, in order: the limits for each
// scope in Saves, then Resume when set.
type BudgetChange struct {
	Saves  []BudgetTarget
	Limits BudgetLimits
	Resume *BudgetTarget
	// Fresh grants the whole allowance again on Resume.
	Fresh bool
}

type BudgetTarget struct{ Kind, ID string }

// ChangeBudget sets limits and resumes on the bot's Runner. What the work used stays unless the
// change is Fresh.
func (s *Store) ChangeBudget(bot *Bot, chatID string, change BudgetChange, done func(error)) {
	if s.IsMock {
		for _, target := range change.Saves {
			if b := s.Budget(target.Kind, target.ID, bot.RunnerID); b != nil {
				b.Limits = change.Limits
			} else {
				s.Budgets = append(s.Budgets, BudgetState{Kind: target.Kind, ID: target.ID, RunnerID: bot.RunnerID, ChatID: chatID, Limits: change.Limits, State: "ready", UpdatedAt: float64(time.Now().Unix())})
			}
		}
		if change.Resume != nil {
			if b := s.Budget(change.Resume.Kind, change.Resume.ID, bot.RunnerID); b != nil {
				b.State, b.Reached = "ready", ""
				if change.Fresh {
					b.Usage = BudgetUsage{}
				}
			}
		}
		s.emit(Event{Kind: EventBudgetsChanged})
		s.post(func() { done(nil) })
		return
	}
	Async(s, func() (struct{}, error) {
		for _, target := range change.Saves {
			params := map[string]any{"kind": target.Kind, "id": target.ID, "bot_id": bot.ID, "chat_id": chatID, "runner_id": bot.RunnerID, "limits": change.Limits}
			if _, err := call[map[string]any](s, "budgets.set", params); err != nil {
				return struct{}{}, err
			}
		}
		if target := change.Resume; target != nil {
			// The request id makes a repeated delivery a no-op on the Runner.
			params := map[string]any{"kind": target.Kind, "id": target.ID, "runner_id": bot.RunnerID, "renew": change.Fresh, "run": true, "request_id": uuid()}
			if _, err := call[map[string]any](s, "budgets.resume", params); err != nil {
				return struct{}{}, err
			}
		}
		return struct{}{}, nil
	}, func(_ struct{}, err error) { done(err) })
}

// CallLimits is a plugin account's shared call limit on its Runner.
type CallLimits struct {
	Limits struct {
		MaxCalls       uint64 `json:"max_calls"`
		WindowSecs     uint64 `json:"window_secs"`
		MaxConcurrency uint64 `json:"max_concurrency"`
	} `json:"limits"`
	// RetryAt is when the service asked the calls to wait until.
	RetryAt   *float64 `json:"retry_at"`
	ServiceID string   `json:"service_id"`
	PluginID  string   `json:"plugin_id"`
}

// SharesService is an account of a service with other accounts, which can share a limit too.
func (l CallLimits) SharesService() bool { return l.ServiceID != l.PluginID }

// Waiting is when calls wait until, while the service asked them to.
func (l CallLimits) Waiting() (time.Time, bool) {
	if l.RetryAt == nil {
		return time.Time{}, false
	}
	at := time.Unix(0, int64(*l.RetryAt*1e9))
	return at, at.After(time.Now())
}

// Summary is "60 a minute", "10 every 30 s".
func (l CallLimits) Summary() string {
	n := l.Limits.MaxCalls
	switch l.Limits.WindowSecs {
	case 60:
		return L("%d a minute", n)
	case 3600:
		return L("%d an hour", n)
	case 86_400:
		return L("%d a day", n)
	}
	return L("%d every %@", n, Duration(int(l.Limits.WindowSecs)))
}

// CallLimitFields are the Call Limit sheet's fields as typed.
type CallLimitFields struct{ Calls, Window, Concurrency string }

func CallLimitFieldsFor(l CallLimits) CallLimitFields {
	return CallLimitFields{strconv.FormatUint(l.Limits.MaxCalls, 10), strconv.FormatUint(l.Limits.WindowSecs, 10), strconv.FormatUint(l.Limits.MaxConcurrency, 10)}
}

func (f CallLimitFields) values() (calls, window, concurrency uint64, err error) {
	bad := errors.New(L("Use whole numbers: up to 10,000 calls every 1 to 86,400 seconds, and up to 256 at once."))
	numbers := make([]uint64, 3)
	for i, text := range []string{f.Calls, f.Window, f.Concurrency} {
		if numbers[i], err = strconv.ParseUint(strings.TrimSpace(text), 10, 64); err != nil {
			return 0, 0, 0, bad
		}
	}
	calls, window, concurrency = numbers[0], numbers[1], numbers[2]
	if calls > 10_000 || window < 1 || window > 86_400 || concurrency > 256 {
		return 0, 0, 0, bad
	}
	return calls, window, concurrency, nil
}

func scopeName(service bool) string {
	if service {
		return "service"
	}
	return "account"
}

// CallLimits reads a plugin account's call limit, or with `service` the one all of its service's
// accounts share.
func (s *Store) CallLimits(pluginID, runnerID string, service bool, done func(CallLimits, error)) {
	if s.IsMock {
		var limits CallLimits
		limits.Limits.MaxCalls, limits.Limits.WindowSecs, limits.Limits.MaxConcurrency = 60, 60, 4
		limits.PluginID, limits.ServiceID = pluginID, pluginID
		if pluginID == "github" {
			at := float64(time.Now().Add(4 * time.Minute).Unix())
			limits.RetryAt = &at
		}
		s.post(func() { done(limits, nil) })
		return
	}
	params := map[string]any{"plugin_id": pluginID, "runner_id": runnerID, "scope": scopeName(service)}
	Async(s, func() (CallLimits, error) { return call[CallLimits](s, "connector_limits.get", params) }, done)
}

// SetCallLimits saves the fields as the account's limit, or the service's.
func (s *Store) SetCallLimits(pluginID, runnerID string, service bool, fields CallLimitFields, done func(error)) {
	calls, window, concurrency, err := fields.values()
	if err != nil {
		s.post(func() { done(err) })
		return
	}
	if s.IsMock {
		s.post(func() { done(nil) })
		return
	}
	params := map[string]any{"plugin_id": pluginID, "runner_id": runnerID, "scope": scopeName(service),
		"limits": map[string]uint64{"max_calls": calls, "window_secs": window, "max_concurrency": concurrency}}
	Async(s, func() (map[string]any, error) { return call[map[string]any](s, "connector_limits.set", params) }, func(_ map[string]any, err error) { done(err) })
}

// Dollars is "$5.00", "<$0.01".
func Dollars(value float64) string {
	if value > 0 && value < 0.01 {
		return "<$0.01"
	}
	return fmt.Sprintf("$%.2f", value)
}

// Spend is money the turns used, with an estimate marked as one: "$0.42", "$0.42 est.", or both.
func Spend(api, estimate float64) string {
	switch {
	case api > 0 && estimate > 0:
		return L("%@ + %@ est.", Dollars(api), Dollars(estimate))
	case estimate > 0:
		return L("%@ est.", Dollars(estimate))
	}
	return Dollars(api)
}

// Count is "12,400".
func Count[T int | uint64](value T) string {
	text := strconv.FormatUint(uint64(value), 10)
	for i := len(text) - 3; i > 0; i -= 3 {
		text = text[:i] + "," + text[i:]
	}
	return text
}

func tokenCount(value uint64) string {
	if value > math.MaxInt32 {
		return Count(value)
	}
	return Tokens(int(value))
}

// Minutes is "45 min", "2 h 30 min", "30 s".
func Minutes(seconds int) string {
	if seconds < 60 {
		return L("%d s", seconds)
	}
	minutes := int(math.Round(float64(seconds) / 60))
	if minutes < 60 {
		return L("%d min", minutes)
	}
	if minutes%60 == 0 {
		return L("%d h", minutes/60)
	}
	return L("%d h %d min", minutes/60, minutes%60)
}

// Duration is a call window: "30 s", "5 min", "2 h".
func Duration(seconds int) string {
	switch {
	case seconds%3600 == 0:
		return L("%d h", seconds/3600)
	case seconds%60 == 0:
		return L("%d min", seconds/60)
	}
	return L("%d s", seconds)
}
