package model

import (
	"encoding/json"
	"errors"
	"slices"
	"sync"
	"testing"
	"time"
)

type historyReply struct {
	data json.RawMessage
	err  error
}
type historyRequest struct {
	params map[string]any
	reply  chan historyReply
}
type historyTransport struct {
	requests chan historyRequest
	mu       sync.Mutex
	count    int
}

func (h *historyTransport) Reconnect() {}
func (h *historyTransport) Request(method string, params any) (json.RawMessage, error) {
	if method != "chats.messages" {
		return json.RawMessage(`{}`), nil
	}
	h.mu.Lock()
	h.count++
	h.mu.Unlock()
	request := historyRequest{params: params.(map[string]any), reply: make(chan historyReply, 1)}
	h.requests <- request
	reply := <-request.reply
	return reply.data, reply.err
}
func (h *historyTransport) calls() int { h.mu.Lock(); defer h.mu.Unlock(); return h.count }
func historyFixture() (*Store, *historyTransport, chan func()) {
	h := &historyTransport{requests: make(chan historyRequest, 8)}
	posts := make(chan func(), 16)
	s := NewStore(h, func(fn func()) { posts <- fn }, false)
	s.Chats = []*Chat{{ID: "target", Messages: []*Message{{ID: "newest", Body: Body{Kind: BodyText, Text: "Newest"}}}, HasMore: true}}
	return s, h, posts
}
func takeHistoryRequest(t *testing.T, h *historyTransport) historyRequest {
	t.Helper()
	select {
	case request := <-h.requests:
		return request
	case <-time.After(3 * time.Second):
		t.Fatal("no history request")
		return historyRequest{}
	}
}
func runHistoryPost(t *testing.T, posts chan func()) {
	t.Helper()
	select {
	case fn := <-posts:
		fn()
	case <-time.After(3 * time.Second):
		t.Fatal("no posted reply")
	}
}
func historyPage(ids []string, more bool) json.RawMessage {
	messages := make([]map[string]any, 0, len(ids))
	for _, id := range ids {
		messages = append(messages, map[string]any{"id": id, "chat_id": "target", "author": map[string]string{"kind": "bot", "bot_id": "specialist"}, "body": map[string]string{"kind": "text", "text": id}, "state": map[string]string{"kind": "complete"}, "created_at": 1})
	}
	data, _ := json.Marshal(map[string]any{"messages": messages, "has_more": more})
	return data
}

func TestHistoryPagingSharesRequestAndPostsAppliedStateBeforeCallbacks(t *testing.T) {
	s, h, posts := historyFixture()
	var order []string
	s.Subscribe(func(event Event) {
		if event.Kind == EventOlderMessagesLoaded {
			order = append(order, "event")
		}
	})
	s.LoadOlderMessages("target")
	request := takeHistoryRequest(t, h)
	for _, name := range []string{"first", "second"} {
		s.LoadOlderMessagesThen("target", func(err error) {
			if err != nil || s.IsLoadingOlder("target") || len(s.Chat("target").Messages) != 3 {
				t.Fatalf("callback before history applied: %v", err)
			}
			order = append(order, name)
		})
	}
	if request.params["chat_id"] != "target" || request.params["before"] != "newest" {
		t.Fatalf("params %+v", request.params)
	}
	request.reply <- historyReply{data: historyPage([]string{"oldest", "older", "older", "newest"}, false)}
	if len(order) != 0 {
		t.Fatal("reply touched store before main-thread post")
	}
	runHistoryPost(t, posts)
	if !slices.Equal(order, []string{"event", "first", "second"}) || h.calls() != 1 {
		t.Fatalf("order %v; calls %d", order, h.calls())
	}
	if s.Chat("target").HasMore {
		t.Fatal("history did not end")
	}
}

func TestHistoryFailurePausesAutomaticPagingButExplicitRetryWorks(t *testing.T) {
	for _, failure := range []bool{true, false} {
		s, h, posts := historyFixture()
		var result error
		s.LoadOlderMessagesThen("target", func(err error) { result = err })
		request := takeHistoryRequest(t, h)
		if failure {
			request.reply <- historyReply{err: errors.New("offline fixture")}
		} else {
			request.reply <- historyReply{data: historyPage([]string{"newest"}, true)}
		}
		runHistoryPost(t, posts)
		if result == nil || s.IsLoadingOlder("target") {
			t.Fatal("failure not delivered")
		}
		for range 4 {
			s.LoadOlderMessages("target")
		}
		if h.calls() != 1 {
			t.Fatal("automatic paging retried the failed boundary")
		}
		s.LoadOlderMessagesThen("target", func(err error) { result = err })
		retry := takeHistoryRequest(t, h)
		retry.reply <- historyReply{data: historyPage([]string{"older"}, false)}
		runHistoryPost(t, posts)
		if result != nil || len(s.Chat("target").Messages) != 2 {
			t.Fatalf("retry failed: %v", result)
		}
	}
}

func TestHistoryRepliesCannotOverwriteChangedTranscriptOrAccount(t *testing.T) {
	for _, scope := range []string{"boundary", "bootstrap", "identity", "forgotten"} {
		s, h, posts := historyFixture()
		yes := true
		s.HasIdentity = &yes
		var result error
		s.LoadOlderMessagesThen("target", func(err error) { result = err })
		request := takeHistoryRequest(t, h)
		if scope == "bootstrap" {
			s.bootstrapGeneration++
		} else if scope == "identity" {
			s.IdentityID = "different-account"
		} else if scope == "forgotten" {
			no := false
			s.HasIdentity = &no
		} else {
			s.Chat("target").Messages = []*Message{{ID: "replacement"}}
		}
		request.reply <- historyReply{data: historyPage([]string{"old-account-row"}, false)}
		runHistoryPost(t, posts)
		if result == nil || len(s.Chat("target").Messages) != 1 || !s.Chat("target").HasMore {
			t.Fatal("stale history reply overwrote current transcript")
		}
		if s.olderFailed["target"] != "" {
			t.Fatal("stale failure paused new-scope history")
		}
	}
}
