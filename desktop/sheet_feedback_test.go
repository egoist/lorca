package main

import (
	"encoding/json"
	"errors"
	"slices"
	"sync"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The real views and model run over synthetic CLI responses. No account, model or relay is used.
type feedbackFixture struct {
	mu    sync.Mutex
	calls []feedbackFixtureCall
	data  model.WorkflowFeedback
	err   error
	hold  chan struct{}
	posts chan func()
}
type feedbackFixtureCall struct {
	method string
	params map[string]any
}

func (*feedbackFixture) Reconnect() {}
func (f *feedbackFixture) Request(method string, params any) (json.RawMessage, error) {
	raw, _ := json.Marshal(params)
	var p map[string]any
	_ = json.Unmarshal(raw, &p)
	f.mu.Lock()
	f.calls = append(f.calls, feedbackFixtureCall{method, p})
	hold := f.hold
	err := f.err
	f.mu.Unlock()
	if method != "feedback.list" && hold != nil {
		<-hold
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	if method == "chats.mark_read" || method == "ui.watching" {
		return json.RawMessage(`{}`), nil
	}
	if method == "feedback.list" {
		return json.Marshal(f.data)
	}
	if err != nil {
		return nil, err
	}
	switch method {
	case "feedback.accept":
		f.data.Proposals = nil
		f.data.Revisions = []model.FeedbackRevision{{ID: "revision-1", Version: 1, State: "applied", CanRollback: true, CurrentHash: "live-revision-hash", RollbackDiff: "--- current\n+++ previous\n-Start with the summary.\n+Summarize the inbox."}}
	case "feedback.reject":
		f.data.Proposals = nil
	case "feedback.rollback":
		f.data.Revisions[0].CanRollback = false
		f.data.Revisions = append(f.data.Revisions, model.FeedbackRevision{ID: "revision-2", Version: 2, State: "applied"})
	case "feedback.exclude":
		if chat, ok := p["chat_id"].(string); ok {
			f.data.Settings.ExcludedChats = append(f.data.Settings.ExcludedChats, chat)
			f.data.Proposals = nil
			f.data.Feedback[0].Excluded = true
			f.data.Feedback[0].Note = ""
			f.data.Feedback[0].Example = ""
		}
		if _, ok := p["target"]; ok {
			f.data.Proposals = nil
		}
		if id, ok := p["id"].(string); ok {
			for i := range f.data.Feedback {
				if f.data.Feedback[i].ID == id {
					f.data.Feedback[i].Excluded = true
					f.data.Feedback[i].Note = ""
					f.data.Feedback[i].Example = ""
				}
			}
			f.data.Proposals = nil
		}
	case "feedback.settings":
		if seconds, ok := p["review_every_secs"].(float64); ok {
			value := int64(seconds)
			f.data.Settings.ReviewEverySecs = &value
		} else {
			f.data.Settings.ReviewEverySecs = nil
		}
	case "feedback.review", "feedback.record":
	default:
		return nil, errors.New("Unexpected fixture method " + method)
	}
	return json.RawMessage(`{}`), nil
}
func (f *feedbackFixture) drain() {
	for {
		select {
		case fn := <-f.posts:
			fn()
		default:
			return
		}
	}
}
func (f *feedbackFixture) wait(t *testing.T, tt *ui.Tester, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for time.Now().Before(deadline) {
		f.drain()
		tt.Frame()
		if condition() && !tt.HasText(L("Loading…")) && !tt.HasText(L("Working…")) {
			f.drain()
			tt.Frame()
			return
		}
		time.Sleep(10 * time.Millisecond)
	}
	t.Fatalf("fixture timed out: %q", tt.Texts())
}
func (f *feedbackFixture) last(method string) (map[string]any, bool) {
	f.mu.Lock()
	defer f.mu.Unlock()
	for i := len(f.calls) - 1; i >= 0; i-- {
		if f.calls[i].method == method {
			return f.calls[i].params, true
		}
	}
	return nil, false
}
func feedbackTester(t *testing.T) (*mainWindow, *ui.Tester, *feedbackFixture, *model.Message) {
	t.Helper()
	m := demoWindow(t)
	m.userWantsInspector = false
	m.sidebarCollapsed = true
	origin := model.FeedbackOrigin{ChatID: "chat-nova", MessageID: "feedback-origin"}
	f := &feedbackFixture{posts: make(chan func(), 64), data: model.WorkflowFeedback{
		Feedback:  []model.FeedbackExample{{ID: "feedback-edit", Kind: "edited", Origin: origin, Note: "Start with the summary.", Example: "Three messages need a reply today."}, {ID: "feedback-neutral", Kind: "ignored_alert", Origin: model.FeedbackOrigin{ChatID: "chat-relay", MessageID: "neutral-origin"}, Note: "Another included chat: no response is not a preference."}},
		Proposals: []model.FeedbackProposal{{ID: "proposal-1", State: "pending", Target: model.FeedbackTarget{Kind: "routine_prompt", ID: "rt-brief"}, Explanation: "The selected edit puts the summary first. Keep that order in future briefs.", Origins: []model.FeedbackOrigin{origin}, Evidence: []string{"feedback-edit"}, Diff: "--- current\n+++ proposed\n-Summarize the inbox.\n+Start with the summary, then list action items.", DiffHash: "displayed-diff-hash"}},
		Targets:   []model.FeedbackTargetChoice{{Name: "Morning brief", Target: model.FeedbackTarget{Kind: "routine_prompt", ID: "rt-brief"}}},
	}}
	base := store
	store = model.NewStore(f, func(fn func()) { f.posts <- fn }, false)
	store.Bots, store.Chats, store.Devices, store.Routines = base.Bots, base.Chats, base.Devices, base.Routines
	store.Providers, store.Models = base.Providers, base.Models
	store.HasIdentity = base.HasIdentity
	store.IsConnected = true
	store.IsStarting = false
	message := &model.Message{ID: "feedback-origin", Author: model.BotAuthor("bot-nova"), Body: model.Body{Kind: model.BodyText, Text: "Action items: reply to three messages. Summary: the inbox is otherwise clear."}, State: model.MessageState{Kind: model.StateComplete}, CreatedAt: time.Unix(1800000000, 0)}
	store.Chat("chat-nova").Messages = append(store.Chat("chat-nova").Messages, message)
	store.Chat("chat-relay").Messages = append(store.Chat("chat-relay").Messages, &model.Message{ID: "neutral-origin", Author: model.BotAuthor("bot-nova"), Body: model.Body{Kind: model.BodyText, Text: "Nothing needs a decision today."}, State: model.MessageState{Kind: model.StateComplete}, CreatedAt: time.Unix(1800000000, 0)})
	m.selectChat("chat-nova")
	tt := ui.NewTester(m.frame(m.view), 1180, 980)
	f.drain()
	tt.Frame()
	return m, tt, f, message
}
func feedbackChoose(t *testing.T, tt *ui.Tester, label, item string) {
	t.Helper()
	r, ok := tt.Find(label)
	if !ok {
		t.Fatalf("no field %s", label)
	}
	if label == L("Periodic review") {
		tt.ClickAt(r.X+r.W+30, r.Y+r.H/2)
	} else {
		tt.ClickAt(r.X+170, r.Y+r.H/2)
	}
	tt.Frame()
	if err := tt.ChooseMenuItem(item); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
}
func feedbackScrollTo(t *testing.T, tt *ui.Tester, label string) {
	t.Helper()
	for range 18 {
		if r, ok := tt.Find(label); ok && r.Y > 130 && r.Y+r.H < 870 {
			return
		}
		tt.Scroll(905, 690, 0, 220)
		tt.Frame()
	}
	t.Fatalf("cannot reach %s: %q", label, tt.Texts())
}
func TestFeedbackCaptureKeepsEditsAndSendsOrigin(t *testing.T) {
	m, tt, f, message := feedbackTester(t)
	// The message context menu exposes capture, preserving Reply and text-selection actions.
	if err := tt.RightClick(model.MessageText(message)); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !slices.Contains(tt.Menu(), L("Record workflow feedback…")) {
		t.Fatal(tt.Menu())
	}
	if err := tt.ChooseMenuItem(L("Record workflow feedback…")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return tt.HasText(L("Record workflow feedback")) && !tt.HasText(L("Loading…")) })
	feedbackChoose(t, tt, L("Feedback kind"), L("User edited"))
	feedbackChoose(t, tt, L("Workflow"), "Morning brief")
	if err := tt.Click(L("Your feedback or explanation")); err != nil {
		t.Fatal(err)
	}
	tt.Type("Put the summary first.")
	tt.Frame()
	if err := tt.Click(L("Corrected draft")); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("Summary first. Action items second.")
	tt.Frame()
	// A kind switch must preserve persistent correction/note fields and stable control keys.
	feedbackChoose(t, tt, L("Feedback kind"), L("Explicit feedback"))
	feedbackChoose(t, tt, L("Feedback kind"), L("User edited"))
	renderTo(t, tt, "desktop-feedback-capture")
	if err := tt.Click(L("Record")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return !m.hasSheet() })
	params, ok := f.last("feedback.record")
	if !ok {
		t.Fatal("no capture request")
	}
	payload := params["feedback"].(map[string]any)
	if payload["kind"] != "edited" || payload["after"] != "Summary first. Action items second." || payload["note"] != "Put the summary first." {
		t.Fatal(payload)
	}
	if origin := payload["origin"].(map[string]any); origin["chat_id"] != "chat-nova" || origin["message_id"] != message.ID {
		t.Fatal(origin)
	}
	if payload["event_id"] == "" || payload["target"].(map[string]any)["id"] != "rt-brief" {
		t.Fatal(payload)
	}
}
func TestFeedbackReviewGuardRollbackAndSource(t *testing.T) {
	m, tt, f, _ := feedbackTester(t)
	m.presentWorkflowFeedback("bot-nova", "chat-nova")
	f.wait(t, tt, func() bool { return tt.HasText(L("Accept revision")) })
	feedbackScrollTo(t, tt, L("Accept revision"))
	renderTo(t, tt, "desktop-feedback-proposal")
	if err := tt.Click(L("Accept revision")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return tt.HasText(L("Roll back this revision")) })
	p, _ := f.last("feedback.accept")
	if p["diff_hash"] != "displayed-diff-hash" || p["id"] != "proposal-1" || len(p) != 3 {
		t.Fatal(p)
	}
	feedbackScrollTo(t, tt, L("Roll back this revision"))
	renderTo(t, tt, "desktop-feedback-rollback")
	if err := tt.Click(L("Roll back this revision")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return tt.HasText(L("Version %d · %@", 2, "applied")) })
	p, _ = f.last("feedback.rollback")
	if p["expected_hash"] != "live-revision-hash" || len(p) != 3 {
		t.Fatal(p)
	}
	// Opening work navigates inside Lorca rather than opening untrusted external schemes.
	tt.Scroll(905, 350, 0, -1800)
	tt.Frame()
	feedbackScrollTo(t, tt, L("Open originating work"))
	if err := tt.Click(L("Open originating work")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return !m.hasSheet() })
	if m.selection.ChatID != "chat-relay" || m.chat.flashID != "neutral-origin" {
		t.Fatal("source not revealed")
	}
}
func TestFeedbackExclusionsQuietReviewAndSettings(t *testing.T) {
	m, tt, f, _ := feedbackTester(t)
	m.presentWorkflowFeedback("bot-nova", "chat-nova")
	f.wait(t, tt, func() bool { return tt.HasText(L("Accept revision")) })
	feedbackChoose(t, tt, L("Periodic review"), L("Weekly"))
	f.wait(t, tt, func() bool {
		p, ok := f.last("feedback.settings")
		return ok && p["review_every_secs"] == float64(604800) && !tt.HasText(L("Working…"))
	})
	feedbackChoose(t, tt, L("Periodic review"), L("Off"))
	f.wait(t, tt, func() bool {
		p, ok := f.last("feedback.settings")
		return ok && p["review_every_secs"] == nil && !tt.HasText(L("Working…"))
	})
	if err := tt.Click(L("Review now")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { _, ok := f.last("feedback.review"); return ok && !tt.HasText(L("Working…")) })
	if err := tt.Click(L("Exclude this chat")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return tt.HasText(L("This chat is excluded")) })
	if tt.HasText("Start with the summary.") || tt.HasText(L("Accept revision")) {
		t.Fatal("excluded evidence/proposal remained visible")
	}
	if !tt.HasText(L("Ignored alert · neutral")) || !tt.HasText(L("No improvements waiting for review.")) {
		t.Fatal(tt.Texts())
	}
	renderTo(t, tt, "desktop-feedback-exclusions")
}
func TestFeedbackConflictDoesNotApplyOrResetGuard(t *testing.T) {
	m, tt, f, _ := feedbackTester(t)
	m.presentWorkflowFeedback("bot-nova", "chat-nova")
	f.wait(t, tt, func() bool { return tt.HasText(L("Accept revision")) })
	f.mu.Lock()
	f.err = errors.New("The target changed since you opened it. Reload it.")
	f.mu.Unlock()
	feedbackScrollTo(t, tt, L("Accept revision"))
	if err := tt.Click(L("Accept revision")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return tt.HasText("The target changed since you opened it. Reload it.") })
	if !m.hasSheet() || !tt.HasText(L("Accept revision")) {
		t.Fatal("conflict discarded the review")
	}
	p, _ := f.last("feedback.accept")
	if p["diff_hash"] != "displayed-diff-hash" {
		t.Fatal("guard changed on conflict")
	}
	tt.Scroll(905, 350, 0, -1800)
	tt.Frame()
	renderTo(t, tt, "desktop-feedback-conflict")
}

func TestFeedbackCaptureFreezesFieldsAndKeepsRetryEvent(t *testing.T) {
	m, tt, f, message := feedbackTester(t)
	m.presentFeedbackCapture("chat-nova", message)
	f.wait(t, tt, func() bool { return tt.HasText(L("Record workflow feedback")) && !tt.HasText(L("Loading…")) })
	feedbackChoose(t, tt, L("Feedback kind"), L("Explicit feedback"))
	if err := tt.Click(L("Your feedback or explanation")); err != nil {
		t.Fatal(err)
	}
	tt.Type("Keep this preference.")
	tt.Frame()
	hold := make(chan struct{})
	f.mu.Lock()
	f.hold = hold
	f.err = errors.New("Temporary failure. Retry the same decision.")
	f.mu.Unlock()
	if err := tt.Click(L("Record")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { _, ok := f.last("feedback.record"); return ok })
	first, _ := f.last("feedback.record")
	original := first["feedback"].(map[string]any)
	if err := tt.Click(L("Your feedback or explanation")); err != nil {
		t.Fatal(err)
	}
	tt.Type("Should not alter the saving draft.")
	tt.Frame()
	close(hold)
	f.wait(t, tt, func() bool { return tt.HasText("Temporary failure. Retry the same decision.") })
	f.mu.Lock()
	f.hold = nil
	f.err = nil
	f.mu.Unlock()
	if err := tt.Click(L("Record")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return !m.hasSheet() })
	retry, _ := f.last("feedback.record")
	saved := retry["feedback"].(map[string]any)
	if saved["event_id"] != original["event_id"] || saved["note"] != "Keep this preference." {
		t.Fatal("saving fields or retry id changed", saved)
	}
}
func TestFeedbackDismissalIgnoresLateLoad(t *testing.T) {
	m, tt, f, _ := feedbackTester(t)
	m.presentWorkflowFeedback("bot-nova", "chat-nova")
	tt.Frame()
	tt.Key(0, ui.KeyEscape)
	tt.Frame()
	if m.hasSheet() {
		t.Fatal("Escape failed")
	}
	f.wait(t, tt, func() bool { _, ok := f.last("feedback.list"); return ok })
	f.drain()
	tt.Frame()
	if m.hasSheet() || tt.HasText(L("Accept revision")) {
		t.Fatal("late reply revived a closed sheet")
	}
}
func TestFeedbackWorkflowAndExampleExclusionsAreScoped(t *testing.T) {
	for _, which := range []string{"workflow", "example"} {
		t.Run(which, func(t *testing.T) {
			m, tt, f, _ := feedbackTester(t)
			m.presentWorkflowFeedback("bot-nova", "chat-nova")
			f.wait(t, tt, func() bool { return tt.HasText(L("Accept revision")) })
			label := L("Exclude this workflow")
			if which == "example" {
				label = L("Exclude")
			}
			feedbackScrollTo(t, tt, label)
			if err := tt.Click(label); err != nil {
				t.Fatal(err)
			}
			f.wait(t, tt, func() bool { _, ok := f.last("feedback.exclude"); return ok && !tt.HasText(L("Working…")) })
			p, _ := f.last("feedback.exclude")
			if which == "workflow" {
				if p["target"].(map[string]any)["id"] != "rt-brief" {
					t.Fatal(p)
				}
			} else {
				if p["id"] != "feedback-neutral" {
					t.Fatal(p)
				}
			}
		})
	}
}

func TestFeedbackEditedRetryGetsNewImmutableEvent(t *testing.T) {
	m, tt, f, message := feedbackTester(t)
	m.presentFeedbackCapture("chat-nova", message)
	f.wait(t, tt, func() bool { return tt.HasText(L("Record workflow feedback")) && !tt.HasText(L("Loading…")) })
	if err := tt.Click(L("Your feedback or explanation")); err != nil {
		t.Fatal(err)
	}
	tt.Type("First opinion.")
	tt.Frame()
	f.mu.Lock()
	f.err = errors.New("Receipt unavailable.")
	f.mu.Unlock()
	if err := tt.Click(L("Record")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return tt.HasText("Receipt unavailable.") })
	first, _ := f.last("feedback.record")
	id := first["feedback"].(map[string]any)["event_id"]
	if err := tt.Click(L("Your feedback or explanation")); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("Corrected opinion.")
	tt.Frame()
	f.mu.Lock()
	f.err = nil
	f.mu.Unlock()
	if err := tt.Click(L("Record")); err != nil {
		t.Fatal(err)
	}
	f.wait(t, tt, func() bool { return !m.hasSheet() })
	retry, _ := f.last("feedback.record")
	payload := retry["feedback"].(map[string]any)
	if payload["event_id"] == id || payload["note"] != "Corrected opinion." {
		t.Fatal("edited retry reused an immutable event", payload)
	}
}
