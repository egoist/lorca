package main

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A secret request reads from the CLI's shape, as the macOS app reads it.
func TestSecretRequestDecodes(t *testing.T) {
	var wire model.WireMessage
	raw := `{"id":"msg-1","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},"state":{"kind":"complete"},"created_at":1,
		"body":{"kind":"permission","plugin_id":"playwright","plugin_name":"Browser","tool":"secret","summary":"GitHub password","decision":"pending",
		"reason":"To open the pull request.","secret":{"use":"browser","site":"github.com","fields":[{"name":"password","label":"GitHub password"}]}}}`
	if err := json.Unmarshal([]byte(raw), &wire); err != nil {
		t.Fatal(err)
	}
	message := model.ToMessage(wire)
	request := message.Body.Request
	if note := notificationFor(message); note == nil || note.body != "Asks for GitHub password" {
		t.Errorf("notification %+v", note)
	}
	if !request.IsSecret() || request.Secret.Site != "github.com" || request.Secret.Fields[0] != (model.SecretField{Name: "password", Label: "GitHub password"}) {
		t.Fatalf("not read as a secret request: %+v", request.Secret)
	}
	if got := request.VerbPhrase(); got != "needs a secret for github.com" {
		t.Errorf("verb phrase %q", got)
	}
	request.Decision = model.DecisionAllowed
	if got := secretCaption(request); got != "Saved · GitHub password" {
		t.Errorf("answered caption %q", got)
	}
}

// askForSecret puts a bot's secret request at the end of the Developer's chat.
func askForSecret() {
	store.Append(&model.Message{ID: "msg-ask", Author: model.You, Body: model.Body{Kind: model.BodyText, Text: "Publish the CLI package to npm once the review is done."}, State: model.MessageState{Kind: model.StateComplete}, CreatedAt: time.Now().Add(-time.Minute)}, "chat-patch")
	store.Append(&model.Message{
		ID:     "msg-secret",
		Author: model.BotAuthor("bot-patch"),
		Body: model.Body{Kind: model.BodyPermission, Request: &model.PermissionRequest{
			PluginID: "computer", PluginName: "Studio", Tool: "secret", Summary: "npm token", Decision: model.DecisionPending,
			Reason: "To publish the package to npm for you.",
			Secret: &model.SecretAsk{Use: model.SecretCommand, Fields: []model.SecretField{{Name: "NPM_TOKEN", Label: "npm token"}}},
		}},
		State:     model.MessageState{Kind: model.StateComplete},
		CreatedAt: time.Now(),
	}, "chat-patch")
}

// The card asks for each value, saves only once each has one, and reads Saved after.
func TestSecretCardSaves(t *testing.T) {
	m := demoWindow(t)
	askForSecret()
	m.selectChat("chat-patch")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settle(tt)
	for _, text := range []string{"Developer needs a secret for its commands", "To publish the package to npm for you.", "Saved on Studio. Developer never sees it."} {
		if !tt.HasText(text) {
			t.Fatalf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "secret-card")
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText("Saved on Studio. Developer never sees it.") {
		t.Fatal("Save went with nothing typed")
	}
	if err := tt.Click("npm token"); err != nil {
		t.Fatal(err)
	}
	tt.Type("npm_0123456789")
	settle(tt)
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText("Saved · npm token") {
		t.Fatalf("the card did not read Saved: %q", tt.Texts())
	}
	if tt.HasText("Saved on Studio. Developer never sees it.") {
		t.Error("an answered card keeps its fields")
	}
	renderBoth(t, tt, "secret-card-saved")
}

// Settings lists a Runner's secrets by label, bot, and where each goes; a row opens its sheet.
func TestSettingsSecrets(t *testing.T) {
	m, tt := settingsWindowTester(t, 700)
	m.showSettingsDevice("dev-studio")
	m.showSettings(model.PaneSecrets)
	settle(tt)
	for _, text := range []string{"Secrets on Studio", "GitHub password", "Developer · github.com", "Semantic Scholar API key", "Researcher · Commands"} {
		if !tt.HasText(text) {
			t.Fatalf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "settings-secrets")
	if err := tt.Click("GitHub password"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !m.hasSheet() || !tt.HasText("Developer signs in to github.com with it.") {
		t.Fatalf("no sheet: %q", tt.Texts())
	}
	renderBoth(t, tt, "secret-sheet")
	if err := tt.Click("Delete…"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.Click("Delete"); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if m.hasSheet() || tt.HasText("Developer · github.com") {
		t.Errorf("the secret stayed: %q", tt.Texts())
	}
	// A phone keeps none.
	m.showSettingsDevice("dev-phone")
	settle(tt)
	if tt.HasText("Loading…") {
		t.Error("a phone's secrets were asked for")
	}
}
