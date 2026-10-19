package main

import (
	"slices"
	"strings"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func TestInspectorRoutineSwitch(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(func(c *ui.Context) {
		applyTheme(c)
		m.inspectorRoutines(c, store.Bot("bot-nova"))
	}, 360, 400)
	settle(tt)
	routine := store.Routine("rt-brief")
	if routine == nil || !routine.IsEnabled {
		t.Fatal("the demo routine is missing or paused")
	}
	if err := tt.Click(L("Pause %@", routine.Name)); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Routine("rt-brief").IsEnabled || m.hasSheet() {
		t.Fatal("the switch should pause the routine without opening its sheet")
	}
	if err := tt.Click(L("Resume %@", routine.Name)); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !store.Routine("rt-brief").IsEnabled || m.hasSheet() {
		t.Fatal("the switch should resume the routine without opening its sheet")
	}
}

// A decision provider reviews actions and runs no bot: Runs with offers neither it nor a decision
// model, and the default names the first model that chats.
func TestInspectorLeavesDecisionModelsOut(t *testing.T) {
	m := demoWindow(t)
	if !store.Credential("custom:openrouter-decisions").Decides() || slices.Contains(store.ProviderKinds(), "custom:openrouter-decisions") {
		t.Fatalf("kinds %q", store.ProviderKinds())
	}
	// A bot made without asking runs on the first connected provider that chats, never on a
	// decision provider listed before it.
	saved := store.Providers
	store.Providers = []model.ProviderCredential{
		{Kind: "custom:openrouter-decisions", IsConnected: true, Name: "OpenRouter Decisions", API: model.APISystemOne},
		{Kind: "custom:ollama", IsConnected: true, Name: "Ollama", API: model.APIChatCompletions},
	}
	if got := store.PreferredProvider(); got != "custom:ollama" {
		t.Errorf("preferred %q", got)
	}
	store.Providers = saved
	bot := store.Bot("bot-nova")
	store.SetBotRuntime(bot.ID, "opencode", "", "")
	tt := ui.NewTester(func(c *ui.Context) {
		applyTheme(c)
		m.inspectorRuntime(c, store.Bot("bot-nova"))
	}, 360, 400)
	settle(tt)
	if err := tt.Click("OpenCode Zen"); err != nil {
		t.Fatal(err)
	}
	if got := tt.Menu(); slices.Contains(got, "OpenRouter Decisions") || !slices.Contains(got, "Ollama") {
		t.Errorf("providers %q", got)
	}
	tt.Key(0, ui.KeyEscape)
	settle(tt)
	if err := tt.Click(L("Default (%@)", "DeepSeek V4.1 Flash")); err != nil {
		t.Fatalf("%v in %q", err, tt.Texts())
	}
	if got := tt.Menu(); slices.Contains(got, "Jev 1.13") || slices.Contains(got, "Jev 1.13 Free") || !slices.Contains(got, "Kimi K3") {
		t.Errorf("models %q", got)
	}
}

// The DM inspector shows what goes on in the chat; a click on its bot opens the bot's settings, a
// tab per page: its profile and memory, what it runs with and where its turns run, and its plugins.
func TestInspectorBotOpensSettings(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-nova")
	inspector := ui.NewTester(func(c *ui.Context) {
		applyTheme(c)
		m.inspectorView(c, "chat-nova")
	}, 300, 900)
	inspector.SetScale(2)
	settle(inspector)
	for _, moved := range []string{L("Runs with"), L("Memory"), L("Plugins"), L("Where turns run")} {
		if inspector.HasText(strings.ToUpper(moved)) {
			t.Errorf("the inspector still shows %s", moved)
		}
	}
	if !inspector.HasText(strings.ToUpper(L("Usage"))) || !inspector.HasText(strings.ToUpper(L("Routines"))) {
		t.Fatalf("the inspector lacks Usage or Routines in %q", inspector.Texts())
	}
	renderBoth(t, inspector, "inspector-dm")
	click(t, inspector, "Project Manager")
	if !m.hasSheet() {
		t.Fatal("a click on the bot did not open its settings")
	}
	sheet := ui.NewTester(m.frame(m.view), 1100, 900)
	sheet.SetScale(2)
	settleTransitions(sheet)
	tabs := []struct {
		tab, render      string
		shown, elsewhere []string
	}{
		{L("Profile"), "bot-settings", []string{L("Bot Settings"), strings.ToUpper(L("Profile")), strings.ToUpper(L("Memory"))},
			[]string{strings.ToUpper(L("Runs with")), strings.ToUpper(L("Plugins"))}},
		{L("Runs with"), "bot-settings-runs-with", []string{strings.ToUpper(L("Runs with")), strings.ToUpper(L("Where turns run"))},
			[]string{strings.ToUpper(L("Profile")), strings.ToUpper(L("Memory"))}},
		{L("Plugins"), "bot-settings-plugins", []string{strings.ToUpper(L("Plugins"))},
			[]string{strings.ToUpper(L("Runs with")), strings.ToUpper(L("Profile"))}},
	}
	for _, each := range tabs {
		click(t, sheet, each.tab)
		settle(sheet)
		for _, shown := range each.shown {
			if !sheet.HasText(shown) {
				t.Errorf("the %s tab lacks %s in %q", each.tab, shown, sheet.Texts())
			}
		}
		for _, other := range each.elsewhere {
			if sheet.HasText(other) {
				t.Errorf("the %s tab shows %s", each.tab, other)
			}
		}
		renderBoth(t, sheet, each.render)
	}
	click(t, sheet, L("Done"))
	settleTransitions(sheet)
	if m.hasSheet() {
		t.Fatal("Done did not close the settings")
	}
}
