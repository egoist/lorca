package main

import (
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// attentionFixture is the demo's team with a brief and four items, as the Mac's probe shows them.
func attentionFixture() {
	revision := func(n uint64) model.AttentionRevision {
		return model.AttentionRevision{Counter: n, DeviceID: "dev-workbench"}
	}
	source := func(ids ...string) []model.AttentionSource {
		var sources []model.AttentionSource
		for _, id := range ids {
			sources = append(sources, model.AttentionSource{ChatID: id})
		}
		return sources
	}
	store.Attention = model.AttentionView{
		Items: []model.AttentionItem{
			{ID: "a1", Category: "blocker", Title: "Staging certificate expires tonight", Summary: "The renewal needs an approval before 6 PM.", NextAction: "Approve the renewal so DevOps can run it.", CoordinatorBotID: "bot-nova", Sources: source("chat-ember"), Urgent: true, Revision: revision(9)},
			{ID: "a2", Category: "review", Title: "Release notes draft", Summary: "Writer and Project Manager both flagged the same draft.", NextAction: "Read the draft and approve it or leave notes.", CoordinatorBotID: "bot-nova", Sources: source("chat-launch", "chat-quill"), Revision: revision(8)},
			{ID: "a3", Category: "commitment", Title: "Website copy due Thursday", Summary: "Writer committed to the final copy.", NextAction: "Writer delivers the final copy by Thursday noon.", CoordinatorBotID: "bot-nova", Sources: source("chat-quill"), Revision: revision(7)},
			{ID: "a4", Category: "change", Title: "Go/no-go moved to Friday", Summary: "The call moved so the TLS rollout can finish first.", NextAction: "Say if Friday at 10 AM doesn’t work for you.", CoordinatorBotID: "bot-nova", Sources: source("chat-relay"), Revision: revision(6)},
		},
		Briefs: []model.AttentionBrief{
			{CoordinatorBotID: "bot-nova", ChatID: "chat-nova", Decisions: []string{"Ship the relay with TLS on by default."}, Changes: []string{"The go/no-go call moves to Friday at 10 AM."}, NextAction: "Approve the release notes draft.", UpdatedAt: float64(time.Now().Add(-10 * time.Minute).Unix())},
		},
		Preferences: model.AttentionPreferences{Summaries: true, UrgentDirect: true},
	}
}

func TestAttentionPopover(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-relay")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	tt.SetScale(2)
	settle(tt)
	if tt.HasText(L("Attention (%d)", 0)) {
		t.Fatal("the button shows with nothing waiting")
	}
	attentionFixture()
	settle(tt)
	renderBoth(t, tt, "attention-toolbar")
	if err := tt.Click(L("Attention (%d)", 4)); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	for _, text := range []string{L("Attention"), "Project Manager", "Ship the relay with TLS on by default.", L("Decision"), "Staging certificate expires tonight", "Approve the renewal so DevOps can run it.", L("Urgent"), " · Launch copy, Writer"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	for _, text := range []string{"task-", "review-", L("Mark as Resolved")} {
		for _, shown := range tt.Texts() {
			if shown == text {
				t.Errorf("the popover shows %q", text)
			}
		}
	}
	renderBoth(t, tt, "attention-popover")

	// The ⋯ menu: a switch per notification, and the coordinator.
	if err := tt.Click(L("Options")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if err := tt.ChooseMenuItem(L("Briefs")); err != nil {
		t.Fatalf("%v in %q", err, tt.Menu())
	}
	settle(tt)
	if store.Attention.Preferences.Summaries || !store.Attention.Preferences.UrgentDirect {
		t.Fatalf("Briefs changed the wrong preference: %+v", store.Attention.Preferences)
	}
	if err := tt.Click(L("Options")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if err := tt.ChooseMenuItem("Writer"); err != nil {
		t.Fatalf("%v in %q", err, tt.Menu())
	}
	settle(tt)
	if id := store.Attention.Preferences.DefaultCoordinatorBotID; id == nil || *id != "bot-quill" {
		t.Fatal("Writer is not the coordinator")
	}

	// A row's menu marks it resolved.
	if err := tt.RightClick("Release notes draft"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if err := tt.ChooseMenuItem(L("Mark as Resolved")); err != nil {
		t.Fatalf("%v in %q", err, tt.Menu())
	}
	settle(tt)
	if len(store.Attention.Items) != 3 || tt.HasText("Release notes draft") {
		t.Fatal("the item stayed")
	}

	// A click on a row opens its chat and closes the popover.
	if err := tt.Click("Staging certificate expires tonight"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.selection.ChatID != "chat-ember" || m.attention.open {
		t.Fatalf("selection %q, open %v", m.selection.ChatID, m.attention.open)
	}
}

func TestAttentionMenuCommandWithNothingWaiting(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-relay")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	tt.SetScale(2)
	settle(tt)
	m.run("attention")
	settle(tt)
	if !tt.HasText(L("Nothing needs your attention")) {
		t.Fatal(tt.Texts())
	}
	renderBoth(t, tt, "attention-empty")
	m.run("attention")
	settle(tt)
	if m.attention.open || tt.HasText(L("Nothing needs your attention")) {
		t.Fatal("the command did not close the popover")
	}
}
