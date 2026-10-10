package main

import (
	"encoding/json"
	"strings"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// routineTriggers puts a one-time reminder, a watch on one pull request, and a routine around
// calendar events in the demo's roster, as the CLI sends them.
func routineTriggers(t *testing.T) {
	t.Helper()
	now := float64(time.Now().Unix())
	tomorrow := time.Now().AddDate(0, 0, 1)
	nine := float64(time.Date(tomorrow.Year(), tomorrow.Month(), tomorrow.Day(), 9, 0, 0, 0, time.Local).Unix())
	routines := []map[string]any{
		{"id": "rt-dentist", "bot_id": "bot-nova", "name": "Call the dentist", "prompt": "Remind me to call the dentist about Thursday.",
			"schedule": "once " + tomorrow.Format("2006-01-02") + " 09:00", "schedule_text": "Once on " + tomorrow.Format("2006-01-02") + " at 9:00 AM",
			"timezone": "Local", "is_enabled": true, "state": "on", "once_at": nine, "next_run_at": nine, "created_at": now - 3600},
		{"id": "rt-login-pr", "bot_id": "bot-nova", "name": "Login PR", "prompt": "Tell me what changed on the login pull request and whether it needs me.",
			"schedule": "every 10m", "schedule_text": "Watches acme/project#42", "is_enabled": true, "state": "on",
			"pull_request": map[string]any{"repo": "acme/project", "number": 42, "title": "Add passkey sign-in", "url": "https://github.com/acme/project/pull/42"},
			"next_run_at":  now + 6*60, "last_run_at": now - 50*60, "last_outcome": "sent", "created_at": now - 86400,
			"health": map[string]any{"last_check_at": now - 240, "last_success_at": now - 240, "status": "quiet"}},
		{"id": "rt-call-prep", "bot_id": "bot-nova", "name": "Call prep", "prompt": "Write a one-page prep for the customer call: who they are, open issues, and what to ask.",
			"schedule": "15m before events", "schedule_text": "15 minutes before events matching “Customer call”", "is_enabled": true, "state": "on",
			"calendar": map[string]any{"account": "Google Calendar · Work", "matching": "Customer call", "minutes": 15, "after": false,
				"next_event": map[string]any{"title": "Customer call: Acme", "start": now + 75*60, "end": now + 105*60}},
			"next_run_at": now + 60*60, "created_at": now - 86400*3},
	}
	store.Routines = nil
	for _, data := range routines {
		encoded, err := json.Marshal(data)
		if err != nil {
			t.Fatal(err)
		}
		var wire model.WireRoutine
		if err := json.Unmarshal(encoded, &wire); err != nil {
			t.Fatal(err)
		}
		store.Routines = append(store.Routines, model.ToRoutine(wire))
	}
}

func TestRoutineTriggerWords(t *testing.T) {
	demoWindow(t)
	routineTriggers(t)
	if got := store.Routine("rt-login-pr").ScheduleText; got != L("Watches %@", "acme/project#42") {
		t.Errorf("watch %q", got)
	}
	if got := store.Routine("rt-call-prep").ScheduleText; got != L("%@ before events matching “%@”", L("%d minutes", 15), "Customer call") {
		t.Errorf("events %q", got)
	}
	if got := store.Routine("rt-dentist").ScheduleText; !strings.HasPrefix(got, "Once on ") || !strings.HasSuffix(got, " at 9:00 AM") {
		t.Errorf("once %q", got)
	}
	if got := model.AroundEvents(0, true, ""); got != L("When each event ends") {
		t.Errorf("end %q", got)
	}
	if got := model.AroundEvents(120, false, ""); got != L("%@ before each event", L("%d hours", 2)) {
		t.Errorf("hours %q", got)
	}
	watch := store.Routine("rt-login-pr")
	if !watch.LooksFirst() || watch.Symbol() != "arrow.triangle.pull" || !strings.Contains(watch.Detail(), L("%@ · Next check %@", watch.ScheduleText, model.Upcoming(watch.NextRunAt))) {
		t.Errorf("watch row %q %q", watch.Symbol(), watch.Detail())
	}
	if watch.ScheduleSummary() != watch.ScheduleText {
		t.Error("a watch names no timezone")
	}
	watch.State, watch.Health = "failed", model.RoutineHealth{Status: "failed"}
	if watch.Problem() != model.ProblemWatchFailed {
		t.Errorf("problem %v", watch.Problem())
	}
}

func TestRoutineTriggerSheets(t *testing.T) {
	m, tt := sheetTester(t, func(m *mainWindow) {})
	routineTriggers(t)
	for _, id := range []string{"rt-dentist", "rt-login-pr", "rt-call-prep"} {
		m.presentRoutine(id, store.Bot("bot-nova"), nil)
		settle(tt)
		switch id {
		case "rt-dentist":
			if tt.HasText(L("Missed runs")) {
				t.Error("a one-time routine runs once its Runner is back, whatever the policy")
			}
		case "rt-login-pr":
			if !tt.HasText(L("Pull request")) || !tt.HasText("Add passkey sign-in") || !tt.HasText(L("Next check")) || !tt.HasText(L("Last check")) {
				t.Errorf("texts %q", tt.Texts())
			}
		case "rt-call-prep":
			if !tt.HasText(L("Calendar")) || !tt.HasText("Google Calendar · Work") || !tt.HasText("Customer call: Acme") {
				t.Errorf("texts %q", tt.Texts())
			}
		}
		renderBoth(t, tt, "routine-"+id)
		closeSheet(t, m, tt)
	}
}

func TestRenderRoutineTriggersInInspector(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-nova")
	routineTriggers(t)
	// Tall enough that the inspector shows its Routines section.
	tt := ui.NewTester(m.frame(m.view), 1180, 1700)
	settle(tt)
	if !tt.HasText("Call the dentist") || !tt.HasText("Login PR") || !tt.HasText("Call prep") {
		t.Errorf("texts %q", tt.Texts())
	}
	renderBoth(t, tt, "inspector-routine-triggers")
}
