package main

import (
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
)

func TestRenderPluginSheet(t *testing.T) {
	m, tt := sheetATester(t, func(m *mainWindow) {
		store.AutoReview.Rules = append(store.AutoReview.Rules, model.AutoReviewRule{ID: "rule-1", Text: "Create issues", Behavior: "allow", Tool: "github/create_issue"})
		m.presentPlugin("github", sheetAWorkbench())
	})
	for _, text := range []string{"GitHub", "Installed on Workbench.", "STATUS", "Ready", "Always allowed", "create_issue", "Reset", "SIGN-IN", "Signed in", "Sign in again", "Sign Out", "SETUP", "GITHUB_TOKEN", "Keys and sign-ins stay on this device.", "Save", "Remove…", "Done"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	sheetARender(t, tt, "plugin-github")

	// Reset takes the plugin's rules out of Auto-review.
	if err := tt.Click("Reset"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	for _, rule := range store.AutoReview.Rules {
		if rule.Tool == "github/create_issue" {
			t.Errorf("rules %v", store.AutoReview.Rules)
		}
	}
	if tt.HasText("Always allowed") {
		t.Errorf("after Reset: %q", tt.Texts())
	}

	// A secret is written, then its field empties.
	if err := tt.Click("GITHUB_TOKEN"); err != nil {
		t.Fatal(err)
	}
	r, _ := tt.Find("GITHUB_TOKEN")
	tt.ClickAt(r.X+r.W-40, r.Y+r.H/2)
	tt.Type("ghp_123")
	tt.Frame()
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)

	// Remove asks first.
	if err := tt.Click("Remove…"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("Remove GitHub from Workbench?") {
		t.Fatalf("no question: %q", tt.Texts())
	}
	if err := tt.Click("Remove"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if m.hasSheet() {
		t.Errorf("sheet still up: %q", tt.Texts())
	}
}

func TestRenderPluginSheetSignIn(t *testing.T) {
	_, tt := sheetATester(t, func(m *mainWindow) { m.presentPlugin("linear", sheetAWorkbench()) })
	for _, text := range []string{"Linear", "Sign in", "Not signed in"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	sheetARender(t, tt, "plugin-linear")

	// A device-flow sign-in waiting for its code, on another Runner.
	m, tt := sheetATester(t, func(m *mainWindow) {})
	studio := store.Device("dev-studio")
	s := &pluginSheet{w: &m.appWindow, pluginID: "atlassian", runner: studio, values: map[string]string{}, copiedAt: map[string]time.Time{},
		detail: &model.PluginDetail{
			Status:   model.InstalledPlugin{ID: "atlassian", Name: "Atlassian", State: model.PluginNeedsAuth, Detail: "Waiting for the sign-in"},
			Homepage: "https://www.atlassian.com/platform/remote-mcp-server",
			Servers: []model.PluginDetailServer{
				{Name: "jira", OAuth: true, Code: "WDJB-MJHT", Link: "https://auth.atlassian.com/activate"},
				{Name: "confluence", OAuth: true, SignedIn: true},
			},
			Variables: []model.PluginDetailVariable{
				{Name: "ATLASSIAN_SITE", Description: "Your site, as example.atlassian.net.", Required: true, IsSet: true, Value: "lorca.atlassian.net"},
				{Name: "ATLASSIAN_TOKEN", Secret: true, IsSet: true},
			},
			Skills: []model.NamedText{{Name: "triage", Description: "Sorts new issues by area and urgency."}},
		}}
	m.present(s.view, nil)
	sheetASettle(tt)
	for _, text := range []string{"Atlassian", "Installed on Studio.", "Waiting for the sign-in", "Site", "www.atlassian.com", "jira", "WDJB-MJHT", "Copy code and open auth.atlassian.com", "confluence", "ATLASSIAN_SITE *", "SKILLS", "triage", "Keys and sign-ins are sent sealed to Studio and stay there."} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	sheetARender(t, tt, "plugin-code")
}

func TestPluginSheetOpensMcpServer(t *testing.T) {
	_, tt := sheetATester(t, func(m *mainWindow) { m.presentPlugin("deepwiki", sheetAWorkbench()) })
	if !tt.HasText("deepwiki") || !tt.HasText("read_wiki_structure") || !tt.HasText("Edit as") {
		t.Errorf("deepwiki: %q", tt.Texts())
	}
}
