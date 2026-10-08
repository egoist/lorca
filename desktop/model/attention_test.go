package model

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"
)

type attentionTransport struct {
	request func(string, any) (json.RawMessage, error)
}

func (t attentionTransport) Request(method string, params any) (json.RawMessage, error) {
	return t.request(method, params)
}
func (t attentionTransport) Reconnect() {}
func attentionProjectionJSON(title string) json.RawMessage {
	return json.RawMessage(`{"items":[{"id":"item","category":"review","title":"` + title + `","summary":"Decide scope","next_action":"Review the draft","coordinator_bot_id":"bot","sources":[{"chat_id":"chat","task_id":"task-00000076-0000-4000-8000-000000000001","review_id":"review-1"}],"revision":{"counter":18446744073709551615,"device_id":"runner"}}],"briefs":[],"preferences":{"summaries":false,"urgent_direct":true,"default_coordinator_bot_id":"bot"}}`)
}
func TestAttentionWireSnapshotEventsAndIdentity(t *testing.T) {
	s := NewStore(nil, func(fn func()) { fn() }, false)
	s.isBootstrapping = false
	var view AttentionView
	if err := json.Unmarshal(attentionProjectionJSON("Draft"), &view); err != nil {
		t.Fatal(err)
	}
	s.apply(WireSnapshot{HasIdentity: true, Attention: &view})
	if s.Attention.Items[0].Revision.Counter != ^uint64(0) || s.Attention.Items[0].Sources[0].ReviewID != "review-1" || s.Attention.Preferences.Summaries {
		t.Fatalf("projection: %+v", s.Attention)
	}
	s.HandleEvent("attention.changed", attentionProjectionJSON("Edited"))
	if s.Attention.Items[0].Title != "Edited" {
		t.Fatal("event did not replace view")
	}
	s.HandleEvent("identity.changed", json.RawMessage(`{"has_identity":false}`))
	if len(s.Attention.Items) != 0 || !s.Attention.Preferences.Summaries || !s.Attention.Preferences.UrgentDirect {
		t.Fatal("identity reset retains old account attention")
	}
	for _, tag := range []NotificationTag{NotificationSummary, NotificationUrgent, NotificationQuiet} {
		wire := decodeJSON[WireMessage](t, `{"id":"m","author":{"kind":"bot","bot_id":"bot"},"body":{"kind":"text","text":"Update"},"state":{"kind":"complete"},"notification":"`+string(tag)+`"}`)
		if ToMessage(wire).Notification != tag {
			t.Fatal("message lost notification distinction")
		}
	}
}
func TestAttentionActionsUseCLIRevisionsAndOrderedMainReplies(t *testing.T) {
	posted := make(chan func(), 4)
	calls := make(chan string, 4)
	s := NewStore(attentionTransport{func(method string, params any) (json.RawMessage, error) {
		raw, _ := json.Marshal(params)
		calls <- method + " " + string(raw)
		return json.RawMessage(`{}`), errors.New("Attention item changed; refresh before resolving")
	}}, func(fn func()) { posted <- fn }, false)
	completed := false
	s.ResolveAttention("item", AttentionRevision{Counter: ^uint64(0), DeviceID: "runner"}, func(err error) {
		completed = true
		if err == nil {
			t.Error("failure hidden")
		}
	})
	select {
	case call := <-calls:
		if !strings.Contains(call, `"expected_revision":{"counter":18446744073709551615,"device_id":"runner"}`) {
			t.Fatal(call)
		}
	case <-time.After(time.Second):
		t.Fatal("no CLI call")
	}
	fn := <-posted
	if completed {
		t.Fatal("worker ran UI callback outside main queue")
	}
	fn()
	if !completed {
		t.Fatal("queued callback missing")
	}
	// One preference at a time, so a change on another Device to another one stays.
	s.SetAttentionPreference("default_coordinator_bot_id", nil, nil)
	if call := <-calls; call != `attention.preferences {"default_coordinator_bot_id":null}` {
		t.Fatal(call)
	}
	(<-posted)()
}
