package main

import (
	"strings"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// gmailAccounts gives the demo's Workbench four Gmail accounts, one in each state a row words.
func gmailAccounts(runner *model.Device) {
	account := func(n, name string, state model.PluginState, detail string) model.InstalledPlugin {
		return model.InstalledPlugin{ID: "gmail-0000000000000000000000000000000" + n, ServiceID: "gmail", AccountName: name, Name: "Gmail · " + name,
			Description: "Search, read, and organize your email, and write drafts for you to send.", Icon: "envelope", State: state, Detail: detail}
	}
	runner.Plugins = append(runner.Plugins,
		account("1", "Work", model.PluginReady, "Connected"),
		account("2", "Personal", model.PluginNeedsAuth, "Sign in"),
		account("3", "Family", model.PluginInsufficientAccess, "Sign in again to grant the required access"),
		account("4", "Old job", model.PluginError, "gmailmcp.googleapis.com could not be reached. Check the connection and try again."),
	)
}

// marketplaceOn opens the marketplace on a plugin's page, found the way users find it.
func marketplaceOn(t *testing.T, name string) (*mainWindow, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	m.userWantsInspector = false
	gmailAccounts(sheetAWorkbench())
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	runPosts()
	tt.Frame()
	m.presentMarketplace("")
	runPosts()
	settleTransitions(tt)
	marketClick(t, tt, L("Search plugins and bots"))
	tt.Type(name)
	tt.Frame()
	marketClick(t, tt, name)
	settleTransitions(tt)
	return m, tt
}

func TestIntegrationAccountsOnTheServicePage(t *testing.T) {
	_, tt := marketplaceOn(t, "Gmail")
	wantText(t, tt, L("Accounts on %@", "Workbench"), "Work", "Personal", "Family", "Old job",
		L("Connected"), L("Needs a sign-in"), L("Needs more access"), L("Can’t connect"), L("Add Account…"))
	for _, long := range []string{"Sign in again to grant the required access", L("Manage…")} {
		if tt.HasText(long) {
			t.Errorf("the page shows %q", long)
		}
	}
	renderBoth(t, tt, "integration-gmail-page")

	// The row is the way into the account.
	marketClick(t, tt, "Work")
	sheetASettle(tt)
	settleTransitions(tt)
	wantText(t, tt, "Gmail · Work", L("Name"), L("Signed in"))
	if tt.HasText(L("Manage Accounts…")) {
		t.Error("the account sheet links to another list of accounts")
	}
}

func TestIntegrationAddAccountAsksForItsName(t *testing.T) {
	_, tt := marketplaceOn(t, "Gmail")
	marketClick(t, tt, L("Add Account…"))
	settleTransitions(tt)
	wantText(t, tt, L("New %@ Account", "Gmail"), L("A name such as Work or Personal tells your bots which account to use."))
	tt.Type("Travel")
	tt.Frame()
	renderBoth(t, tt, "integration-new-account")
	marketClick(t, tt, L("Add"))
	sheetASettle(tt)
	settleTransitions(tt)
	wantText(t, tt, "Gmail · Travel")
	var added *model.InstalledPlugin
	for i, plugin := range sheetAWorkbench().Plugins {
		if plugin.AccountName == "Travel" {
			added = &sheetAWorkbench().Plugins[i]
		}
	}
	if added == nil || added.ServiceID != "gmail" || !strings.HasPrefix(added.ID, "gmail-") {
		t.Fatalf("added %+v", added)
	}
}

func TestIntegrationFirstAccountWithoutAName(t *testing.T) {
	_, tt := marketplaceOn(t, "Google Calendar")
	if tt.HasText(L("Accounts on %@", "Workbench")) {
		t.Error("an empty accounts card")
	}
	marketClick(t, tt, L("Add"))
	settleTransitions(tt)
	wantText(t, tt, L("New %@ Account", "Google Calendar"))
	// Return answers the alert's Add; the page's Add sits behind it.
	tt.Key(0, ui.KeyEnter)
	sheetASettle(tt)
	settleTransitions(tt)
	wantText(t, tt, "Google Calendar · Account 1")
}

func TestIntegrationRenameKeepsTheAccount(t *testing.T) {
	_, tt := sheetTester(t, func(m *mainWindow) {
		gmailAccounts(sheetAWorkbench())
		m.presentPlugin("gmail-00000000000000000000000000000001", sheetAWorkbench())
	})
	sheetASettle(tt)
	settleTransitions(tt)
	wantText(t, tt, "Gmail · Work", "Work")
	if err := tt.Click(L("Name")); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("Office")
	tt.Key(0, ui.KeyEnter)
	sheetASettle(tt)
	settleTransitions(tt)
	wantText(t, tt, "Gmail · Office")
	work := sheetAWorkbench().Plugins
	renamed := false
	for _, plugin := range work {
		if plugin.ID == "gmail-00000000000000000000000000000001" {
			renamed = plugin.AccountName == "Office"
		}
		if plugin.AccountName == "Personal" && plugin.ID != "gmail-00000000000000000000000000000002" {
			t.Error("the rename moved another account")
		}
	}
	if !renamed {
		t.Fatalf("not renamed: %+v", work)
	}
}

func TestIntegrationSettingsListsEachAccount(t *testing.T) {
	m, tt := settingsWindowTester(t, 900)
	gmailAccounts(sheetAWorkbench())
	m.showSettings(model.PanePlugins)
	settle(tt)
	wantText(t, tt, "Gmail · Work", "Gmail · Personal")
	renderBoth(t, tt, "integration-settings")
}

// A named account's sheet says how it stands once, in its sign-in row, and leaves the site to its
// service's page.
func TestIntegrationAccountSheetIsOneCard(t *testing.T) {
	for _, c := range []struct {
		id, title, render string
		want              []string
	}{
		{"1", "Gmail · Work", "integration-account-work", []string{L("Signed in"), L("Sign in again"), L("Sign Out")}},
		{"2", "Gmail · Personal", "integration-account-personal", []string{L("Not signed in"), L("Sign in")}},
		{"3", "Gmail · Family", "", []string{L("Needs more access"), L("Sign in")}},
		{"4", "Gmail · Old job", "", []string{L("Can’t connect"), "gmailmcp.googleapis.com could not be reached. Check the connection and try again."}},
	} {
		t.Run(c.title, func(t *testing.T) {
			_, tt := sheetTester(t, func(m *mainWindow) {
				gmailAccounts(sheetAWorkbench())
				m.presentPlugin("gmail-0000000000000000000000000000000"+c.id, sheetAWorkbench())
			})
			sheetASettle(tt)
			settleTransitions(tt)
			wantText(t, tt, append([]string{c.title, L("Account"), L("Name"), L("Sign-in")}, c.want...)...)
			for _, gone := range []string{L("Status"), L("State"), L("Site"), "developers.google.com"} {
				if tt.HasText(gone) {
					t.Errorf("the sheet shows %q", gone)
				}
			}
			if c.render != "" {
				renderBoth(t, tt, c.render)
			}
		})
	}
}
