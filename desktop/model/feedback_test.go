package model

import (
	"encoding/json"
	"errors"
	"reflect"
	"testing"
	"time"
)

type feedbackCall struct {
	method string
	params json.RawMessage
}
type feedbackTransport struct {
	calls chan feedbackCall
	reply json.RawMessage
	err   error
}

func (t *feedbackTransport) Request(method string, params any) (json.RawMessage, error) {
	encoded, _ := json.Marshal(params)
	t.calls <- feedbackCall{method, encoded}
	return t.reply, t.err
}
func (*feedbackTransport) Reconnect() {}
func feedbackStore() (*Store, *feedbackTransport, chan func()) {
	transport := &feedbackTransport{calls: make(chan feedbackCall, 10), reply: json.RawMessage(`{}`)}
	posts := make(chan func(), 10)
	return NewStore(transport, func(fn func()) { posts <- fn }, false), transport, posts
}
func feedbackPost(t *testing.T, posts chan func()) {
	t.Helper()
	select {
	case fn := <-posts:
		fn()
	case <-time.After(2 * time.Second):
		t.Fatal("reply was not posted")
	}
}
func TestFeedbackDecodeKeepsExactGuardsScopesAndNeutralKinds(t *testing.T) {
	data := decodeJSON[WorkflowFeedback](t, `{"feedback":[{"id":"f1","kind":"ignored_alert","excluded":false,"origin":{"chat_id":"c","message_id":"m","review_id":"review-1","task_id":"task-1"}}],"proposals":[{"id":"p","target":{"kind":"playbook","id":"playbook-1","scope":{"kind":"project","id":"group-1"}},"before":{"content":{"instructions":"Old"},"hash":"canonical","revision":9007199254740993},"after":{"instructions":"New"},"diff":"-Old\n+New","diff_hash":"shown-diff","state":"pending","evidence":["f1"]}],"revisions":[{"id":"v1","version":9007199254740993,"state":"applied","can_rollback":true,"current_hash":"current","rollback_diff":"-New\n+Old"}],"settings":{"review_every_secs":null,"excluded_chats":["private-chat"]}}`)
	if data.Proposals[0].Before.Revision != 9007199254740993 || data.Revisions[0].Version != 9007199254740993 {
		t.Fatal("revision precision lost")
	}
	expect(t, *data.Proposals[0].Target.Scope, FeedbackScope{"project", "group-1"})
	expect(t, data.Proposals[0].DiffHash, "shown-diff")
	expect(t, data.Feedback[0].Kind, "ignored_alert")
	expect(t, data.Feedback[0].Origin.TaskID, "task-1")
	if data.Settings.ReviewEverySecs != nil {
		t.Fatal("silent review enablement")
	}
}
func TestFeedbackRequestsFreezeInputAndPostReplies(t *testing.T) {
	s, transport, posts := feedbackStore()
	before, after := "original", "edited"
	input := FeedbackInput{Kind: "edited", Origin: FeedbackOrigin{ChatID: "c", MessageID: "m"}, Before: &before, After: &after, EventID: "stable-event"}
	called := false
	s.RecordFeedback("b", input, func(err error) {
		if err != nil {
			t.Error(err)
		}
		called = true
	})
	before = "changed while waiting"
	after = "later edit"
	call := <-transport.calls
	if called {
		t.Fatal("callback escaped the main-thread post")
	}
	expect(t, call.method, "feedback.record")
	var params struct {
		BotID    string        `json:"bot_id"`
		Feedback FeedbackInput `json:"feedback"`
	}
	if err := json.Unmarshal(call.params, &params); err != nil {
		t.Fatal(err)
	}
	expect(t, *params.Feedback.Before, "original")
	expect(t, *params.Feedback.After, "edited")
	expect(t, params.Feedback.EventID, "stable-event")
	feedbackPost(t, posts)
	if !called {
		t.Fatal("no ordered reply")
	}
}
func TestFeedbackDecisionsUseDisplayedGuardsOnly(t *testing.T) {
	s, transport, posts := feedbackStore()
	proposal := FeedbackProposal{ID: "proposal", State: "pending", Diff: "-old\n+new", DiffHash: "displayed-hash"}
	for _, accept := range []bool{true, false} {
		s.DecideFeedback("bot", proposal, accept, func(err error) {
			if err != nil {
				t.Error(err)
			}
		})
		call := <-transport.calls
		want := "feedback.reject"
		if accept {
			want = "feedback.accept"
		}
		expect(t, call.method, want)
		var p map[string]any
		_ = json.Unmarshal(call.params, &p)
		expect(t, p, map[string]any{"bot_id": "bot", "id": "proposal", "diff_hash": "displayed-hash"})
		feedbackPost(t, posts)
	}
	revision := FeedbackRevision{ID: "revision", State: "applied", CanRollback: true, CurrentHash: "live-hash", RollbackDiff: "-new\n+old"}
	s.RollbackFeedback("bot", revision, func(err error) {
		if err != nil {
			t.Error(err)
		}
	})
	call := <-transport.calls
	expect(t, call.method, "feedback.rollback")
	var p map[string]any
	_ = json.Unmarshal(call.params, &p)
	expect(t, p, map[string]any{"bot_id": "bot", "id": "revision", "expected_hash": "live-hash"})
	feedbackPost(t, posts)
	proposal.DiffHash = ""
	s.DecideFeedback("bot", proposal, true, func(err error) {
		if err == nil {
			t.Fatal("unguarded decision")
		}
	})
	feedbackPost(t, posts)
	revision.CanRollback = false
	s.RollbackFeedback("bot", revision, func(err error) {
		if err == nil {
			t.Fatal("later edits overwritten")
		}
	})
	feedbackPost(t, posts)
	select {
	case call := <-transport.calls:
		t.Fatalf("invalid guard sent %v", call)
	default:
	}
}
func TestFeedbackSettingsAndExclusionsUseCanonicalPayload(t *testing.T) {
	s, transport, posts := feedbackStore()
	done := func(err error) {
		if err != nil {
			t.Error(err)
		}
	}
	interval := int64(604800)
	s.SetFeedbackReview("bot", &interval, done)
	interval = 5
	call := <-transport.calls
	expect(t, call.method, "feedback.settings")
	var p map[string]any
	_ = json.Unmarshal(call.params, &p)
	expect[any](t, p["review_every_secs"], float64(604800))
	feedbackPost(t, posts)
	s.SetFeedbackReview("bot", nil, done)
	call = <-transport.calls
	_ = json.Unmarshal(call.params, &p)
	if value, exists := p["review_every_secs"]; !exists || value != nil {
		t.Fatal("off must send explicit null")
	}
	feedbackPost(t, posts)
	target := FeedbackTarget{Kind: "plugin_skill", PluginID: "existing-account", Name: "Skill"}
	s.ExcludeFeedback("bot", FeedbackExclusion{Target: &target}, done)
	call = <-transport.calls
	_ = json.Unmarshal(call.params, &p)
	expect[any](t, p["target"], map[string]any{"kind": "plugin_skill", "plugin_id": "existing-account", "name": "Skill"})
	feedbackPost(t, posts)
}
func TestFeedbackErrorsAndEventsStayOnTheModelBoundary(t *testing.T) {
	s, transport, posts := feedbackStore()
	transport.err = errors.New("changed since displayed revision")
	got := ""
	s.ReviewFeedback("bot", func(err error) { got = ErrorText(err) })
	<-transport.calls
	if got != "" {
		t.Fatal("worker wrote UI error")
	}
	feedbackPost(t, posts)
	expect(t, got, "changed since displayed revision")
	s.isBootstrapping = false
	var events []Event
	s.Subscribe(func(e Event) { events = append(events, e) })
	s.HandleEvent("feedback.changed", json.RawMessage(`{"bot_id":"bot","pending_count":1}`))
	s.HandleEvent("feedback.changed", json.RawMessage(`{"bot_id":"bot","pending_count":1}`))
	expect(t, s.WorkflowProposalCounts["bot"], 1)
	expect(t, s.WorkflowFeedbackVersions["bot"], uint64(2))
	if len(events) != 2 || events[0].Kind != EventWorkflowFeedbackChanged {
		t.Fatal(events)
	}
}

type feedbackPages struct{ calls []map[string]any }

func (p *feedbackPages) Request(method string, params any) (json.RawMessage, error) {
	encoded, _ := json.Marshal(params)
	var fields map[string]any
	_ = json.Unmarshal(encoded, &fields)
	p.calls = append(p.calls, fields)
	if method != "chats.messages" {
		return nil, errors.New("unexpected source request")
	}
	return json.RawMessage(`{"messages":[{"id":"origin","author":{"kind":"bot","bot_id":"bot"},"body":{"kind":"text","text":"Source work"},"state":{"kind":"complete"},"created_at":1}],"has_more":false}`), nil
}
func (*feedbackPages) Reconnect() {}
func TestFeedbackSourceLoadsOlderPageAndPreservesNewerMessages(t *testing.T) {
	transport := &feedbackPages{}
	posts := make(chan func(), 10)
	s := NewStore(transport, func(fn func()) { posts <- fn }, false)
	s.Chats = []*Chat{{ID: "chat", HasMore: true, Messages: []*Message{{ID: "latest", Body: Body{Kind: BodyText, Text: "Latest"}}}}}
	done := false
	s.LoadFeedbackOrigin(FeedbackOrigin{ChatID: "chat", MessageID: "origin"}, func(err error) {
		if err != nil {
			t.Error(err)
		}
		done = true
	})
	feedbackPost(t, posts)
	if !done || len(s.Chats[0].Messages) != 2 || s.Chats[0].Messages[0].ID != "origin" || s.Chats[0].Messages[1].ID != "latest" {
		t.Fatal("source page lost ordering")
	}
	if !reflect.DeepEqual(transport.calls, []map[string]any{{"chat_id": "chat", "before": "latest"}}) {
		t.Fatal(transport.calls)
	}
}
