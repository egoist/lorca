package model

import (
	"encoding/json"
	"fmt"
	"testing"
)

// What the CLI sends, as the app reads it: custom providers in the account's provider statuses and
// as a bot's provider, and a command's card.

func decodeJSON[T any](t *testing.T, text string) T {
	t.Helper()
	var value T
	if err := json.Unmarshal([]byte(text), &value); err != nil {
		t.Fatal(err)
	}
	return value
}

func TestStatusesKeepCustomProviders(t *testing.T) {
	providers := ToProviders(decodeJSON[[]WireProvider](t, `[
		{"kind": "deepseek", "is_connected": true, "detail": "sk-live…4f2c"},
		{"kind": "mistral", "is_connected": true, "detail": "sk-…0000"},
		{"kind": "custom:openrouter", "is_connected": true, "detail": "sk-…abcd · https://openrouter.ai/api/v1", "base_url": "https://openrouter.ai/api/v1",
		 "name": "OpenRouter", "api": "chat-completions",
		 "models": [{"id": "anthropic/claude-sonnet-5", "name": "Anthropic: Claude Sonnet 5", "context_window": 1000000, "max_output": 128000, "images": true}, {"id": "qwen3:8b"}]},
		{"kind": "custom:lab", "is_connected": true, "detail": "http://lab:8000", "base_url": "http://lab:8000", "name": "Lab", "api": "realtime"}
	]`))
	var kinds []string
	for _, provider := range providers {
		kinds = append(kinds, provider.Kind)
	}
	expect(t, kinds, []string{"deepseek", "custom:openrouter", "custom:lab"})
	expect(t, providers[0], ProviderCredential{Kind: "deepseek", IsConnected: true, Detail: "sk-live…4f2c"})
	yes := true
	expect(t, providers[1], ProviderCredential{
		Kind: "custom:openrouter", IsConnected: true, Detail: "sk-…abcd · https://openrouter.ai/api/v1", BaseURL: "https://openrouter.ai/api/v1",
		Name: "OpenRouter", API: APIChatCompletions,
		Models: []CustomModel{{ID: "anthropic/claude-sonnet-5", Name: "Anthropic: Claude Sonnet 5", ContextWindow: 1_000_000, MaxOutput: 128_000, Images: &yes}, {ID: "qwen3:8b"}},
	})
	// An API this build does not know is left for the sheet to pick; no models is an empty list.
	expect(t, []any{providers[2].Name, providers[2].API, len(providers[2].Models)}, []any{"Lab", CustomAPI(""), 0})
	expect(t, len(ToProviders(nil)), 0)
}

func TestAutoReviewKeepsItsModels(t *testing.T) {
	picked := ToAutoReview(decodeJSON[*WireAutoReview](t, `{"is_enabled": true, "rules": [], "provider": "custom:typesafe",
		"models": {"custom:typesafe": "jev-1.13", "anthropic": "claude-opus-5", "mistral": "m", "deepseek": ""}}`))
	expect(t, picked.Provider, "custom:typesafe")
	expect(t, picked.Models, map[ProviderKind]string{"custom:typesafe": "jev-1.13", "anthropic": "claude-opus-5"})
	// Absent, and a kind this build does not know, are the bot's own provider.
	own := ToAutoReview(decodeJSON[*WireAutoReview](t, `{"is_enabled": false, "rules": []}`))
	expect(t, []any{own.IsEnabled, own.Provider, len(own.Models)}, []any{false, "", 0})
	unknown := ToAutoReview(decodeJSON[*WireAutoReview](t, `{"is_enabled": true, "provider": "mistral"}`))
	expect(t, unknown.Provider, "")
	// The catalog marks its decision models, and a decision provider keeps its API; each status
	// names the provider's default review model.
	models := ToModels(decodeJSON[[]WireModel](t, `[{"provider": "opencode", "id": "jev-1.13", "name": "Jev 1.13", "levels": [], "decides": true},
		{"provider": "opencode", "id": "kimi-k3", "name": "Kimi K3", "levels": ["max"]}]`))
	expect(t, []bool{models[0].Decides, models[1].Decides}, []bool{true, false})
	providers := ToProviders(decodeJSON[[]WireProvider](t, `[{"kind": "deepseek", "is_connected": true, "detail": "", "review_model": "deepseek-flash"},
		{"kind": "custom:typesafe", "is_connected": true, "detail": "", "name": "TypeSafe", "api": "system-one", "review_model": "jev-1.13", "models": [{"id": "jev-1.13", "levels": []}]},
		{"kind": "custom:empty", "is_connected": true, "detail": "", "name": "Empty", "api": "chat-completions", "models": []}]`))
	expect(t, []any{providers[1].API, providers[1].Decides()}, []any{APISystemOne, true})
	expect(t, []string{providers[0].ReviewModel, providers[1].ReviewModel, providers[2].ReviewModel}, []string{"deepseek-flash", "jev-1.13", ""})
}

func TestListedModelKeepsWhatItsServerSaid(t *testing.T) {
	yes := true
	expect(t, ToCustomModel(decodeJSON[WireCustomModel](t, `{"id": "llava:13b", "name": null, "context_window": 4096, "max_output": null, "images": true}`)),
		CustomModel{ID: "llava:13b", ContextWindow: 4096, Images: &yes})
}

func TestBotKeepsItsCustomProvider(t *testing.T) {
	wire := `{"id": "bot-lab", "name": "Lab", "description": "", "symbol_name": "sparkles", "accent": "blue", "runner_id": "dev-workbench", "provider": %q, "model": %s, "thinking": %q, "created_at": 1}`
	bot := ToBot(decodeJSON[WireBot](t, sprintf(wire, "custom:openrouter", `"qwen3:8b"`, "low")))
	expect(t, []string{bot.Provider, bot.Model, bot.Thinking}, []string{"custom:openrouter", "qwen3:8b", "low"})
	gone := ToBot(decodeJSON[WireBot](t, sprintf(wire, "custom:gone", "null", "")))
	expect(t, []string{gone.Provider, gone.Model, gone.Thinking}, []string{"custom:gone", "", ""})
	expect(t, ToBot(decodeJSON[WireBot](t, sprintf(wire, "mistral", "null", ""))).Provider, "deepseek")
}

func TestRunInBackgroundWhileTheCallWaits(t *testing.T) {
	row := func(isRunning bool, run string) *Message {
		return ToMessage(decodeJSON[WireMessage](t, sprintf(`{"id": "m1", "chat_id": "c1", "author": {"kind": "bot", "bot_id": "b1"},
			"body": {"kind": "tool", "name": "bash", "summary": "Running", "is_running": %v, "run": {"session_id": "bash-1a2b3c", "command": "npm run dev", "state": "running"%s}},
			"state": {"kind": "complete"}, "created_at": 1}`, isRunning, run)))
	}
	waitedOn := row(true, "")
	expect(t, []bool{waitedOn.Body.Tool.Run.Background, RunsInForeground(waitedOn)}, []bool{false, true})
	sent := row(false, `, "background": true`)
	expect(t, []bool{sent.Body.Tool.Run.Background, RunsInForeground(sent)}, []bool{true, false})
	// Started in the background, during its first two seconds.
	expect(t, RunsInForeground(row(true, `, "background": true`)), false)
	// Its call returned; the bot is no longer waiting on it.
	expect(t, RunsInForeground(row(false, "")), false)
	checking := ToMessage(decodeJSON[WireMessage](t, `{"id": "m1", "chat_id": "c1", "author": {"kind": "bot", "bot_id": "b1"},
		"body": {"kind": "tool", "name": "bash", "summary": "Running", "is_running": true, "run": {"session_id": null, "command": "npm run dev", "state": "checking"}},
		"state": {"kind": "complete"}, "created_at": 1}`))
	expect(t, RunsInForeground(checking), false)
}

func sprintf(format string, args ...any) string { return fmt.Sprintf(format, args...) }
