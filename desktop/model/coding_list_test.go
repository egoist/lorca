package model

import (
	"encoding/json"
	"sync/atomic"
	"testing"
	"time"
)

// codingListJSON is `coding.list` with two bots' agents in two chats on two Runners: Scout's,
// working, first; Patch's, done with a pull request, after.
const codingListJSON = `{"agents":[
	{"chat_id":"chat-relay","chat_title":"Relay","message_id":"msg-2","bot_id":"bot-scout","bot_name":"Scout",
	 "agent":{"id":"agent-2","kind":"codex","title":"Fix #249","task":"Fix #249","folder":"~/dev/lorca","branch":"fix-249","state":"working",
	          "device":"Workbench","device_id":"dev-workbench","repository":"github.com/egoist/lorca","started_at":1700000100,"updated_at":1700000200}},
	{"chat_id":"chat-patch","chat_title":"Patch","message_id":"msg-1","bot_id":"bot-patch","bot_name":"Patch",
	 "agent":{"id":"agent-1","kind":"claude","title":"Bots on any Runner","task":"Do #249","folder":"~/dev/lorca","state":"idle",
	          "pull_request":"https://github.com/egoist/lorca/pull/252","device":"Studio","device_id":"dev-studio","repository":"github.com/egoist/lorca","updated_at":1700000000}}
]}`

// codingTransport answers coding.list, after `release` lets it when one is set, and counts the calls.
type codingTransport struct {
	calls   *atomic.Int32
	release chan struct{}
}

func (t codingTransport) Request(method string, _ any) (json.RawMessage, error) {
	if method != "coding.list" {
		return json.RawMessage(`{}`), nil
	}
	t.calls.Add(1)
	if t.release != nil {
		<-t.release
	}
	return json.RawMessage(codingListJSON), nil
}
func (codingTransport) Reconnect() {}

func TestAccountAgentsFromCodingList(t *testing.T) {
	posts := make(chan func(), 8)
	calls := &atomic.Int32{}
	s := NewStore(codingTransport{calls: calls}, func(fn func()) { posts <- fn }, false)
	s.isBootstrapping = false
	changed := 0
	s.Subscribe(func(event Event) {
		if event.Kind == EventCodingAgentsChanged {
			changed++
		}
	})
	s.refreshAccountAgents()
	(<-posts)()
	list := s.AccountAgents()
	if len(list) != 2 || changed != 1 {
		t.Fatalf("%d agents, %d events", len(list), changed)
	}
	// The CLI's order stays: open first, as it sent them.
	first, second := list[0], list[1]
	if first.Agent.ID != "agent-2" || first.BotName != "Scout" || first.ChatTitle != "Relay" || first.ChatID != "chat-relay" || first.MessageID != "msg-2" || first.BotID != "bot-scout" {
		t.Fatalf("first: %+v", first)
	}
	if first.Agent.DeviceID != "dev-workbench" || first.Agent.Device != "Workbench" || first.Agent.Repository != "github.com/egoist/lorca" || first.Agent.State != AgentWorking {
		t.Fatalf("first agent: %+v", first.Agent)
	}
	if second.Agent.ID != "agent-1" || second.BotName != "Patch" || second.Agent.PullNumber() != "#252" || second.Agent.State != AgentIdle {
		t.Fatalf("second: %+v", second.Agent)
	}
	// The working one counts; the done one changed long ago, so it does not.
	if live := s.LiveAgents(time.Unix(1700000000, 0).Add(10 * 24 * time.Hour)); live != 1 {
		t.Fatalf("%d live", live)
	}
	if live := s.LiveAgents(time.Unix(1700000300, 0)); live != 2 {
		t.Fatalf("%d live just after", live)
	}
	// Only the working one is at work; the done one waits for a follow-up.
	if working := s.WorkingAgents(); working != 1 {
		t.Fatalf("%d working", working)
	}
}

// A start row's card that changes asks again; while a request is on its way, changes wait for it
// and ask once more after. A row with no agent asks nothing.
func TestAccountAgentsRefetchCoalesces(t *testing.T) {
	posts := make(chan func(), 8)
	calls := &atomic.Int32{}
	release := make(chan struct{})
	s := NewStore(codingTransport{calls: calls, release: release}, func(fn func()) { posts <- fn }, false)
	s.isBootstrapping = false
	agentRow := json.RawMessage(`{"chat_id":"chat-patch","message":{"id":"msg-1","chat_id":"chat-patch","author":{"kind":"bot","bot_id":"bot-patch"},
		"body":{"kind":"tool","name":"coding_agent","summary":"Started Claude Code","agent":{"id":"agent-1","kind":"claude","task":"Do it","state":"working"}},
		"state":{"kind":"complete"},"created_at":1700000000}}`)
	textRow := json.RawMessage(`{"chat_id":"chat-patch","message":{"id":"msg-9","chat_id":"chat-patch","author":{"kind":"bot","bot_id":"bot-patch"},
		"body":{"kind":"text","text":"On it."},"state":{"kind":"complete"},"created_at":1700000000}}`)

	s.HandleEvent("message.added", textRow)
	time.Sleep(20 * time.Millisecond)
	if calls.Load() != 0 {
		t.Fatal("a text row asked for the list")
	}
	s.HandleEvent("message.updated", agentRow)
	s.HandleEvent("message.updated", agentRow)
	s.HandleEvent("message.removed", json.RawMessage(`{"chat_id":"chat-patch","message_id":"msg-1"}`))
	waitFor(t, func() bool { return calls.Load() == 1 })
	time.Sleep(20 * time.Millisecond)
	if calls.Load() != 1 {
		t.Fatalf("%d requests in flight", calls.Load())
	}
	release <- struct{}{}
	(<-posts)()
	// Once more for what changed meanwhile, and no more.
	waitFor(t, func() bool { return calls.Load() == 2 })
	release <- struct{}{}
	(<-posts)()
	time.Sleep(20 * time.Millisecond)
	if calls.Load() != 2 || len(s.AccountAgents()) != 2 {
		t.Fatalf("%d requests, %d agents", calls.Load(), len(s.AccountAgents()))
	}
	// A chat that goes asks again.
	s.HandleEvent("chat.removed", json.RawMessage(`{"chat_id":"chat-patch"}`))
	waitFor(t, func() bool { return calls.Load() == 3 })
	release <- struct{}{}
	(<-posts)()
}

// The demo lists its chats' agents as the CLI would: open first, then the latest.
func TestMockAccountAgents(t *testing.T) {
	s := NewStore(nil, func(fn func()) { fn() }, true)
	s.Start()
	list := s.AccountAgents()
	if len(list) < 3 {
		t.Fatalf("%d agents", len(list))
	}
	for i, listed := range list {
		if listed.BotName == "" || listed.ChatTitle == "" || listed.MessageID == "" {
			t.Fatalf("agent %d: %+v", i, listed)
		}
		if i > 0 && listed.Agent.IsOpen() && !list[i-1].Agent.IsOpen() {
			t.Fatal("an open agent after an ended one")
		}
	}
}

func waitFor(t *testing.T, done func() bool) {
	t.Helper()
	for range 200 {
		if done() {
			return
		}
		time.Sleep(5 * time.Millisecond)
	}
	t.Fatal("timed out")
}
