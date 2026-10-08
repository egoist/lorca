package main

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// routineStates puts routines in each state the CLI reports in the demo's roster, as the CLI
// sends them: on in another timezone, quiet checks, paused by failed sign-ins, failing to connect,
// and waiting for an offline Runner.
func routineStates(t *testing.T) {
	t.Helper()
	now := float64(time.Now().Unix())
	// The next time the clock in a zone reads hour:00, on a weekday when weekdays is set.
	next := func(zone string, hour int, weekdays bool) float64 {
		location, err := time.LoadLocation(zone)
		if err != nil {
			t.Fatal(err)
		}
		at := time.Now().In(location)
		at = time.Date(at.Year(), at.Month(), at.Day(), hour, 0, 0, 0, location)
		for !at.After(time.Now()) || weekdays && (at.Weekday() == time.Saturday || at.Weekday() == time.Sunday) {
			at = at.AddDate(0, 0, 1)
		}
		return float64(at.Unix())
	}
	routines := []map[string]any{
		{"id": "rt-brief", "bot_id": "bot-nova", "name": "Morning brief", "prompt": "Post a short brief of what changed.",
			"schedule": "0 9 * * 1-5", "schedule_text": "Weekdays at 9:00 AM", "timezone": "Pacific/Kiritimati", "is_enabled": true, "state": "on",
			"last_run_at": now - 3*3600, "last_outcome": "sent", "next_run_at": next("Pacific/Kiritimati", 9, true), "created_at": now - 86400*12},
		{"id": "rt-reviews", "bot_id": "bot-nova", "name": "Review requests", "prompt": "Tell me which pull requests need my review first.",
			"schedule": "every 10m", "schedule_text": "Every 10 minutes", "is_enabled": true, "state": "on", "check": "return null;",
			"next_run_at": now + 7*60, "created_at": now - 86400*2,
			"health": map[string]any{"last_check_at": now - 180, "last_success_at": now - 180, "status": "quiet"}},
		{"id": "rt-inbox", "bot_id": "bot-nova", "name": "Support inbox", "prompt": "Draft a reply to each support email that needs me.",
			"schedule": "every 30m", "schedule_text": "Every 30 minutes", "is_enabled": false, "paused_reason": "authentication", "state": "blocked",
			"check": "return (await tools.gmail__search({})).structuredContent;", "last_run_at": now - 26*3600, "last_outcome": "sent", "created_at": now - 86400*6,
			"health": map[string]any{"last_check_at": now - 2400, "last_success_at": now - 21*3600, "status": "blocked", "authentication_failures": 3}},
		{"id": "rt-checklist", "bot_id": "bot-nova", "name": "Launch checklist", "prompt": "Report new blockers or completed milestones.",
			"schedule": "every 2h", "schedule_text": "Every 2 hours", "is_enabled": true, "state": "failed", "check": "return null;",
			"next_run_at": now + 38*60, "created_at": now - 86400*3,
			"health": map[string]any{"last_check_at": now - 1320, "last_success_at": now - 8520, "status": "failed", "connection_failures": 2}},
		{"id": "rt-backup", "bot_id": "bot-ember", "name": "Backup report", "prompt": "Tell me if last night's backups failed.",
			"schedule": "0 7 * * *", "schedule_text": "Every day at 7:00 AM", "is_enabled": true, "state": "waiting_for_runner", "missed_run_policy": "skip",
			"last_run_at": next("Local", 7, false) - 2*86400, "last_outcome": "pass", "next_run_at": next("Local", 7, false), "created_at": now - 86400*20},
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

func TestRoutineProblems(t *testing.T) {
	demoWindow(t)
	routineStates(t)
	for id, want := range map[string]model.RoutineProblem{
		"rt-brief": model.ProblemNone, "rt-reviews": model.ProblemNone, "rt-inbox": model.ProblemSignedOut,
		"rt-checklist": model.ProblemCantConnect, "rt-backup": model.ProblemOffline,
	} {
		if got := store.Routine(id).Problem(); got != want {
			t.Errorf("%s: problem %v, want %v", id, got, want)
		}
	}
	if store.Routine("rt-checklist").Problem().NeedsUser() {
		t.Error("a failed connection is tried again on its own")
	}
	if got := store.Routine("rt-brief").ScheduleSummary(); got != L("%@ (%@ time)", "Weekdays at 9:00 AM", "Kiritimati") {
		t.Errorf("schedule %q", got)
	}
	if got := store.Routine("rt-reviews").ScheduleSummary(); got != "Every 10 minutes" {
		t.Errorf("an interval names no zone: %q", got)
	}
	if got := store.Routine("rt-reviews").LastCheckSummary(); got != L("%@ · nothing new", model.DaySeparator(store.Routine("rt-reviews").Health.LastCheckAt)) {
		t.Errorf("last check %q", got)
	}
}

func TestRoutineSheetStates(t *testing.T) {
	m, tt := sheetTester(t, func(m *mainWindow) {})
	routineStates(t)
	m.presentRoutine("rt-inbox", store.Bot("bot-nova"), nil)
	settle(tt)
	if !tt.HasText(L("Needs sign-in")) || !tt.HasText(L("The check couldn’t sign in to a plugin three times in a row, so the routine is paused. Sign in to the plugin again on %@, then resume it.", "Workbench")) {
		t.Errorf("texts %q", tt.Texts())
	}
	if !tt.HasText(L("Last successful check")) || !tt.HasText(L("Resume")) {
		t.Errorf("no history or Resume: %q", tt.Texts())
	}
	if err := tt.Click(L("Run Now")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Routine("rt-inbox").IsRunning {
		t.Error("Run Now ran a routine paused by failed sign-ins")
	}
	renderBoth(t, tt, "routine-signed-out")
	closeSheet(t, m, tt)

	for _, id := range []string{"rt-brief", "rt-reviews", "rt-checklist", "rt-backup"} {
		bot := "bot-nova"
		if id == "rt-backup" {
			bot = "bot-ember"
		}
		m.presentRoutine(id, store.Bot(bot), nil)
		settle(tt)
		renderBoth(t, tt, "routine-"+id)
		if id == "rt-backup" {
			if !tt.HasText(L("Skip")) || !tt.HasText(L("Waiting for Runner")) {
				t.Errorf("texts %q", tt.Texts())
			}
			if err := tt.Click(L("Run Now")); err != nil {
				t.Fatal(err)
			}
			settle(tt)
			if store.Routine("rt-backup").IsRunning {
				t.Error("Run Now ran a routine whose Runner is offline")
			}
		}
		closeSheet(t, m, tt)
	}
}

// closeSheet presses Done.
func closeSheet(t *testing.T, m *mainWindow, tt *ui.Tester) {
	t.Helper()
	if err := tt.Click(L("Done")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if m.hasSheet() {
		t.Fatal("Done left the sheet up")
	}
}

func TestRenderRoutineStatesInInspector(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-nova")
	routineStates(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	if !tt.HasText(L("Needs sign-in")) || !tt.HasText(L("Can’t connect")) {
		t.Errorf("texts %q", tt.Texts())
	}
	renderBoth(t, tt, "inspector-routines")
}

func TestRenderRunnerService(t *testing.T) {
	m, tt := settingsWindowTester(t, 760)
	m.showSettingsDevice("dev-workbench")
	m.showSettings(model.PaneDevice)
	settle(tt)
	note := L("Run lorca service install in Terminal on %@ to keep its bots running while Lorca is closed.", "Workbench")
	if !tt.HasText(L("Background service")) || !tt.HasText(L("Not installed")) || !tt.HasText(note) {
		t.Errorf("texts %q", tt.Texts())
	}
	renderBoth(t, tt, "settings-device-service")
	m.settings.service.status = &model.ServiceStatus{Installed: true, Running: true}
	settle(tt)
	if !tt.HasText(L("Running")) || tt.HasText(note) {
		t.Errorf("texts %q", tt.Texts())
	}
	renderTo(t, tt, "settings-device-service-running")
	m.showSettingsDevice("dev-closet")
	settle(tt)
	if tt.HasText(L("Background service")) {
		t.Error("an offline Runner shows no service")
	}
}
