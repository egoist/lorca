package model

import (
	"encoding/json"
	"errors"
	"testing"
	"time"
)

func TestWorkflowWireAndTheAccountInUse(t *testing.T) {
	p := decodeJSON[WorkflowProgress](t, `{"setup":{"id":"w1","runner_id":"r1","updated_at":1,"pack":{"id":"inbox","name":"Inbox","symbol_name":"envelope","questions":[],"connections":[]},"answers":{"inbox-scope":"Unread today"},"bot_ids":{"triager":"b1"},"connection_ids":{"gmail":"gmail-personal"},"phase":"reviewed","sample":{"job_id":"j1","chat_id":"c1","bot_id":"b1","started_at":1,"state":"reviewed","message_ids":["m1"]}},
		"connections":[
			{"service_id":"gmail","name":"Gmail","selected_id":"gmail-personal","available":true,"choices":[{"id":"gmail-work","name":"Gmail","service_id":"gmail","account_name":"Work","state":"ready","detail":"Ready"},{"id":"gmail-personal","name":"Gmail","service_id":"gmail","account_name":"Personal","state":"needs_auth","detail":"Sign in"}]},
			{"service_id":"github","name":"GitHub","selected_id":null,"available":true,"choices":[{"id":"github","name":"GitHub","state":"ready","detail":"Ready"}]},
			{"service_id":"slack","name":"Slack","selected_id":null,"available":false,"choices":[{"id":"slack-a","name":"Slack","account_name":"A","state":"ready"},{"id":"slack-b","name":"Slack","account_name":"B","state":"ready"}]}],
		"specialists":[],"routines":[{"id":"routine","name":"Check inbox","schedule_text":"Weekdays at 9:00 AM","is_enabled":false}],
		"sample_messages":[{"id":"m1","chat_id":"c1","author":{"kind":"bot","bot_id":"b1"},"body":{"kind":"text","text":"One urgent item."},"state":{"kind":"complete"},"created_at":1}],"is_running":false}`)
	expect(t, p.Setup.Pack.SymbolName, "envelope")
	expect(t, p.Setup.Sample.JobID, "j1")
	expect(t, p.SampleMessages[0].Body.Text, "One urgent item.")
	expect(t, p.Routines[0].IsEnabled, false)
	// The chosen account with its own state, the Runner's only one, and none of several unchosen.
	expect(t, p.Connections[0].Account().AccountName, "Personal")
	expect(t, p.Connections[0].Account().State, PluginNeedsAuth)
	expect(t, p.Connections[1].Account().ID, "github")
	expect(t, p.Connections[2].Account() == nil, true)
	// An older CLI's marketplace has no packs.
	old := ToMarketplace(decodeJSON[WireMarketplace](t, `{"plugins":[],"bots":[]}`))
	expect(t, len(old.Packs), 0)
}

type workflowTestCall struct {
	method string
	params json.RawMessage
}
type workflowTestReply struct {
	body json.RawMessage
	err  error
}
type workflowTestTransport struct {
	calls   chan workflowTestCall
	replies chan workflowTestReply
}

func (t *workflowTestTransport) Request(method string, params any) (json.RawMessage, error) {
	bytes, _ := json.Marshal(params)
	t.calls <- workflowTestCall{method: method, params: bytes}
	reply := <-t.replies
	return reply.body, reply.err
}
func (t *workflowTestTransport) Reconnect() {}

func TestWorkflowParametersSnapshotAndCallbacksUsePost(t *testing.T) {
	transport := &workflowTestTransport{calls: make(chan workflowTestCall, 1), replies: make(chan workflowTestReply, 1)}
	posted := make(chan func(), 4)
	s := NewStore(transport, func(fn func()) { posted <- fn }, false)
	answers := map[string]string{"repositories": "original/repository"}
	callback := false
	s.Workflow("configure", map[string]any{"id": "w1", "answers": answers, "bot_ids": map[string]string{"monitor": "chosen-bot"}}, func(p WorkflowProgress, err error) { callback = true; expect(t, err, nil); expect(t, p.Setup.ID, "w1") })
	answers["repositories"] = "later/edit"
	var call workflowTestCall
	select {
	case call = <-transport.calls:
	case <-time.After(time.Second):
		t.Fatal("no request")
	}
	expect(t, call.method, "workflows.configure")
	var params struct {
		Answers map[string]string `json:"answers"`
		BotIDs  map[string]string `json:"bot_ids"`
	}
	if err := json.Unmarshal(call.params, &params); err != nil {
		t.Fatal(err)
	}
	expect(t, params.Answers["repositories"], "original/repository")
	expect(t, params.BotIDs["monitor"], "chosen-bot")
	transport.replies <- workflowTestReply{body: json.RawMessage(`{"setup":{"id":"w1"}}`)}
	var reply func()
	select {
	case reply = <-posted:
	case <-time.After(time.Second):
		t.Fatal("no posted reply")
	}
	if callback {
		t.Fatal("callback ran before main-thread post")
	}
	reply()
	if !callback {
		t.Fatal("posted callback not run")
	}
	s.Workflow("connection", map[string]any{"id": "w1", "service_id": "gmail", "plugin_id": "gmail-personal"}, func(_ WorkflowProgress, err error) { expect(t, err.Error(), "Runner offline") })
	call = <-transport.calls
	expect(t, call.method, "workflows.connection")
	var account map[string]string
	_ = json.Unmarshal(call.params, &account)
	expect(t, account["plugin_id"], "gmail-personal")
	transport.replies <- workflowTestReply{err: errors.New("Runner offline")}
	select {
	case fn := <-posted:
		fn()
	case <-time.After(time.Second):
		t.Fatal("error not posted")
	}
}
