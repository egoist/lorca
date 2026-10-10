package main

import (
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A bot's email waits as a card the user edits, then sends; a Slack card's Always Send also has
// the bot send directly from now on.
func TestDraftCards(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-quill")
	m.userWantsInspector = false
	tt := ui.NewTester(m.frame(m.view), 1100, 900)
	settle(tt)
	if !tt.HasText(L("%@ drafted an email", "Writer")) || !tt.HasText("launch-note.pdf") {
		t.Fatalf("no email card in %q", tt.Texts())
	}
	renderBoth(t, tt, "draft-email")
	if err := tt.Click(L("Remove %@", "launch-note.pdf")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if tt.HasText("launch-note.pdf") {
		t.Errorf("the attachment is still there")
	}
	if err := tt.Click(L("Send")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	card := store.Chat("chat-quill").Messages
	sent := card[len(card)-2].Body.Draft
	if sent == nil || sent.State != "succeeded" || len(sent.Fields.Attachments) != 0 {
		t.Fatalf("the email was not sent as edited: %+v", sent)
	}
	if !tt.HasText(L("Sent")) || tt.HasText(L("Discard")) {
		t.Errorf("the sent card still offers its buttons: %q", tt.Texts())
	}
	renderBoth(t, tt, "draft-email-sent")

	m.selectChat("chat-launch")
	settle(tt)
	renderBoth(t, tt, "draft-slack")
	if err := tt.Click(L("Always Send")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if bot := store.Bot("bot-nova"); bot.Permissions == nil || bot.Permissions.Drafts {
		t.Errorf("Always Send left drafts on for %s", bot.Name)
	}
	if !tt.HasText(L("Sent")) {
		t.Errorf("the Slack card was not sent: %q", tt.Texts())
	}
	_ = model.BodyDraft
}
