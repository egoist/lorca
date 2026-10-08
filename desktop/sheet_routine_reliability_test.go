package main

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func routineReliabilityTester(t *testing.T, state string) (*mainWindow, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	m.userWantsInspector = false
	m.selectChat("chat-nova")
	now := time.Date(2026, 10, 9, 13, 0, 0, 0, time.UTC).Unix()
	data := map[string]any{
		"id": "rt-parity-78", "bot_id": "bot-nova", "name": "Inbox monitor", "prompt": "Summarize new items in the sample inbox.",
		"schedule": "0 9 * * 1-5", "schedule_text": "Weekdays at 9:00 AM", "timezone": "America/New_York", "missed_run_policy": "coalesce",
		"is_enabled": state != "blocked", "state": state, "runner_available": state != "waiting_for_runner", "created_at": now - 86400,
		"check": "// Synthetic fixture\nreturn null;", "next_run_at": time.Date(2026, 10, 12, 13, 0, 0, 0, time.UTC).Unix(), "next_run_text": "2026-10-12 09:00 -04:00 (America/New_York)",
		"health": map[string]any{"last_check_at": now, "last_success_at": now - 3600},
	}
	if state == "quiet" {
		data["health"] = map[string]any{"last_check_at": now, "last_success_at": now}
	}
	if state == "failed" {
		data["retry_at"] = now + 3600
		data["recovery_action"] = "Check the connection on the assigned Runner. The routine retries automatically after its backoff."
	}
	if state == "blocked" {
		data["paused_reason"] = "authentication"
		delete(data, "next_run_at")
		delete(data, "next_run_text")
		data["recovery_action"] = "Reconnect the provider or integration on the assigned Runner, then resume this routine."
	}
	if state == "waiting_for_runner" {
		data["missed_run_policy"] = "skip"
		data["recovery_action"] = "Start Lorca or lorca serve on the assigned Runner. Install the service on an owned computer that stays available."
	}
	encoded, err := json.Marshal(data)
	if err != nil {
		t.Fatal(err)
	}
	var wire model.WireRoutine
	if err := json.Unmarshal(encoded, &wire); err != nil {
		t.Fatal(err)
	}
	store.Routines = []*model.Routine{model.ToRoutine(wire)}
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	m.presentRoutine("rt-parity-78", store.Bot("bot-nova"), nil)
	settleTransitions(tt)
	return m, tt
}

func TestRoutineReliabilityControlsKeepDraftsAcrossBuilds(t *testing.T) {
	m, tt := routineReliabilityTester(t, "quiet")
	if !tt.HasText("America/New_York") || !tt.HasText("2026-10-12 09:00 -04:00 (America/New_York)") {
		t.Fatalf("missing canonical schedule: %q", tt.Texts())
	}
	if err := tt.Click(L("Run once")); err != nil {
		t.Fatal(err)
	}
	if err := tt.Click(L("Skip")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Routine("rt-parity-78").MissedPolicy() != "skip" {
		t.Fatal("missed-run choice did not reach the store")
	}
	if err := tt.Click(L("Change…")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if len(m.sheets) != 2 {
		t.Fatal("timezone editor did not open")
	}
	if !tt.Focused(L("Timezone")) {
		t.Fatal("timezone field did not receive focus")
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("Asia/Singapore")
	for range 4 {
		tt.Frame()
	}
	if !tt.HasText("Asia/Singapore") {
		t.Fatal("timezone draft disappeared on redraw")
	}
	renderBoth(t, tt, "desktop-78-routine-timezone-editor")
	if err := tt.Click(L("Save")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if len(m.sheets) != 1 || store.Routine("rt-parity-78").Timezone != "Asia/Singapore" {
		t.Fatalf("save: sheets %d, routine %+v", len(m.sheets), store.Routine("rt-parity-78"))
	}
}

func TestRoutineReliabilityStatesAndVisibleActions(t *testing.T) {
	for _, state := range []string{"quiet", "failed", "blocked", "waiting_for_runner"} {
		t.Run(state, func(t *testing.T) {
			_, tt := routineReliabilityTester(t, state)
			r := store.Routine("rt-parity-78")
			if !tt.HasText(r.StateText()) || !tt.HasText(L("Last successful check")) || !tt.HasText(L("Never")) {
				t.Fatalf("missing independent health/state: %q", tt.Texts())
			}
			renderBoth(t, tt, "desktop-78-routine-"+state)
			if state == "blocked" || state == "waiting_for_runner" {
				if err := tt.Click(L("Run Now")); err != nil {
					t.Fatal(err)
				}
				settle(tt)
				if store.Routine(r.ID).IsRunning {
					t.Fatal("disabled Run Now admitted work")
				}
			}
			tt.SetSize(1180, 520)
			settle(tt)
			for _, label := range []string{L("Run Now"), L("Done")} {
				bounds, ok := tt.Find(label)
				if !ok || bounds.Y+bounds.H > 520 {
					t.Fatalf("%s is outside small-window viewport: %+v", label, bounds)
				}
			}
		})
	}
}

func TestRunnerServiceDiscoveryAndSelectionChanges(t *testing.T) {
	m, tt := settingsWindowTester(t, 900)
	m.showSettings(model.PaneDevice)
	settle(tt)
	if !tt.HasText("lorca service install") || !tt.HasText("lorca service status") {
		t.Fatalf("missing service workflow: %q", tt.Texts())
	}
	if err := tt.Click(L("Check status")); err != nil {
		t.Fatal(err)
	}
	m.showSettingsDevice("dev-studio")
	tt.Frame()
	runPosts()
	tt.Frame()
	if m.settings.service.status != nil {
		t.Fatal("previous Runner's service reply replaced the selected Runner")
	}
	if err := tt.Click(L("Check status")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText(L("Not installed")) {
		t.Fatalf("missing confirmed mock status: %q", tt.Texts())
	}
	renderBoth(t, tt, "desktop-78-runner-service")
	store.Device("dev-studio").Status = model.StatusOffline
	settle(tt)
	if !tt.HasText(L("Waiting for Runner")) {
		t.Fatal("offline Runner availability missing")
	}
	renderBoth(t, tt, "desktop-78-runner-service-offline")
}
