package model

import (
	"encoding/json"
	"testing"
	"time"
)

func TestRoutineReliabilityWireKeepsCanonicalTimeAndIndependentHealth(t *testing.T) {
	wire := decodeJSON[WireRoutine](t, `{"id":"r1","bot_id":"b1","name":"Inbox","schedule":"0 9 * * *","schedule_text":"Every day at 9:00 AM","timezone":"America/New_York","missed_run_policy":"skip","is_enabled":true,"state":"quiet","runner_id":"runner-1","runner_available":true,"next_run_at":1791550800,"next_run_text":"2026-10-09 09:00 -04:00 (America/New_York)","health":{"last_check_at":100,"last_success_at":90,"retry_at":200},"retry_at":300,"check":"return null","created_at":1}`)
	r := ToRoutine(wire)
	expect(t, r.SchedulingTimezone(), "America/New_York")
	expect(t, r.MissedPolicy(), "skip")
	expect(t, r.NextSummary(), "2026-10-09 09:00 -04:00 (America/New_York)")
	expect(t, []bool{r.HasRunnerAvailability, r.RunnerAvailable, r.CanRunNow()}, []bool{true, true, true})
	expect(t, r.LastCheckAt, time.Unix(100, 0))
	expect(t, r.LastSuccessfulCheckAt, time.Unix(90, 0))
	expect(t, r.RetryAt, time.Unix(300, 0))
	if !r.LastRunAt.IsZero() {
		t.Fatal("check health must not become a model run")
	}
	legacy := ToRoutine(decodeJSON[WireRoutine](t, `{"id":"old","is_enabled":true}`))
	expect(t, []string{legacy.SchedulingTimezone(), legacy.MissedPolicy()}, []string{"UTC", "coalesce"})
	legacy.HasRunnerAvailability, legacy.RunnerAvailable = true, false
	if legacy.CanRunNow() {
		t.Fatal("offline Runner admits a run")
	}
	legacy.RunnerAvailable, legacy.PausedReason = true, "authentication"
	if legacy.CanRunNow() {
		t.Fatal("authentication pause admits a run")
	}
}

type routineTestCall struct {
	method string
	params map[string]any
	reply  chan json.RawMessage
}
type routineTestTransport struct{ calls chan routineTestCall }

func (r *routineTestTransport) Reconnect() {}
func (r *routineTestTransport) Request(method string, params any) (json.RawMessage, error) {
	call := routineTestCall{method: method, params: params.(map[string]any), reply: make(chan json.RawMessage, 1)}
	r.calls <- call
	return <-call.reply, nil
}

func nextRoutineCall(t *testing.T, calls <-chan routineTestCall) routineTestCall {
	t.Helper()
	select {
	case call := <-calls:
		return call
	case <-time.After(3 * time.Second):
		t.Fatal("no CLI request")
		return routineTestCall{}
	}
}
func nextRoutinePost(t *testing.T, posts <-chan func()) func() {
	t.Helper()
	select {
	case fn := <-posts:
		return fn
	case <-time.After(3 * time.Second):
		t.Fatal("no main-thread reply")
		return nil
	}
}

func TestRoutinePolicyRequestsFreezeInputAndIgnoreOlderReplies(t *testing.T) {
	transport := &routineTestTransport{calls: make(chan routineTestCall, 4)}
	posts := make(chan func(), 4)
	s := NewStore(transport, func(fn func()) { posts <- fn }, false)
	s.Routines = []*Routine{{ID: "r1", BotID: "b1", Timezone: "UTC"}}
	first := "America/New_York"
	s.SetRoutinePolicy("r1", RoutinePolicy{Timezone: &first}, nil)
	first = "changed after admission"
	a := nextRoutineCall(t, transport.calls)
	expect(t, a.method, "routines.update")
	expect(t, a.params, map[string]any{"id": "r1", "timezone": "America/New_York"})
	second := "Asia/Singapore"
	s.SetRoutinePolicy("r1", RoutinePolicy{Timezone: &second}, nil)
	b := nextRoutineCall(t, transport.calls)
	b.reply <- json.RawMessage(`{"routine":{"id":"r1","bot_id":"b1","timezone":"Asia/Singapore"}}`)
	post := nextRoutinePost(t, posts)
	expect(t, s.Routine("r1").Timezone, "UTC")
	post()
	a.reply <- json.RawMessage(`{"routine":{"id":"r1","bot_id":"b1","timezone":"America/New_York"}}`)
	nextRoutinePost(t, posts)()
	expect(t, s.Routine("r1").Timezone, "Asia/Singapore")
}

func TestRoutineServiceDiscoveryUsesReadOnlyCLIAndMainThreadReply(t *testing.T) {
	transport := &routineTestTransport{calls: make(chan routineTestCall, 1)}
	posts := make(chan func(), 1)
	s := NewStore(transport, func(fn func()) { posts <- fn }, false)
	called := false
	s.RunnerServiceStatus("runner-1", func(status ServiceStatus, err error) {
		called = true
		if err != nil || !status.Installed || status.RunningPID == nil {
			t.Fatalf("status: %+v, %v", status, err)
		}
	})
	call := nextRoutineCall(t, transport.calls)
	expect(t, call.method, "device.service_status")
	expect(t, call.params, map[string]any{"id": "runner-1"})
	call.reply <- json.RawMessage(`{"installed":true,"running_pid":42,"supervised":true,"log":"test log"}`)
	post := nextRoutinePost(t, posts)
	if called {
		t.Fatal("callback escaped main-thread queue")
	}
	post()
}
