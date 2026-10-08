package main

import (
	"encoding/json"
	"os"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func attentionFixture(t *testing.T) {
	t.Helper()
	raw, err := os.ReadFile("../.github/evidence/issue-76/fixture.json")
	if err != nil {
		t.Fatal(err)
	}
	var fixture struct {
		Attention model.AttentionView
		Bots      []struct{ ID, Name string }
		Chats     []struct{ ID, Title, Kind string }
	}
	if err := json.Unmarshal(raw, &fixture); err != nil {
		t.Fatal(err)
	}
	for _, bot := range fixture.Bots {
		if store.Bot(bot.ID) != nil {
			continue
		}
		store.Bots = append(store.Bots, &model.Bot{ID: bot.ID, Name: bot.Name, SymbolName: "sparkles", Accent: "indigo", Provider: "deepseek", RunnerID: "dev-workbench"})
	}
	for _, chat := range fixture.Chats {
		if store.Chat(chat.ID) != nil {
			continue
		}
		kind := model.ChatGroup
		if chat.Kind == "dm" {
			kind = model.ChatDM
		}
		store.Chats = append(store.Chats, &model.Chat{ID: chat.ID, Kind: kind, CustomTitle: chat.Title, BotIDs: []string{"chef-demo"}, CreatedAt: time.Now()})
	}
	store.Attention = fixture.Attention
}
func TestAttentionSheetNativeActionsAndFrames(t *testing.T) {
	m := demoWindow(t)
	attentionFixture(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 1000)
	settle(tt)
	if err := tt.Click(L("Attention")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	for _, text := range []string{L("Attention"), L("Coordinator summaries"), L("Urgent direct alerts"), L("Chef’s brief"), "Preview build needs a decision", L("Next: %@", "Choose the Friday preview target.")} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "desktop-attention-active")
	sheetAClickEnd(t, tt, L("Coordinator summaries"))
	settle(tt)
	if store.Attention.Preferences.Summaries || !store.Attention.Preferences.UrgentDirect {
		t.Fatal("summary switch changed wrong preference")
	}
	// Later frames and an unrelated model event keep the edited switch state.
	settle(tt)
	if store.Attention.Preferences.Summaries {
		t.Fatal("preference edit reset on redraw")
	}
	sheetAClickEnd(t, tt, L("Default coordinator"))
	if err := tt.ChooseMenuItem("Scout"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Attention.Preferences.DefaultCoordinatorBotID == nil || *store.Attention.Preferences.DefaultCoordinatorBotID != "scout-demo" {
		t.Fatal("another ordinary bot was not selected")
	}
	for _, id := range []string{"demo-review", "demo-blocker"} {
		var item model.AttentionItem
		for _, each := range store.Attention.Items {
			if each.ID == id {
				item = each
			}
		}
		store.ResolveAttention(item.ID, item.Revision, nil)
	}
	settle(tt)
	renderBoth(t, tt, "desktop-attention-followups")
	store.Attention.Items = nil
	settle(tt)
	if !tt.HasText(L("Nothing needs attention")) {
		t.Fatal(tt.Texts())
	}
	renderBoth(t, tt, "desktop-attention-resolved")
	if err := tt.Click(L("Open brief")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.hasSheet() || m.selection.ChatID != "chef-dm-demo" {
		t.Fatal("source link did not close sheet and navigate")
	}
}
func TestAttentionResolveButtonAndSource(t *testing.T) {
	m := demoWindow(t)
	attentionFixture(t)
	store.Attention.Briefs = nil
	store.Attention.Items = store.Attention.Items[1:2]
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settle(tt)
	m.presentAttention()
	settleTransitions(tt)
	if err := tt.Click(L("Mark resolved") + " · Review the short launch draft"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if len(store.Attention.Items) != 0 || !tt.HasText(L("Nothing needs attention")) {
		t.Fatal("resolve left active item")
	}
	// Populate a fresh source-linked item while the same sheet is alive.
	attentionFixture(t)
	store.Attention.Briefs = nil
	store.Attention.Items = store.Attention.Items[1:2]
	settle(tt)
	if err := tt.Click("Orchard launch · demo · review-demo-v1"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.selection.ChatID != "launch-demo" || m.hasSheet() {
		t.Fatal("source chat navigation failed")
	}
}
func TestAttentionMenuCommand(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settle(tt)
	m.run("attention")
	settleTransitions(tt)
	if !m.hasSheet() || !tt.HasText(L("Nothing needs attention")) {
		t.Fatal(tt.Texts())
	}
	if err := tt.Click(L("Done")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.hasSheet() {
		t.Fatal("Done did not close attention")
	}
}
