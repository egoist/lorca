package main

import (
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func TestRenderChannelsInInspector(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-tally")
	tt := ui.NewTester(m.frame(m.view), 1180, 1500)
	settle(tt)
	if !tt.HasText(L("Channels")) && !tt.HasText("CHANNELS") {
		t.Errorf("no Channels section: %q", tt.Texts())
	}
	if !tt.HasText("Community feedback") || !tt.HasText("Mentions, replies, #feedback") {
		t.Errorf("texts %q", tt.Texts())
	}
	renderBoth(t, tt, "inspector-channels")

	store.SetChannelPaused("ev-feedback", true)
	settle(tt)
	if !store.Channel("ev-feedback").IsPaused() {
		t.Error("the switch did not pause the channel")
	}
	store.SetChannelPaused("ev-feedback", false)

	// A held message reads in orange before what the channel takes.
	store.Channel("ev-feedback").State, store.Channel("ev-feedback").HeldDelivery = model.ChannelHeld, "d1"
	settle(tt)
	if !tt.HasText(L("On hold")) {
		t.Errorf("no On hold: %q", tt.Texts())
	}
	renderBoth(t, tt, "inspector-channels-held")

	// A bot without channels has no section.
	m.selectChat("chat-nova")
	settle(tt)
	if tt.HasText("Community feedback") {
		t.Error("another bot shows the channel")
	}
}

func TestChannelSheet(t *testing.T) {
	m, tt := sheetTester(t, func(m *mainWindow) {})
	m.presentChannel("ev-feedback", store.Bot("bot-tally"), m.open)
	settle(tt)
	for _, text := range []string{L("Listening"), "Telegram · Community", L("Every chat the bot is in"), "Mentions, replies, #feedback", L("Conversations"), "Acme Community"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "channel-sheet")
	closeSheet(t, m, tt)

	store.Channel("ev-feedback").State, store.Channel("ev-feedback").HeldDelivery = model.ChannelHeld, "d1"
	m.presentChannel("ev-feedback", store.Bot("bot-tally"), m.open)
	settle(tt)
	renderBoth(t, tt, "channel-sheet-held")
	if err := tt.Click(L("Skip")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if channel := store.Channel("ev-feedback"); channel.State != model.ChannelListening || channel.HeldDelivery != "" {
		t.Errorf("Skip left %+v", channel)
	}
	closeSheet(t, m, tt)

	// A conversation's row opens it.
	m.presentChannel("ev-feedback", store.Bot("bot-tally"), m.open)
	settle(tt)
	// The sidebar's row of the same name comes first in build order: click the sheet's, the last
	// row of its Conversations section.
	section, ok := tt.Find(L("Conversations"))
	if !ok {
		t.Fatal("no Conversations section")
	}
	tt.ClickAt(section.X+60, section.Y+section.H-16)
	settleTransitions(tt)
	if m.hasSheet() || m.selection.ChatID != "chat-community" {
		t.Errorf("the row left the sheet up or opened %q", m.selection.ChatID)
	}
}

func TestRenderChannelConversation(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-community")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	for _, text := range []string{"Acme Community", "Alice Chen", "Ben Ortiz", "Thanks Alice, tracked in #142."} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "channel-conversation")

	// The conversation is only a transcript: deleting it keeps the bot and its DM.
	if store.DM("bot-tally") != "chat-tally" {
		t.Errorf("the bot's DM is %q", store.DM("bot-tally"))
	}
	store.DeleteChat("chat-community")
	if store.Bot("bot-tally") == nil || store.Chat("chat-tally") == nil {
		t.Error("deleting a channel's conversation deleted its bot")
	}
}
