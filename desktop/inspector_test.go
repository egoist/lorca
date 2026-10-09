package main

import (
	"slices"
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
		m.inspectorRuntime(c, store.Bot("bot-nova"), store.Chat("chat-nova"))
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
