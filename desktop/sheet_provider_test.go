package main

import (
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func sheetAPreset(t *testing.T, name string) *model.CustomPreset {
	t.Helper()
	for i := range model.CustomPresets {
		if model.CustomPresets[i].Name == name {
			preset := model.CustomPresets[i]
			return &preset
		}
	}
	t.Fatalf("no preset %s", name)
	return nil
}

func TestRenderCustomProviderSheet(t *testing.T) {
	var saved model.ProviderKind
	m, tt := sheetATester(t, func(m *mainWindow) {
		m.presentCustomProvider("", sheetAPreset(t, "OpenRouter"), func(kind model.ProviderKind) { saved = kind })
	})
	if !tt.HasText("Add OpenRouter") || !tt.HasText("Loading models…") {
		t.Errorf("before the listing: %q", tt.Texts())
	}
	// A preset needs its key next.
	if !tt.Focused("API key") {
		t.Errorf("the key has no focus")
	}
	sheetAWait(tt, 400*time.Millisecond)
	for _, text := range []string{"Anthropic: Claude Sonnet 5", "anthropic/claude-sonnet-5", "1.0M", "Requests go to https://openrouter.ai/api/v1/chat/completions."} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	sheetARender(t, tt, "custom-openrouter")

	// A model picked from the list, and one the server does not list.
	if err := tt.Click("OpenAI: GPT-6 Sol"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("1 selected") {
		t.Errorf("after a pick: %q", tt.Texts())
	}
	if err := tt.Click("Search or add a model ID"); err != nil {
		t.Fatal(err)
	}
	tt.Type("my/own-model")
	tt.Frame()
	if !tt.HasText("Add “my/own-model”") {
		t.Errorf("no Add row: %q", tt.Texts())
	}
	sheetARender(t, tt, "custom-openrouter-add")
	tt.Key(0, ui.KeyEnter)
	tt.Frame()
	if !tt.HasText("2 selected") || !tt.HasText("my/own-model") || !m.hasSheet() {
		t.Errorf("after Return: %q", tt.Texts())
	}
	if err := tt.Click("Add"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if !tt.HasText("OpenRouter connected.") {
		t.Errorf("after Add: %q", tt.Texts())
	}
	sheetAWait(tt, 700*time.Millisecond)
	if m.hasSheet() || saved != "custom:openrouter" {
		t.Errorf("saved %q, sheet up %v", saved, m.hasSheet())
	}
	credential := store.Credential("custom:openrouter")
	if credential == nil || len(credential.Models) != 2 || credential.Models[0].ID != "openai/gpt-6-sol" {
		t.Errorf("credential %+v", credential)
	}
}

func TestRenderCustomProviderSheetSaved(t *testing.T) {
	m, tt := sheetATester(t, func(m *mainWindow) { m.presentCustomProvider("custom:ollama", nil, nil) })
	sheetAWait(tt, 400*time.Millisecond)
	for _, text := range []string{"Ollama", "Save", "Delete", "qwen3:8b", "gemma3:12b", "2 selected", "Default"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	sheetARender(t, tt, "custom-ollama")
	if err := tt.Click("Delete"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if m.hasSheet() || store.Credential("custom:ollama") != nil {
		t.Errorf("not deleted: %q", tt.Texts())
	}

	// An empty sheet asks for a base URL first.
	_, tt = sheetATester(t, func(m *mainWindow) { m.presentCustomProvider("", nil, nil) })
	if !tt.HasText("Add Custom Provider") || !tt.HasText("Enter the base URL to load the server’s models.") || !tt.Focused("Name") {
		t.Errorf("empty: %q", tt.Texts())
	}
	sheetARender(t, tt, "custom-empty")
}

func TestAddProviderMenu(t *testing.T) {
	m, tt := sheetATester(t, func(m *mainWindow) {})
	view := func(c *ui.Context) {
		m.frame(m.view)(c)
		ui.Overlay(c, func() {
			ui.Button(c, "Add Provider…").Absolute().Left(10).Top(10).Menu(func(menu *ui.Menu) { m.addProviderMenu(menu) })
		})
	}
	tt = ui.NewTester(view, 1180, 760)
	if err := tt.Click("Add Provider…"); err != nil {
		t.Fatal(err)
	}
	items := tt.Menu()
	want := []string{"OpenAI", "OpenRouter", "Gemini", "Groq", "Together AI", "", "Ollama", "LM Studio", "", "Other Server…"}
	if len(items) != len(want) {
		t.Fatalf("menu %q", items)
	}
	for i := range want {
		if want[i] != "" && items[i] != want[i] {
			t.Errorf("menu %q", items)
			break
		}
	}
	// The account's Ollama opens as it is.
	if err := tt.ChooseMenuItem("Ollama"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if !tt.HasText("Ollama") || !tt.HasText("Delete") {
		t.Errorf("Ollama: %q", tt.Texts())
	}
}

func TestRenderConnectProviderSheet(t *testing.T) {
	_, tt := sheetATester(t, func(m *mainWindow) { m.presentConnectProvider("opencode", "", nil) })
	for _, text := range []string{"Connect OpenCode Zen", "API key", "API base URL", "Leave empty to use OpenCode Zen’s API.", "Connect"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	if !tt.Focused("API key") {
		t.Errorf("the key has no focus")
	}
	tt.Type("sk-secret")
	tt.Frame()
	sheetARender(t, tt, "connect-opencode")
	// The eye shows the key, the keyboard staying in the field.
	if err := tt.Click("Show API key"); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if _, ok := tt.Find("Hide API key"); !ok || !tt.Focused("API key") {
		t.Errorf("after the eye: focused %v", tt.Focused("API key"))
	}
	sheetARender(t, tt, "connect-opencode-shown")
	// The demo has no CLI to check the key with.
	if err := tt.Click("Connect"); err != nil {
		t.Fatal(err)
	}
	time.Sleep(50 * time.Millisecond)
	sheetASettle(tt)
	if !tt.HasText("The Lorca CLI is not running") {
		t.Errorf("after Connect: %q", tt.Texts())
	}

	_, tt = sheetATester(t, func(m *mainWindow) { m.presentConnectProvider("chatgpt", "", nil) })
	for _, text := range []string{"Connect ChatGPT", "Sign in with ChatGPT…", "Sign-in uses the same OAuth flow as the Codex CLI. It needs a ChatGPT subscription."} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	sheetARender(t, tt, "connect-chatgpt")
}
