package main

import (
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
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

// testFlyout stands in for the attention flyout: its content draws in a tester of its own.
type testFlyout struct {
	tt            *ui.Tester
	closed        func()
	isShut        bool
	width, height int
}

func (f *testFlyout) Close() {
	if !f.isShut {
		f.isShut = true
		f.closed()
	}
}
func (f *testFlyout) Invalidate() {}
func (f *testFlyout) SetContentSize(width, height int) {
	f.width, f.height = width, height
}

// stubFlyouts makes the flyouts the window opens testFlyouts; the last one opened is the slice's.
func stubFlyouts(t *testing.T) *[]*testFlyout {
	var opened []*testFlyout
	saved := openFlyout
	openFlyout = func(_ *mygo.Window, _ ui.Rect, width, height int, view func(c *ui.Context), closed func()) flyoutWindow {
		f := &testFlyout{tt: ui.NewTester(view, width, height), closed: closed, width: width, height: height}
		f.tt.SetScale(2)
		opened = append(opened, f)
		return f
	}
	t.Cleanup(func() { openFlyout = saved })
	return &opened
}

// settleFlyout draws the flyout until it is fitted to its content.
func settleFlyout(f *testFlyout) {
	for range 3 {
		settle(f.tt)
		f.tt.SetSize(f.width, f.height)
	}
	settle(f.tt)
}

func TestAttentionPopover(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
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
	if len(*flyouts) != 1 {
		t.Fatalf("%d flyouts open", len(*flyouts))
	}
	f := (*flyouts)[0]
	settleFlyout(f)
	if f.height <= 120 {
		t.Fatalf("height %d", f.height)
	}
	ft := f.tt
	for _, text := range []string{L("Attention"), "Project Manager", "Ship the relay with TLS on by default.", L("Decision"), "Staging certificate expires tonight", "Approve the renewal so DevOps can run it.", L("Urgent"), " · Launch copy, Writer"} {
		if !ft.HasText(text) {
			t.Errorf("no %q in %q", text, ft.Texts())
		}
	}
	for _, text := range []string{"task-", "review-"} {
		for _, shown := range ft.Texts() {
			if shown == text {
				t.Errorf("the flyout shows %q", text)
			}
		}
	}
	// The item from two chats opens either; the others open theirs from the row.
	for _, text := range []string{L("Mark as Resolved"), L("Dismiss"), L("Open “%@”", "Launch copy"), L("Open “%@”", "Writer")} {
		if !ft.HasText(text) {
			t.Errorf("no %q button in %q", text, ft.Texts())
		}
	}
	renderBoth(t, ft, "attention-popover")

	// The ⋯ menu: a switch per notification, and the coordinator.
	if err := ft.Click(L("Options")); err != nil {
		t.Fatal(err)
	}
	ft.Frame()
	if err := ft.ChooseMenuItem(L("Briefs")); err != nil {
		t.Fatalf("%v in %q", err, ft.Menu())
	}
	settle(tt)
	if store.Attention.Preferences.Summaries || !store.Attention.Preferences.UrgentDirect {
		t.Fatalf("Briefs changed the wrong preference: %+v", store.Attention.Preferences)
	}
	if err := ft.Click(L("Options")); err != nil {
		t.Fatal(err)
	}
	ft.Frame()
	if err := ft.ChooseMenuItem("Writer"); err != nil {
		t.Fatalf("%v in %q", err, ft.Menu())
	}
	settle(tt)
	if id := store.Attention.Preferences.DefaultCoordinatorBotID; id == nil || *id != "bot-quill" {
		t.Fatal("Writer is not the coordinator")
	}

	// A row's button marks it resolved.
	if err := ft.Click(L("Mark as Resolved")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	settleFlyout(f)
	if len(store.Attention.Items) != 3 || ft.HasText("Staging certificate expires tonight") {
		t.Fatal("the item stayed")
	}

	// A brief's button dismisses it.
	if err := ft.Click(L("Dismiss")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	settleFlyout(f)
	if len(store.Attention.Briefs) != 0 || ft.HasText("Ship the relay with TLS on by default.") {
		t.Fatal("the brief stayed")
	}

	// A source chat's button opens it and closes the flyout.
	if err := ft.Click(L("Open “%@”", "Writer")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.selection.ChatID != "chat-quill" || m.attention.open || !f.isShut {
		t.Fatalf("selection %q, open %v", m.selection.ChatID, m.attention.open)
	}

	// So does a click on a row, with its chat.
	m.run("attention")
	settle(tt)
	f = (*flyouts)[len(*flyouts)-1]
	settleFlyout(f)
	if err := f.tt.Click("Go/no-go moved to Friday"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.selection.ChatID != "chat-relay" || m.attention.open || !f.isShut {
		t.Fatalf("selection %q, open %v", m.selection.ChatID, m.attention.open)
	}
}

// A row waiting on a review opens the review's chat and its sheet; once the review is decided the
// row opens only the chat.
func TestAttentionRowOpensItsReview(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	m.selectChat("chat-relay")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	tt.SetScale(2)
	attentionFixture()
	store.Attention.Items[1].Sources = []model.AttentionSource{{ChatID: "chat-launch"}, {ChatID: "chat-nova", ReviewID: "review-comment"}}
	settle(tt)
	m.run("attention")
	settle(tt)
	settleFlyout((*flyouts)[len(*flyouts)-1])
	if err := (*flyouts)[len(*flyouts)-1].tt.Click("Release notes draft"); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if m.selection.ChatID != "chat-nova" || m.attention.open || !m.hasSheet() {
		t.Fatalf("selection %q, open %v, sheet %v", m.selection.ChatID, m.attention.open, m.hasSheet())
	}
	tt.Key(0, ui.KeyEscape)
	settleTransitions(tt)
	if m.hasSheet() {
		t.Fatal("Escape left the sheet up")
	}
	store.Review("review-comment").State = "rejected"
	m.run("attention")
	settle(tt)
	settleFlyout((*flyouts)[len(*flyouts)-1])
	if err := (*flyouts)[len(*flyouts)-1].tt.Click("Release notes draft"); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if m.selection.ChatID != "chat-launch" || m.hasSheet() {
		t.Fatalf("decided: selection %q, sheet %v", m.selection.ChatID, m.hasSheet())
	}
}

func TestAttentionMenuCommandWithNothingWaiting(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	m.selectChat("chat-relay")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	tt.SetScale(2)
	settle(tt)
	m.run("attention")
	settle(tt)
	if len(*flyouts) != 1 {
		t.Fatalf("%d flyouts open", len(*flyouts))
	}
	f := (*flyouts)[0]
	settleFlyout(f)
	if !f.tt.HasText(L("Nothing needs your attention")) {
		t.Fatal(f.tt.Texts())
	}
	renderBoth(t, f.tt, "attention-empty")
	m.run("attention")
	settle(tt)
	if m.attention.open || !f.isShut {
		t.Fatal("the command did not close the popover")
	}
}

// A row opens its chat on the message the item cites, which pulses; so does a source chat's
// button, on that chat's message.
func TestAttentionRowRevealsItsMessage(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	m.selectChat("chat-relay")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	tt.SetScale(2)
	attentionFixture()
	cited := store.Chat("chat-launch").Messages[1].ID
	store.Attention.Items[1].Sources = []model.AttentionSource{{ChatID: "chat-launch", MessageID: cited}, {ChatID: "chat-quill"}}
	settle(tt)
	m.run("attention")
	settle(tt)
	f := (*flyouts)[len(*flyouts)-1]
	settleFlyout(f)
	if err := f.tt.Click("Release notes draft"); err != nil {
		t.Fatal(err)
	}
	// The pulse starts with the click and lasts 1.1 seconds of the clock, which a slow frame of the
	// newly shown chat can outlast: look for it before the window draws.
	if flash := m.chat.flashID; flash != cited {
		t.Fatalf("flash %q", flash)
	}
	settle(tt)
	if m.selection.ChatID != "chat-launch" || !f.isShut {
		t.Fatalf("selection %q, shut %v", m.selection.ChatID, f.isShut)
	}
	renderTo(t, tt, "attention-reveal")

	// A chat the item cites no message of opens at its end.
	m.run("attention")
	settle(tt)
	f = (*flyouts)[len(*flyouts)-1]
	settleFlyout(f)
	if err := f.tt.Click(L("Open “%@”", "Writer")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.selection.ChatID != "chat-quill" || m.chat.flashID != "" {
		t.Fatalf("selection %q, flash %q", m.selection.ChatID, m.chat.flashID)
	}
}

// A cited message the transcript draws no row for reveals the nearest one before it, else after.
func TestAttentionShownMessage(t *testing.T) {
	a, b, c := &model.Message{ID: "a"}, &model.Message{ID: "b"}, &model.Message{ID: "c"}
	chat := &model.Chat{Messages: []*model.Message{a, b, c}}
	rows := []chatRow{{kind: rowMessage, message: a}, {kind: rowMessage, message: c}}
	for cited, want := range map[string]string{"a": "a", "b": "a", "c": "c", "gone": "gone"} {
		if got := shownMessage(rows, chat, cited); got != want {
			t.Errorf("%s: %s, want %s", cited, got, want)
		}
	}
	if got := shownMessage(rows[1:], chat, "b"); got != "c" {
		t.Errorf("nothing before: %s", got)
	}
}

// The flyout opens at its list's height, measured before it shows, and stays there.
func TestAttentionFlyoutOpensAtItsHeight(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	attentionFixture()
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settle(tt)
	m.run("attention")
	settle(tt)
	f := (*flyouts)[0]
	opened := f.height
	settleFlyout(f)
	if opened < 200 || f.height != opened {
		t.Fatalf("opened at %d, fitted to %d", opened, f.height)
	}
}
