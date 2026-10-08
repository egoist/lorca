package main

import (
	"strings"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func integrationFixtureAccounts(runner *model.Device) {
	runner.Plugins = []model.InstalledPlugin{
		{ID: "gmail-00000000000000000000000000000001", ServiceID: "gmail", AccountName: "Work", Name: "Gmail · Work", State: model.PluginReady, Detail: "Connected", Icon: "envelope"},
		{ID: "gmail-00000000000000000000000000000002", ServiceID: "gmail", AccountName: "Personal", Name: "Gmail · Personal", State: model.PluginNeedsAuth, Detail: "Sign in", Icon: "envelope"},
		{ID: "gmail-00000000000000000000000000000003", ServiceID: "gmail", AccountName: "Shared", Name: "Gmail · Shared", State: model.PluginInsufficientAccess, Detail: "Sign in again to grant the required access", Icon: "envelope"},
		{ID: "gmail-00000000000000000000000000000004", ServiceID: "gmail", AccountName: "Offline", Name: "Gmail · Offline", State: model.PluginError, Detail: "Unable to reach the service. Try again.", Icon: "envelope"},
	}
}

func TestIntegrationAccountSelectionAndRename(t *testing.T) {
	m, tt := sheetTester(t, func(m *mainWindow) {
		integrationFixtureAccounts(sheetAWorkbench())
		m.presentPluginAccounts("gmail", "Gmail", sheetAWorkbench())
	})
	settleTransitions(tt)
	wantText(t, tt, "Work", "Personal", "Shared", "Connected", "Sign in again to grant the required access", L("Add Account…"))
	renderTo(t, tt, "integration-accounts-light")
	tt.SetDark(true)
	settleTransitions(tt)
	renderTo(t, tt, "integration-accounts-dark")
	tt.SetDark(false)
	if err := tt.Click(L("Manage…")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	settleTransitions(tt)
	wantText(t, tt, "Gmail · Work", L("Account name"), L("Manage Accounts…"), L("Signed in"), L("Sign Out"))
	renderTo(t, tt, "integration-connected")
	id := store.PluginAccounts("gmail", "dev-workbench")[0].ID
	if err := tt.Click(L("Account name")); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("Office")
	tt.Frame()
	// A roster-triggered load must preserve the field that is being edited.
	store.SetAutoReview(store.AutoReview)
	runPosts()
	tt.Frame()
	if err := tt.Click(L("Save")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	accounts := store.PluginAccounts("gmail", "dev-workbench")
	if accounts[0].ID != id || accounts[0].AccountName != "Office" || accounts[1].AccountName != "Personal" {
		t.Fatalf("rename crossed accounts: %+v", accounts)
	}
	wantText(t, tt, "Gmail · Office")
	if err := tt.Click(L("Sign Out")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	wantText(t, tt, L("Not signed in"), L("Sign in"))
	if accounts = store.PluginAccounts("gmail", "dev-workbench"); accounts[0].State != model.PluginNeedsAuth || accounts[1].State != model.PluginNeedsAuth {
		t.Fatal(accounts)
	}
	// The State row also says Sign in; target the sign-in row's trailing action.
	row, ok := tt.Find(L("Not signed in"))
	if !ok {
		t.Fatal("no sign-in row")
	}
	tt.ClickAt(row.X+row.W+28, row.Y+row.H/2)
	sheetASettle(tt)
	wantText(t, tt, L("Signed in"))
	if err := tt.Click(L("Manage Accounts…")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	wantText(t, tt, "Office", "Personal")
	if !m.hasSheet() {
		t.Fatal("account picker closed")
	}
}

func TestIntegrationAddAccountUsesTheChosenName(t *testing.T) {
	_, tt := sheetTester(t, func(m *mainWindow) { m.presentPluginAccounts("gmail", "Gmail", sheetAWorkbench()) })
	settleTransitions(tt)
	wantText(t, tt, L("No accounts connected"))
	if err := tt.Click(L("Add Account…")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if err := tt.Click(L("Account name")); err != nil {
		t.Fatal(err)
	}
	tt.Type("Work")
	tt.Frame()
	renderTo(t, tt, "integration-add-account")
	if err := tt.Click(L("Add")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	settleTransitions(tt)
	accounts := store.PluginAccounts("gmail", "dev-workbench")
	if len(accounts) != 1 || accounts[0].AccountName != "Work" || !strings.HasPrefix(accounts[0].ID, "gmail-") {
		t.Fatal(accounts)
	}
	wantText(t, tt, "Gmail · Work", L("Not signed in"))
}

func TestIntegrationRecoveryScreens(t *testing.T) {
	for _, scenario := range []struct{ label, text, image string }{
		{"Personal", "Sign in", "integration-needs-sign-in"},
		{"Shared", "Sign in again to grant the required access", "integration-insufficient-access"},
		{"Offline", "Unable to reach the service. Try again.", "integration-error"},
	} {
		t.Run(scenario.label, func(t *testing.T) {
			_, tt := sheetTester(t, func(m *mainWindow) {
				integrationFixtureAccounts(sheetAWorkbench())
				for _, account := range store.PluginAccounts("gmail", "dev-workbench") {
					if account.AccountName == scenario.label {
						m.presentPlugin(account.ID, sheetAWorkbench())
					}
				}
			})
			sheetASettle(tt)
			settleTransitions(tt)
			wantText(t, tt, "Gmail · "+scenario.label, scenario.text, L("Sign in"))
			renderTo(t, tt, scenario.image)
		})
	}
}

func TestIntegrationMarketplaceRecognizesNamedInstances(t *testing.T) {
	m := demoWindow(t)
	integrationFixtureAccounts(sheetAWorkbench())
	m.userWantsInspector = false
	mk := &marketplace{m: m, runnerID: "dev-workbench", installing: map[string]bool{}, installed: map[string]map[string]model.InstalledPlugin{}, loading: marketLoaded,
		catalog: model.Marketplace{Plugins: []model.MarketplacePlugin{{ID: "gmail", Name: "Gmail", NamedAccounts: true, Description: "Search and read mail from a named account.", Icon: "envelope"}}}}
	mk.show(&marketPage{kind: marketPluginPage, id: "gmail"})
	mk.sheet = m.present(mk.view, nil)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settleTransitions(tt)
	wantText(t, tt, L("Manage…"), "Connected")
	if err := tt.Click(L("Manage…")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	wantText(t, tt, "Work", "Personal", L("Add Account…"))
}

func TestIntegrationMarketplaceScreens(t *testing.T) {
	m := demoWindow(t)
	m.userWantsInspector = false
	integrationFixtureAccounts(sheetAWorkbench())
	var catalog model.Marketplace
	store.Marketplace(func(value model.Marketplace, err error) {
		if err != nil {
			t.Error(err)
		}
		catalog = value
	})
	runPosts()
	var plugins []model.MarketplacePlugin
	for _, plugin := range catalog.Plugins {
		if plugin.ID == "gmail" || plugin.ID == "slack" || plugin.ID == "google-calendar" || plugin.ID == "google-drive" {
			plugins = append(plugins, plugin)
		}
	}
	if len(plugins) != 4 {
		t.Fatal("curated integrations missing", plugins)
	}
	mk := &marketplace{m: m, runnerID: "dev-workbench", catalog: model.Marketplace{Plugins: plugins}, loading: marketLoaded,
		installing: map[string]bool{}, installed: map[string]map[string]model.InstalledPlugin{}}
	mk.show(&marketPage{kind: marketListPage, title: L("Plugins"), items: func(catalog *model.Marketplace) []marketItem { return marketPluginItems(catalog.Plugins, nil) }})
	mk.sheet = m.present(mk.view, nil)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settleTransitions(tt)
	wantText(t, tt, "Gmail", "Slack", "Google Calendar", "Google Drive")
	renderTo(t, tt, "integration-marketplace")
}
