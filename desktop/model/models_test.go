package model

import (
	"reflect"
	"slices"
	"testing"
)

// Custom providers in the model: their kinds, names, models, and thinking levels; the presets and
// the base URL the custom provider sheet reads; and its model checklist.

var ollama = ProviderCredential{
	Kind: "custom:ollama", IsConnected: true, Detail: "http://localhost:11434/v1", BaseURL: "http://localhost:11434/v1", Name: "Ollama", API: APIChatCompletions,
	Models: []CustomModel{
		{ID: "qwen3:8b", Levels: []string{"low", "medium", "high"}},
		{ID: "anthropic/claude-sonnet-5", Name: "Anthropic: Claude Sonnet 5", Levels: []string{"off", "low", "medium", "high", "xhigh", "max"}},
	},
}

func expect[T any](t *testing.T, got, want T) {
	t.Helper()
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got %#v, want %#v", got, want)
	}
}

func TestCustomKinds(t *testing.T) {
	expect(t, []bool{IsProviderKind("deepseek"), IsProviderKind("custom:openrouter"), IsCustomKind("custom:openrouter"), IsCustomKind("deepseek"), IsProviderKind("mistral")},
		[]bool{true, true, true, false, false})
}

func TestCustomProviderNames(t *testing.T) {
	expect(t, ProviderName("opencode", nil), "OpenCode Zen")
	expect(t, ProviderName("opencode-go", []ProviderCredential{ollama}), "OpenCode Go")
	expect(t, ProviderName("custom:ollama", []ProviderCredential{ollama}), "Ollama")
	expect(t, ProviderName("custom:my-lab", []ProviderCredential{ollama}), "my-lab")
}

func levelIDs(levels []Choice) []string {
	ids := make([]string, 0, len(levels))
	for _, level := range levels {
		ids = append(ids, level.ID)
	}
	return ids
}

func TestCustomProviderModels(t *testing.T) {
	catalog := WithCustomModels([]ProviderModel{{"grok", "grok-4.7", "Grok 4.7", []string{"low", "medium", "high", "xhigh"}}}, []ProviderCredential{ollama})
	expect(t, ProviderModels(catalog, "custom:ollama"), []ProviderModel{
		{"custom:ollama", "qwen3:8b", "qwen3:8b", []string{"low", "medium", "high"}},
		{"custom:ollama", "anthropic/claude-sonnet-5", "Anthropic: Claude Sonnet 5", []string{"off", "low", "medium", "high", "xhigh", "max"}},
	})
	expect(t, len(ProviderModels(catalog, "custom:gone")), 0)
	expect(t, ProviderModels(catalog, "grok")[0].Label, "Grok 4.7")
	expect(t, levelIDs(ThinkingLevels(catalog, "custom:ollama", "")), []string{"low", "medium", "high"})
	expect(t, levelIDs(ThinkingLevels(catalog, "custom:ollama", "anthropic/claude-sonnet-5")), []string{"off", "low", "medium", "high", "xhigh", "max"})
}

func TestProviderRows(t *testing.T) {
	expect(t, []any{ProviderSymbol("custom:ollama"), ProviderSubtitle("custom:ollama"), UsesAPIKey("custom:ollama")}, []any{"server.rack", "Custom", false})
	expect(t, []any{ProviderSymbol("anthropic"), ProviderSubtitle("anthropic"), UsesAPIKey("anthropic")}, []any{"key.fill", "API key", true})
	expect(t, []any{ProviderSymbol("grok"), ProviderSubtitle("grok"), UsesAPIKey("grok")}, []any{"person.badge.key.fill", "Subscription", false})
}

func TestCustomEndpoint(t *testing.T) {
	cases := []struct {
		api       CustomAPI
		base, url string
	}{
		{APIMessages, "https://api.anthropic.com/v1", "https://api.anthropic.com/v1/messages"},
		{APIMessages, "https://api.anthropic.com/v1/messages", "https://api.anthropic.com/v1/messages"},
		{APIMessages, "https://api.anthropic.com", "https://api.anthropic.com/v1/messages"},
		{APIChatCompletions, "http://localhost:11434/v1/chat/completions", "http://localhost:11434/v1/chat/completions"},
		{APIChatCompletions, " http://localhost:11434/v1// ", "http://localhost:11434/v1/chat/completions"},
		{APIResponses, "https://x/v1/", "https://x/v1/responses"},
		{APIResponses, "https://x/v1/responses/", "https://x/v1/responses"},
		// Only the API's own path is cut, once.
		{APIChatCompletions, "https://x/v1/responses", "https://x/v1/responses/chat/completions"},
		{APIMessages, "https://x/v1/v1", "https://x/v1/v1/messages"},
	}
	for _, test := range cases {
		expect(t, CustomEndpoint(test.api, test.base), test.url)
	}
	expect(t, CustomEndpointNote(APIChatCompletions, ""), "Lorca adds /chat/completions to it.")
	expect(t, CustomEndpointNote(APIMessages, "   "), "Lorca adds /v1/messages to it.")
	expect(t, CustomEndpointNote(APIResponses, "https://openrouter.ai/api/v1/"), "Requests go to https://openrouter.ai/api/v1/responses.")
	expect(t, CustomBaseURLPlaceholder(APIMessages), "https://api.example.com")
	expect(t, CustomBaseURLPlaceholder(APIResponses), "https://api.example.com/v1")
}

func TestPresets(t *testing.T) {
	var got [][]any
	var placeholders []string
	for _, preset := range CustomPresets {
		got = append(got, []any{preset.Name, preset.API, preset.BaseURL, preset.Local})
		placeholders = append(placeholders, preset.KeyPlaceholder())
	}
	expect(t, got, [][]any{
		{"OpenAI", APIResponses, "https://api.openai.com/v1", false},
		{"OpenRouter", APIChatCompletions, "https://openrouter.ai/api/v1", false},
		{"Gemini", APIChatCompletions, "https://generativelanguage.googleapis.com/v1beta/openai", false},
		{"Groq", APIChatCompletions, "https://api.groq.com/openai/v1", false},
		{"Together AI", APIChatCompletions, "https://api.together.xyz/v1", false},
		{"Ollama", APIChatCompletions, "http://localhost:11434/v1", true},
		{"LM Studio", APIChatCompletions, "http://localhost:1234/v1", true},
	})
	expect(t, placeholders, []string{
		"sk-… from platform.openai.com", "sk-or-… from openrouter.ai/keys", "Key from aistudio.google.com", "gsk_… from console.groq.com",
		"Key from api.together.ai", "Optional for a server on your network", "Optional for a server on your network",
	})
}

func TestPresetProviderAndBaseURLs(t *testing.T) {
	preset := func(name string) CustomPreset {
		return CustomPresets[slices.IndexFunc(CustomPresets, func(p CustomPreset) bool { return p.Name == name })]
	}
	mine := ollama
	mine.Kind, mine.Name = "custom:openrouter", "openrouter"
	expect(t, PresetProvider(preset("OpenRouter"), []ProviderCredential{ollama, mine}).Kind, "custom:openrouter")
	expect(t, PresetProvider(preset("Ollama"), []ProviderCredential{ollama}).Kind, "custom:ollama")
	expect(t, PresetProvider(preset("OpenRouter"), []ProviderCredential{ollama}) == nil, true)

	expect(t, MatchingPreset("http://localhost:11434/v1").Name, "Ollama")
	expect(t, MatchingPreset(" http://localhost:1234 ").Name, "LM Studio")
	expect(t, MatchingPreset("https://openrouter.ai/api/v1/chat/completions").Name, "OpenRouter")
	expect(t, MatchingPreset("http://localhost:8080/v1") == nil, true)
	expect(t, MatchingPreset("api.openai.com") == nil, true)
	expect(t, SuggestedProviderName("http://localhost:11434/v1"), "Ollama")
	expect(t, SuggestedProviderName("https://llm.example.com:8443/v1"), "llm.example.com")
	expect(t, SuggestedProviderName(""), "")
	expect(t, CustomHost("https://api.example.com/v1"), "api.example.com")
	expect(t, CustomHost("not a url"), "")
	for url, want := range map[string]bool{"http://localhost:11434/v1": true, " https://api.example.com ": true, "http://": false, "ftp://example.com": false, "localhost:11434": false, "": false} {
		if IsUsableBaseURL(url) != want {
			t.Errorf("IsUsableBaseURL(%q) != %v", url, want)
		}
	}
}

func checklistIDs(c ModelChecklist) []string {
	var ids []string
	for _, model := range c.Models {
		ids = append(ids, model.ID)
	}
	return ids
}

func picked(c ModelChecklist) []string {
	var ids []string
	for id, on := range c.Selected {
		if on {
			ids = append(ids, id)
		}
	}
	slices.Sort(ids)
	return ids
}

func models(ids ...string) []CustomModel {
	var out []CustomModel
	for _, id := range ids {
		out = append(out, CustomModel{ID: id})
	}
	return out
}

func TestSavedChecklist(t *testing.T) {
	saved := SavedChecklist(models("b", "a"))
	expect(t, []any{checklistIDs(saved), picked(saved), saved.DefaultID}, []any{[]string{"b", "a"}, []string{"a", "b"}, "b"})
	empty := SavedChecklist(nil)
	expect(t, []any{len(empty.Models), len(empty.Selected), empty.DefaultID}, []any{0, 0, ""})
}

func TestListingKeepsPickedAndTyped(t *testing.T) {
	var nine []CustomModel
	for i := range 9 {
		nine = append(nine, CustomModel{ID: "m" + string(rune('0'+i))})
	}
	checklist := SavedChecklist(nil).TakeListing(nine)
	// Nine models: too many to pick them all.
	expect(t, []any{len(checklist.Models), len(checklist.Selected), checklist.DefaultID}, []any{9, 0, ""})
	checklist = checklist.Toggle("m4").Add("mine")
	expect(t, checklistIDs(checklist)[:2], []string{"mine", "m0"})
	yes := true
	next := checklist.TakeListing([]CustomModel{{ID: "m4", Name: "Model Four", ContextWindow: 128_000}, {ID: "n1"}, {ID: "n1"}, {ID: "mine", Images: &yes}})
	expect(t, checklistIDs(next), []string{"mine", "m4", "n1"})
	expect(t, next.Models[0], CustomModel{ID: "mine", Images: &yes})
	expect(t, next.Models[1], CustomModel{ID: "m4", Name: "Model Four", ContextWindow: 128_000})
	expect(t, []any{picked(next), next.DefaultID}, []any{[]string{"m4", "mine"}, "m4"})
	// A server that lists nothing keeps the picked and typed models, none of the last server's.
	unlisted := next.Toggle("m4").TakeListing(nil)
	expect(t, []any{checklistIDs(unlisted), picked(unlisted), unlisted.DefaultID}, []any{[]string{"mine"}, []string{"mine"}, "mine"})
}

func TestShortListingStartsPicked(t *testing.T) {
	local := SavedChecklist(nil).TakeListing(models("qwen3:8b", "gemma3:12b", "llava:13b"))
	expect(t, []any{picked(local), local.DefaultID}, []any{[]string{"gemma3:12b", "llava:13b", "qwen3:8b"}, "qwen3:8b"})
	yes := true
	edited := SavedChecklist(models("llava:13b")).TakeListing([]CustomModel{{ID: "qwen3:8b"}, {ID: "llava:13b", Images: &yes}})
	expect(t, []any{checklistIDs(edited), picked(edited), edited.DefaultID}, []any{[]string{"llava:13b", "qwen3:8b"}, []string{"llava:13b"}, "llava:13b"})
	expect(t, *edited.Models[0].Images, true)
}

func TestDefaultFollowsPicks(t *testing.T) {
	checklist := SavedChecklist(models("a", "b", "c")).Toggle("a")
	expect(t, []any{picked(checklist), checklist.DefaultID}, []any{[]string{"b", "c"}, "b"})
	// Picking the old default again leaves the new one.
	checklist = checklist.Toggle("a")
	expect(t, checklist.DefaultID, "b")
	checklist = checklist.Toggle("a").Toggle("b").Toggle("c")
	expect(t, []any{len(checklist.Selected), checklist.DefaultID}, []any{0, ""})
	expect(t, checklist.Toggle("c").DefaultID, "c")
	// The first model added with nothing picked becomes the default.
	expect(t, SavedChecklist(nil).Add("x").DefaultID, "x")
	expect(t, SavedChecklist(models("a")).Add("x").DefaultID, "a")
}

func TestModelSearch(t *testing.T) {
	list := []CustomModel{{ID: "anthropic/claude-sonnet-5", Name: "Anthropic: Claude Sonnet 5"}, {ID: "qwen3:8b"}, {ID: "Qwen3:32b"}}
	expect(t, checklistIDs(ModelChecklist{Models: FilterModels(list, "  ")}), []string{"anthropic/claude-sonnet-5", "qwen3:8b", "Qwen3:32b"})
	expect(t, checklistIDs(ModelChecklist{Models: FilterModels(list, "QWEN")}), []string{"qwen3:8b", "Qwen3:32b"})
	expect(t, checklistIDs(ModelChecklist{Models: FilterModels(list, "sonnet 5")}), []string{"anthropic/claude-sonnet-5"})
	expect(t, AddCandidate(" qwen3 ", list), "qwen3")
	expect(t, AddCandidate("qwen3:8b", list), "")
	expect(t, AddCandidate("QWEN3:8B", list), "QWEN3:8B")
	expect(t, AddCandidate("", list), "")
	expect(t, list[0].DisplayName(), "Anthropic: Claude Sonnet 5")
	expect(t, list[1].DisplayName(), "qwen3:8b")
}

func TestOrderedModelIDs(t *testing.T) {
	checklist := SavedChecklist(models("a", "b", "c", "d")).Toggle("b")
	withDefault := checklist
	withDefault.DefaultID = "c"
	expect(t, withDefault.OrderedIDs(), []string{"c", "a", "d"})
	expect(t, checklist.OrderedIDs(), []string{"a", "c", "d"})
	checklist.DefaultID = ""
	expect(t, checklist.OrderedIDs(), []string{"a", "c", "d"})
}
