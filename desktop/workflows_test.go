package main

import (
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// workflowTester opens the marketplace over the demo's main window, its catalog in, with the
// demo's sample finishing at once.
func workflowTester(t *testing.T) (*mainWindow, *ui.Tester, *marketplace) {
	t.Helper()
	m := demoWindow(t)
	saved := model.DemoWorkflowSampleTime
	model.DemoWorkflowSampleTime = 20 * time.Millisecond
	t.Cleanup(func() { model.DemoWorkflowSampleTime = saved })
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	tt.SetPreferences(ui.Preferences{ReduceMotion: true, TextScale: 1})
	settle(tt)
	mk := m.appWindow.presentMarketplace("dev-workbench", &marketPage{kind: marketHomePage}, m.open)
	settle(tt)
	return m, tt, mk
}

// workflowOpen opens a workflow's page from the home page.
func workflowOpen(t *testing.T, tt *ui.Tester, name string) *workflowPage {
	t.Helper()
	marketClick(t, tt, name)
	settle(tt)
	for _, page := range marketplaceOf(t).pages {
		if page.workflow != nil && page.workflow.pack.Name == name {
			return page.workflow
		}
	}
	t.Fatalf("no page for %q", name)
	return nil
}

func marketplaceOf(t *testing.T) *marketplace {
	t.Helper()
	if workflowMarket == nil {
		t.Fatal("no marketplace")
	}
	return workflowMarket
}

var workflowMarket *marketplace

// workflowType types into the setup's field for `question`, to the right of its key.
func workflowType(t *testing.T, tt *ui.Tester, question, text string) {
	t.Helper()
	key, ok := tt.Find(question)
	if !ok {
		t.Fatalf("no %q in %q", question, tt.Texts())
	}
	tt.ClickAt(key.X+key.W+120, key.Y+key.H/2)
	tt.Type(text)
	settle(tt)
}

// workflowWaitForSample lets the demo's sample finish and the page read it.
func workflowWaitForSample(tt *ui.Tester) {
	time.Sleep(60 * time.Millisecond)
	settle(tt)
	settle(tt)
}

func TestWorkflowSampleBeforeSchedule(t *testing.T) {
	m, tt, mk := workflowTester(t)
	workflowMarket = mk
	wantText(t, tt, L("Workflows"), "Repository monitoring", "Meeting preparation")
	renderBoth(t, tt, "marketplace-workflows")

	wp := workflowOpen(t, tt, "Repository monitoring")
	wantText(t, tt, "Repositories", L("Bot"), L("%@ (new)", "Repository Monitor"), "GitHub", L("Connected"), L("Run Sample"))
	if tt.HasText(L("Schedule")) || tt.HasText(L("Cancel Setup")) {
		t.Fatal("schedule or cancel before anything is set up")
	}
	// Run Sample waits for the setup's answer.
	workflowClick(t, tt, L("Run Sample"))
	if wp.progress.Setup.Sample != nil {
		t.Fatal("sample ran without an answer")
	}
	renderBoth(t, tt, "workflow-new")

	workflowType(t, tt, "Repositories", "example/workflow-demo")
	// A roster event reads the setup again and leaves what the user typed alone.
	for _, notify := range pluginSheetWatch.watchers {
		notify(model.Event{Kind: model.EventRosterChanged})
	}
	settle(tt)
	if wp.answer("repositories") != "example/workflow-demo" {
		t.Fatalf("typed answer lost: %q", wp.answer("repositories"))
	}
	renderBoth(t, tt, "workflow-filled")

	// Long enough to look at the page while it runs.
	model.DemoWorkflowSampleTime = 1500 * time.Millisecond
	workflowClick(t, tt, L("Run Sample"))
	wantText(t, tt, L("%@ is working on it…", "Repository Monitor"), L("Schedule"), L("Off"), L("Cancel Setup"))
	if tt.HasText(L("Turn On Schedule")) {
		t.Fatal("schedule offered before the sample's result")
	}
	renderBoth(t, tt, "workflow-running")
	time.Sleep(1500 * time.Millisecond)
	workflowWaitForSample(tt)
	wantText(t, tt, "#128 Fix token refresh on wake", L("Not Now"), L("Turn On Schedule"))
	if wp.progress.Routines[0].IsEnabled {
		t.Fatal("the sample turned the schedule on")
	}
	renderBoth(t, tt, "workflow-result")

	chat := wp.progress.Setup.Sample.ChatID
	workflowClick(t, tt, L("Turn On Schedule"))
	if marketplaceSheet != nil {
		t.Fatal("the sheet stayed up after Turn On Schedule")
	}
	if m.selection.ChatID != chat {
		t.Fatalf("landed on %+v, not the workflow's chat %q", m.selection, chat)
	}

	mk = m.appWindow.presentMarketplace("dev-workbench", &marketPage{kind: marketHomePage}, m.open)
	workflowMarket = mk
	settle(tt)
	wp = workflowOpen(t, tt, "Repository monitoring")
	wantText(t, tt, L("On"), L("Turn Off Workflow"))
	if !wp.progress.Routines[0].IsEnabled || tt.HasText(L("Run Sample")) {
		t.Fatal("not on, or still offering a sample")
	}
	renderBoth(t, tt, "workflow-on")

	workflowClick(t, tt, L("Turn Off Workflow"))
	wantText(t, tt, L("%@ is off.", "Repository monitoring"), L("Workflows"))
	if len(mk.pages) != 1 {
		t.Fatal("turning off stayed on the workflow's page")
	}
}

func TestWorkflowAccountsWaitForSignIn(t *testing.T) {
	_, tt, mk := workflowTester(t)
	workflowMarket = mk
	// A calendar account that waits for its sign-in, and no Drive account yet: Add makes one,
	// named after the workflow, which then waits for its own sign-in.
	wp := workflowOpen(t, tt, "Meeting preparation")
	wantText(t, tt, "Google Calendar", "Google Drive", L("Sign In"), L("Add"))
	workflowClick(t, tt, L("Add"))
	if id := wp.progress.Setup.ConnectionIDs["google-drive"]; id == "" {
		t.Fatal("Add chose no account")
	}
	if account := wp.progress.Connections[1].Account(); account == nil || account.State != model.PluginNeedsAuth || account.AccountName != "Meeting preparation" {
		t.Fatalf("the added Drive account: %+v", account)
	}
	renderBoth(t, tt, "workflow-accounts")
	mk.goBack()
	settle(tt)

	// Of two inboxes, the user picks one; until then the row says so.
	inbox := workflowOpen(t, tt, "Inbox triage")
	wantText(t, tt, "Gmail", L("Not chosen"), L("Choose…"))
	workflowClick(t, tt, L("Choose…"))
	if err := tt.ChooseMenuItem("Personal"); err != nil {
		t.Fatalf("pick an inbox: %v", err)
	}
	settle(tt)
	if inbox.progress.Setup.ConnectionIDs["gmail"] != "gmail-personal" {
		t.Fatalf("picked %q", inbox.progress.Setup.ConnectionIDs["gmail"])
	}
	wantText(t, tt, "Personal", L("Connected"))
	renderBoth(t, tt, "workflow-named-accounts")
	mk.goBack()
	settle(tt)

	// GitHub on the Runner waits for its sign-in: the row's one button opens the plugin's sheet.
	for _, device := range store.Devices {
		for i := range device.Plugins {
			if device.Plugins[i].ID == "github" {
				device.Plugins[i].State, device.Plugins[i].Detail = model.PluginNeedsAuth, "Sign in"
			}
		}
	}
	wp = workflowOpen(t, tt, "Repository monitoring")
	workflowType(t, tt, "Repositories", "example/workflow-demo")
	wantText(t, tt, L("Sign In"))
	renderBoth(t, tt, "workflow-sign-in")
	workflowClick(t, tt, L("Run Sample"))
	if wp.progress.Setup.Sample != nil {
		t.Fatal("sample ran before the sign-in")
	}
	workflowClick(t, tt, L("Sign In"))
	wantText(t, tt, L("Sign in"))
}

func TestWorkflowFromOnboarding(t *testing.T) {
	o, tt := onboardingDemo(t)
	o.show(onboardingDone)
	onboardingSettle(tt)
	wantText(t, tt, L("Open Lorca"), L("Choose a Workflow…"))
	onboardingRender(t, tt, "onboarding-done")
	onboardingClick(t, tt, L("Choose a Workflow…"))
	onboardingSettle(tt)
	wantText(t, tt, L("Choose a Workflow"), "Repository monitoring", "Inbox triage")
	onboardingRender(t, tt, "onboarding-workflows")
}

func TestWorkflowChinese(t *testing.T) {
	_, tt, mk := workflowTester(t)
	workflowMarket = mk
	l10n.Set("zh-Hans", "zh-CN")
	defer l10n.Set("en", "en-US")
	settle(tt)
	workflowOpen(t, tt, "Repository monitoring")
	workflowType(t, tt, "Repositories", "example/workflow-demo")
	wantText(t, tt, L("Run Sample"), L("Accounts"), L("Connected"))
	renderTo(t, tt, "workflow-chinese")
}

func workflowClick(t *testing.T, tt *ui.Tester, label string) {
	t.Helper()
	marketClick(t, tt, label)
	settle(tt)
}

func TestWorkflowFeedbackCollectorListensOnceOn(t *testing.T) {
	m, tt, mk := workflowTester(t)
	workflowMarket = mk
	wp := workflowOpen(t, tt, "Feedback collector")
	wantText(t, tt, "Repository", "Telegram", "GitHub")
	workflowType(t, tt, "Repository", "acme/app")
	workflowClick(t, tt, L("Run Sample"))
	workflowWaitForSample(tt)
	tt.SetSize(1180, 1400)
	settle(tt)
	wantText(t, tt, "Community feedback", "Telegram · Mentions, replies, #feedback")
	if wp.progress.Channels[0].IsOn() {
		t.Fatal("the channel listens before the workflow is on")
	}
	workflowClick(t, tt, L("Turn On Schedule"))
	mk = m.appWindow.presentMarketplace("dev-workbench", &marketPage{kind: marketHomePage}, m.open)
	workflowMarket = mk
	settle(tt)
	wp = workflowOpen(t, tt, "Feedback collector")
	if !wp.progress.Channels[0].IsOn() {
		t.Fatal("turning the workflow on left its channel off")
	}
	tt.Scroll(590, 500, 0, 800)
	settle(tt)
	renderBoth(t, tt, "workflow-feedback-on")
}
