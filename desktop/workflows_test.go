package main

import (
	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
	"maps"
	"testing"
)

func workflowTester(t *testing.T) (*workflowSheet, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	s := m.presentWorkflows(nil, "dev-workbench", nil)
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	tt.SetPreferences(ui.Preferences{ReduceMotion: true, TextScale: 1})
	settle(tt)
	return s, tt
}
func workflowClick(t *testing.T, tt *ui.Tester, label string) {
	t.Helper()
	for range 16 {
		r, ok := tt.Find(label)
		if !ok || (r.Y >= 65 && r.Y+r.H <= 860) {
			break
		}
		tt.Scroll(590, 400, 0, r.Y-350)
		tt.Frame()
	}
	if err := tt.Click(label); err != nil {
		t.Fatal(err)
	}
	settle(tt)
}
func workflowChoose(t *testing.T, tt *ui.Tester, label, item string) {
	t.Helper()
	workflowClick(t, tt, label)
	if err := tt.ChooseMenuItem(item); err != nil {
		t.Fatalf("choose %q: %v", item, err)
	}
	settle(tt)
}
func workflowRepository(t *testing.T) (*workflowSheet, *ui.Tester) {
	t.Helper()
	s, tt := workflowTester(t)
	s.start(s.packs[2])
	settle(tt)
	workflowClick(t, tt, L("Answer for %@", "Which repositories should be monitored?"))
	tt.Type("example/workflow-demo")
	settle(tt)
	workflowClick(t, tt, L("Continue"))
	workflowChoose(t, tt, L("%@ account", "GitHub"), "GitHub")
	return s, tt
}

func TestWorkflowNativeSelectionReviewAndActivation(t *testing.T) {
	s, tt := workflowTester(t)
	wantText(t, tt, L("Guided Workflows"), "Meeting preparation", "Repository monitoring")
	renderBoth(t, tt, "workflow-outcomes")
	s.start(s.packs[2])
	settle(tt)
	wantText(t, tt, "Which repositories should be monitored?", L("Specialists · reuse a bot on this Runner or add a suitable one."))
	workflowClick(t, tt, L("Answer for %@", "Which repositories should be monitored?"))
	tt.Type("example/workflow-demo")
	settle(tt)
	role := s.progress.Specialists[0]
	existing := role.Choices[1]
	workflowChoose(t, tt, L("Specialist: %@", role.Name), existing.Name)
	// A roster tick cannot reset the user's bound fields or chosen specialist.
	before := s.answers["repositories"]
	for _, notify := range pluginSheetWatch.watchers {
		notify(model.Event{Kind: model.EventRosterChanged})
	}
	if s.answers["repositories"] != before || *before != "example/workflow-demo" || *s.bots[role.ID] != existing.ID {
		t.Fatal("draft changed during a roster event")
	}
	renderBoth(t, tt, "workflow-questions")
	workflowClick(t, tt, L("Continue"))
	if s.progress.Setup.BotIDs[role.ID] != existing.ID {
		t.Fatal("explicit specialist was lost")
	}
	if tt.HasText(L("Enable Schedules")) {
		t.Fatal("activation before a sample")
	}
	workflowChoose(t, tt, L("%@ account", "GitHub"), "GitHub")
	workflowClick(t, tt, L("Run Sample"))
	wantText(t, tt, L("I Have Reviewed This Result"))
	if tt.HasText(L("Enable Schedules")) {
		t.Fatal("activation before review")
	}
	renderBoth(t, tt, "workflow-before-review")
	workflowClick(t, tt, L("I Have Reviewed This Result"))
	wantText(t, tt, L("Enable Schedules"), L("Finish with Schedules Paused"))
	if s.progress.Routines[0].IsEnabled {
		t.Fatal("review armed schedule")
	}
	renderBoth(t, tt, "workflow-after-review")
	workflowClick(t, tt, L("Enable Schedules"))
	if !s.progress.Routines[0].IsEnabled {
		t.Fatal("activation not sent")
	}
	workflowClick(t, tt, L("Cancel Setup and Pause Imported Routines"))
	wantText(t, tt, L("Resume Setup"))
	if s.progress.Routines[0].IsEnabled {
		t.Fatal("cancel kept imported schedule active")
	}
	renderBoth(t, tt, "workflow-cancelled")
	workflowClick(t, tt, L("Resume Setup"))
	if s.progress.Setup.BotIDs[role.ID] != existing.ID {
		t.Fatal("resume duplicated/replaced specialist")
	}
}

func TestWorkflowNamedAccountsAndRecoverableSetup(t *testing.T) {
	s, tt := workflowTester(t)
	s.start(s.packs[1])
	settle(tt)
	for _, q := range s.progress.Setup.Pack.Questions {
		workflowClick(t, tt, L("Answer for %@", q.Label))
		tt.Type("Test scope")
		settle(tt)
	}
	workflowClick(t, tt, L("Continue"))
	if s.progress.Connections[0].SelectedID != "" {
		t.Fatal("named account chosen implicitly")
	}
	workflowChoose(t, tt, L("%@ account", "Gmail"), "Gmail · Personal (demo)")
	if s.progress.Setup.ConnectionIDs["gmail"] != "gmail-demo-Personal" {
		t.Fatal("wrong named instance")
	}
	// Fixture an expired sign-in without launching a real browser or using credentials.
	s.progress.Connections[0].State = model.PluginNeedsAuth
	s.progress.Connections[0].Detail = "Sign-in expired (fixture). Reconnect this selected account."
	s.progress.CanSample = false
	s.progress.BlockedReason = "Reconnect Gmail (fixture) before running a sample."
	settle(tt)
	wantText(t, tt, L("Sign In…"))
	renderBoth(t, tt, "workflow-connection-recovery")
	original := s.progress.Setup.ID
	workflowClick(t, tt, L("Close"))
	if !s.closed {
		t.Fatal("sheet remained subscribed")
	}
	m := app.main
	next := m.presentWorkflows(nil, "dev-workbench", nil)
	settle(tt)
	next.start(next.packs[1])
	settle(tt)
	if next.progress.Setup.ID != original || next.progress.Setup.ConnectionIDs["gmail"] != "gmail-demo-Personal" {
		t.Fatal("reopen lost saved instance/setup")
	}
}

func TestWorkflowPendingRefreshPreservesDirtyFields(t *testing.T) {
	s, tt := workflowRepository(t)
	workflowClick(t, tt, L("Edit Setup"))
	*s.answers["repositories"] = "another/repository"
	original := s.answers["repositories"]
	s.refresh()
	settle(tt)
	if original != s.answers["repositories"] || *original != "another/repository" {
		t.Fatal("refresh replaced dirty field")
	}
	if !maps.Equal(s.progress.Setup.Answers, map[string]string{"repositories": "example/workflow-demo"}) {
		t.Fatal("unsaved draft escaped to authority")
	}
}

func TestWorkflowOnboardingEntryAndChinese(t *testing.T) {
	o, tt := onboardingDemo(t)
	o.show(onboardingDone)
	onboardingSettle(tt)
	wantText(t, tt, L("Choose a Workflow…"), L("Open Lorca"))
	onboardingRender(t, tt, "workflow-entry")
	onboardingClick(t, tt, L("Choose a Workflow…"))
	wantText(t, tt, L("Guided Workflows"))
	s, fixture := workflowRepository(t)
	l10n.Set("zh-Hans", "zh-CN")
	settle(fixture)
	if !fixture.HasText(L("Run Sample")) {
		t.Fatal("translated control missing")
	}
	renderTo(t, fixture, "workflow-chinese")
	l10n.Set("en", "en-US")
	s.sheet.dismiss()
}
