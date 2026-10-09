package main

import (
	"encoding/json"
	"errors"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// feedbackCLI answers like the CLI: `feedback.list` with `list`, a change with `err` or nothing.
// It keeps what each call sent.
type feedbackCLI struct {
	mu    sync.Mutex
	calls []feedbackCLICall
	list  string
	err   error
	posts chan func()
}

type feedbackCLICall struct {
	method string
	params map[string]any
}

func (*feedbackCLI) Reconnect() {}

func (f *feedbackCLI) Request(method string, params any) (json.RawMessage, error) {
	raw, _ := json.Marshal(params)
	var p map[string]any
	_ = json.Unmarshal(raw, &p)
	f.mu.Lock()
	defer f.mu.Unlock()
	f.calls = append(f.calls, feedbackCLICall{method, p})
	switch {
	case method == "feedback.list":
		return json.RawMessage(f.list), nil
	case f.err != nil && len(method) > 9 && method[:9] == "feedback.":
		return nil, f.err
	}
	return json.RawMessage(`{}`), nil
}

func (f *feedbackCLI) last(method string) map[string]any {
	f.mu.Lock()
	defer f.mu.Unlock()
	for _, call := range slices.Backward(f.calls) {
		if call.method == method {
			return call.params
		}
	}
	return nil
}

// wait runs the store's replies and draws until condition holds.
func (f *feedbackCLI) wait(t *testing.T, tt *ui.Tester, condition func() bool) {
	t.Helper()
	for deadline := time.Now().Add(3 * time.Second); time.Now().Before(deadline); time.Sleep(10 * time.Millisecond) {
		for drained := false; !drained; {
			select {
			case fn := <-f.posts:
				fn()
			default:
				drained = true
			}
		}
		tt.Frame()
		if condition() {
			return
		}
	}
	t.Fatalf("timed out: %q", tt.Texts())
}

const feedbackCLIList = `{"feedback":[{"id":"feedback-edit","kind":"edited","origin":{"chat_id":"chat-nova","message_id":"m-brief"},"note":"Lead with what needs my decision.","example":"","target":{"kind":"routine_prompt","id":"rt-brief"},"created_at":1760000100.0}],
 "feedback_count":1,
 "proposals":[{"id":"proposal-1","target":{"kind":"routine_prompt","id":"rt-brief"},"explanation":"You moved decisions to the top of a brief.","diff":"--- current\n+++ proposed\n@@ -1,1 +1,1 @@\n-Summarize the inbox.\n+Open with what needs a decision.\n","diff_hash":"shown-diff","evidence":["feedback-edit"],"state":"pending","created_at":1760000200.0}],
 "revisions":[],"settings":{"review_every_secs":null},
 "targets":[{"target":{"kind":"routine_prompt","id":"rt-brief"},"name":"Morning brief"}]}`

// feedbackTester is Project Manager's chat over a CLI that answers as feedbackCLI does.
func feedbackTester(t *testing.T, inspector bool) (*mainWindow, *ui.Tester, *feedbackCLI) {
	t.Helper()
	m := demoWindow(t)
	m.userWantsInspector = inspector
	m.sidebarCollapsed = true
	f := &feedbackCLI{list: feedbackCLIList, posts: make(chan func(), 64)}
	demo := store
	store = model.NewStore(f, func(fn func()) { f.posts <- fn }, false)
	store.Bots, store.Chats, store.Devices, store.Routines = demo.Bots, demo.Chats, demo.Devices, demo.Routines
	store.Providers, store.Models, store.HasIdentity = demo.Providers, demo.Models, demo.HasIdentity
	store.IsConnected, store.IsStarting = true, false
	store.Subscribe(m.storeChanged)
	m.selectChat("chat-nova")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	tt.Frame()
	return m, tt, f
}

// feedbackDemoMessage is the demo's message in Project Manager's chat that starts with `prefix`.
func feedbackDemoMessage(t *testing.T, prefix string) *model.Message {
	t.Helper()
	for _, message := range store.Chat("chat-nova").Messages {
		if strings.HasPrefix(message.Body.Text, prefix) {
			return message
		}
	}
	t.Fatalf("no message %q", prefix)
	return nil
}

// clickBeside clicks the control to the right of a form label.
func clickBeside(t *testing.T, tt *ui.Tester, label string) {
	t.Helper()
	r, ok := tt.Find(label)
	if !ok {
		t.Fatalf("no %s in %q", label, tt.Texts())
	}
	tt.ClickAt(r.X+160, r.Y+r.H/2+4)
	tt.Frame()
}

func TestFeedbackFormSendsWhatTheUserSaid(t *testing.T) {
	m, tt, f := feedbackTester(t, false)
	reply, yours := feedbackDemoMessage(t, "Writer has the brief."), feedbackDemoMessage(t, "Ask Writer")
	if err := tt.RightClick(model.MessageText(yours)); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if slices.Contains(tt.Menu(), L("Give Feedback…")) {
		t.Fatalf("feedback on the user's own message: %q", tt.Menu())
	}
	tt.CloseMenu()
	if err := tt.RightClick(model.MessageText(reply)); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if err := tt.ChooseMenuItem(L("Give Feedback…")); err != nil {
		t.Fatalf("%v: %q", err, tt.Menu())
	}
	f.wait(t, tt, func() bool { return f.last("feedback.list") != nil && tt.HasText(L("Applies to")) })

	choose(t, tt, L("Looks good"), L("I corrected it"))
	if !tt.HasText(L("Your version")) {
		t.Fatalf("no corrected version: %q", tt.Texts())
	}
	clickBeside(t, tt, L("Your version"))
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("Writer has the brief. Decide on the announcement by noon.")
	clickBeside(t, tt, L("Note"))
	tt.Type("Say what needs my decision.")
	choose(t, tt, L("Any"), "Morning brief")
	settleTransitions(tt)
	renderTo(t, tt, "feedback-form-fixture")
	if err := tt.Click(L("Save")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return !m.hasSheet() })

	sent := f.last("feedback.record")
	feedback, _ := sent["feedback"].(map[string]any)
	origin, _ := feedback["origin"].(map[string]any)
	target, _ := feedback["target"].(map[string]any)
	if sent["bot_id"] != "bot-nova" || feedback["kind"] != "edited" || feedback["note"] != "Say what needs my decision." ||
		feedback["before"] != model.MessageText(reply) || feedback["after"] != "Writer has the brief. Decide on the announcement by noon." ||
		origin["chat_id"] != "chat-nova" || origin["message_id"] != reply.ID || target["id"] != "rt-brief" {
		t.Fatalf("sent %+v", sent)
	}
}

func TestFeedbackSuggestionSendsTheChangeItShowed(t *testing.T) {
	m, tt, f := feedbackTester(t, true)
	f.wait(t, tt, func() bool { return tt.HasText(L("All feedback")) })
	tt.Scroll(1030, 400, 0, 3000)
	tt.Frame()
	if err := tt.Click(L("Suggested change")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return tt.HasText(L("Accept")) && tt.HasText("Open with what needs a decision.") })

	// A target that changed meanwhile is refused, in the app's words, and the sheet stays.
	f.mu.Lock()
	f.err = errors.New("The target changed; review a fresh proposal")
	f.mu.Unlock()
	if err := tt.Click(L("Accept")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool {
		return tt.HasText(L("“%@” changed after this, so the change can't be made.", "Morning brief"))
	})
	if err := tt.Click(L("OK")); err != nil {
		t.Fatal(err)
	}
	f.mu.Lock()
	f.err = nil
	f.mu.Unlock()
	f.wait(t, tt, func() bool { return tt.HasText(L("Accept")) })
	if err := tt.Click(L("Accept")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return !m.hasSheet() })
	sent := f.last("feedback.accept")
	if len(sent) != 3 || sent["bot_id"] != "bot-nova" || sent["id"] != "proposal-1" || sent["diff_hash"] != "shown-diff" {
		t.Fatalf("sent %+v", sent)
	}
}

func TestFeedbackFormIgnoresALateReply(t *testing.T) {
	m, tt, f := feedbackTester(t, false)
	m.presentFeedback("chat-nova", feedbackDemoMessage(t, "Writer has the brief."))
	tt.Frame()
	for m.hasSheet() {
		m.sheets[len(m.sheets)-1].dismiss()
	}
	f.wait(t, tt, func() bool { return f.last("feedback.list") != nil && !m.hasSheet() })
}

// pinClock fixes the formatters' clock, which the demo's times also read, for one test: what a view
// stamps and what the test expects come from the same instant, whatever minute or day it runs in.
func pinClock(t *testing.T, at time.Time) {
	t.Helper()
	model.Now = func() time.Time { return at }
	t.Cleanup(func() { model.Now = time.Now })
}

// The demo, as `LORCA_MOCK=1` shows it: Project Manager's feedback in the inspector and its sheets.
func TestFeedbackInTheDemo(t *testing.T) {
	pinClock(t, time.Date(2026, time.October, 7, 14, 30, 0, 0, time.Local))
	m := demoWindow(t)
	store.Subscribe(m.storeChanged)
	m.sidebarCollapsed = true
	m.selectChat("chat-nova")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settle(tt)
	tt.Scroll(1030, 400, 0, 3000)
	settle(tt)
	if !tt.HasText(L("All feedback")) || !tt.HasText(L("%d notes", 4)+" · "+L("1 change")) {
		t.Fatalf("no feedback section: %q", tt.Texts())
	}
	renderBoth(t, tt, "feedback-inspector")

	if err := tt.Click(L("Suggested change")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	renderBoth(t, tt, "feedback-suggestion")
	if err := tt.Click(L("Reject")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.hasSheet() || len(m.inspector.feedback["bot-nova"].Suggestions) != 0 || tt.HasText(L("Suggested change")) {
		t.Fatal("the rejected suggestion stays")
	}

	// The sheets stand alone over the chat, as the Mac's do.
	m.userWantsInspector = false
	settle(tt)
	m.presentFeedbackList("bot-nova")
	settle(tt)
	renderBoth(t, tt, "feedback-list")
	choose(t, tt, L("Weekly"), L("When asked"))
	settle(tt)
	if f := m.inspector.feedback["bot-nova"]; f.ReviewEvery != nil {
		t.Fatalf("still weekly: %+v", f.ReviewEvery)
	}
	if err := tt.Click(L("Look Now")); err != nil {
		t.Fatal(err)
	}
	time.Sleep(1100 * time.Millisecond)
	settle(tt)
	if !tt.HasText(L("Nothing to change right now.")) {
		t.Fatalf("no answer to Look Now: %q", tt.Texts())
	}

	if err := tt.RightClick("Keep the brief to five bullets or fewer."); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if want := []string{L("Show in Chat"), "-", L("Exclude"), L("Exclude Everything from “%@”", "Project Manager")}; !slices.Equal(tt.Menu(), want) {
		t.Fatalf("menu %q, want %q", tt.Menu(), want)
	}
	if err := tt.ChooseMenuItem(L("Exclude")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if tt.HasText("Keep the brief to five bullets or fewer.") {
		t.Fatal("the excluded note still shows")
	}

	change := m.inspector.feedback["bot-nova"].Changes[0]
	if err := tt.Click(L("Changed %@", model.Stamp(change.CreatedAt))); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	renderBoth(t, tt, "feedback-change")
	if err := tt.Click(L("Undo Change")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	undone := m.inspector.feedback["bot-nova"].Changes[0]
	if !undone.IsUndo || !tt.HasText(L("Undone %@", model.Stamp(undone.CreatedAt))) {
		t.Fatalf("no undo in the list: %q", tt.Texts())
	}

	// A note shows the message it is about.
	note := *m.inspector.feedback["bot-nova"].Note("fb-edit")
	if err := tt.Click(note.Text); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.hasSheet() || m.selection.ChatID != "chat-nova" || m.chat.flashID != note.MessageID {
		t.Fatalf("message not shown: sheet %v, flash %q", m.hasSheet(), m.chat.flashID)
	}

	m.presentFeedback("chat-nova", feedbackDemoMessage(t, "**Today's focus"))
	settle(tt)
	choose(t, tt, L("Looks good"), L("I corrected it"))
	clickBeside(t, tt, L("Note"))
	tt.Type("Decisions go first.")
	renderBoth(t, tt, "feedback-form")
}
