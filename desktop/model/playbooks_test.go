package model

import (
	"encoding/json"
	"errors"
	"slices"
	"testing"
	"time"
)

type playbookTransport struct {
	requests chan struct {
		method string
		params json.RawMessage
	}
	response json.RawMessage
	err      error
}

func (t *playbookTransport) Reconnect() {}
func (t *playbookTransport) Request(method string, params any) (json.RawMessage, error) {
	data, _ := json.Marshal(params)
	t.requests <- struct {
		method string
		params json.RawMessage
	}{method, data}
	return t.response, t.err
}

func TestPlaybookWireAndGuardedReviewRequests(t *testing.T) {
	posts := make(chan func(), 10)
	transport := &playbookTransport{requests: make(chan struct {
		method string
		params json.RawMessage
	}, 10), response: json.RawMessage(`{
        "id":"playbook-one","scope":{"kind":"bot","id":"bot-one"},"name":"weekly-review","description":"Review a public report",
        "path":"playbook://playbook-one/SKILL.md","status":"draft","revision":2,"hash":"hash-two",
        "content":{"name":"weekly-review","description":"Review a public report","instructions":"Compare evidence","examples":"A public example","references":[{"path":"references/a.md","text":"First"}],"scripts":[]},
        "provenance":{"kind":"corrections","chat_id":"group-one","message_ids":["one","two"],"note":"Repeated correction"},
        "revisions":[{"revision":1,"status":"draft","content":{"name":"weekly-review","description":"Review","instructions":"Old wording"},"provenance":{"kind":"workflow","message_ids":[],"note":"Prior workflow"},"created_at":100}]
    }`)}
	s := NewStore(transport, func(fn func()) { posts <- fn }, false)
	scope := PlaybookScope{"bot", "bot-one"}
	var record PlaybookRecord
	called := false
	s.Playbook(scope, "playbook-one", func(result PlaybookRecord, err error) {
		if err != nil {
			t.Fatal(err)
		}
		record, called = result, true
	})
	request := <-transport.requests
	if request.method != "playbooks.get" {
		t.Fatal(request.method)
	}
	var params map[string]json.RawMessage
	_ = json.Unmarshal(request.params, &params)
	if string(params["scope"]) != `{"kind":"bot","id":"bot-one"}` {
		t.Fatal(string(params["scope"]))
	}
	fn := <-posts
	if called {
		t.Fatal("callback bypassed the main-thread post queue")
	}
	fn()
	if record.Content == nil || record.Revision != 2 || record.Hash != "hash-two" || record.Revisions[0].Content.Instructions != "Old wording" {
		t.Fatalf("wire: %+v", record)
	}
	if record.Provenance.ChatID == nil || *record.Provenance.ChatID != "group-one" {
		t.Fatal(record.Provenance)
	}
	content := record.Content.Clone()
	content.Instructions, content.References[0].Text = "Corrected wording", "Changed reference"
	if record.Content.References[0].Text != "First" {
		t.Fatal("editing changed the opened revision's content")
	}
	s.SavePlaybook(scope, content, &record, func(_ PlaybookRecord, err error) {
		if err != nil {
			t.Fatal(err)
		}
	})
	request = <-transport.requests
	_ = json.Unmarshal(request.params, &params)
	if request.method != "playbooks.save" || string(params["expected_revision"]) != "2" || string(params["expected_hash"]) != `"hash-two"` || string(params["id"]) != `"playbook-one"` {
		t.Fatal(string(request.params))
	}
	var provenance PlaybookProvenance
	_ = json.Unmarshal(params["provenance"], &provenance)
	if provenance.Kind != "reviewed_edit" || !slices.Equal(provenance.MessageIDs, []string{"one", "two"}) {
		t.Fatal(provenance)
	}
	(<-posts)()

	s.SavePlaybook(PlaybookScope{"project", "group-two"}, content, nil, nil)
	request = <-transport.requests
	params = map[string]json.RawMessage{}
	_ = json.Unmarshal(request.params, &params)
	if _, hasID := params["id"]; hasID || string(params["expected_revision"]) != "0" || string(params["expected_hash"]) != `""` {
		t.Fatal(string(request.params))
	}
	(<-posts)()

	transport.err = errors.New("Playbook changed since you opened it; reload before saving")
	var conflict error
	s.SavePlaybook(scope, content, &record, func(_ PlaybookRecord, err error) { conflict = err })
	<-transport.requests
	(<-posts)()
	if conflict == nil || len(s.AutoReview.Rules) != 0 {
		t.Fatal("conflict lost or permissions changed")
	}
}

func TestPlaybookCaptureSourcesAndExplicitScope(t *testing.T) {
	start := time.Unix(100, 0)
	message := func(id string, author Author, text string, n int) *Message {
		return &Message{ID: id, Author: author, Body: Body{Kind: BodyText, Text: text}, CreatedAt: start.Add(time.Duration(n) * time.Second)}
	}
	one := message("one", You, "Include a public example", 0)
	reply := message("reply", BotAuthor("bot-one"), "Completed the public workflow", 1)
	two := message("two", You, "Again, include a public example", 2)
	other := message("other", BotAuthor("bot-other"), "Another bot's work", 3)
	streaming := message("streaming", BotAuthor("bot-one"), "Still running", 4)
	streaming.State.Kind = StateStreaming
	chat := &Chat{ID: "group-one", Kind: ChatGroup, BotIDs: []string{"bot-one"}, Messages: []*Message{one, reply, two, other, streaming}}
	s := NewStore(nil, func(fn func()) { fn() }, false)
	s.Bots = []*Bot{{ID: "bot-one"}, {ID: "bot-other"}}
	s.Chats = []*Chat{chat, {ID: "other-group", Kind: ChatGroup, BotIDs: []string{"bot-other"}}}
	scopes := s.PlaybookScopes(chat.ID)
	if len(scopes) != 2 || scopes[0] != (PlaybookScope{"project", chat.ID}) || scopes[1] != (PlaybookScope{"bot", "bot-one"}) {
		t.Fatal(scopes)
	}
	bot, kind, sources, picked := CaptureSources(chat, reply)
	if bot != "bot-one" || kind != "workflow" || len(sources) != 3 || !picked[one.ID] || !picked[reply.ID] || picked[two.ID] {
		t.Fatalf("%s %s %+v %+v", bot, kind, sources, picked)
	}
	bot, kind, sources, picked = CaptureSources(chat, two)
	if bot != "bot-one" || kind != "corrections" || len(sources) != 2 || !picked[two.ID] || picked[one.ID] {
		t.Fatal(kind, sources, picked)
	}
	if bot, _, _, _ := CaptureSources(chat, other); bot != "" {
		t.Fatal("nonmember capture was offered")
	}
	if bot, _, _, _ := CaptureSources(chat, streaming); bot != "" {
		t.Fatal("streaming capture was offered")
	}
	chat.Kind = ChatDM
	if scopes := s.PlaybookScopes(chat.ID); len(scopes) != 1 || scopes[0].Kind != "bot" {
		t.Fatal("DM inherited a project", scopes)
	}
}

func TestPlaybookDraftRemoveAndPortableExportUseOnlyTheirContracts(t *testing.T) {
	posts := make(chan func(), 10)
	transport := &playbookTransport{requests: make(chan struct {
		method string
		params json.RawMessage
	}, 10), response: json.RawMessage(`{"status":"draft","revision":1,"hash":"draft-hash"}`)}
	s := NewStore(transport, func(fn func()) { posts <- fn }, false)
	scope := PlaybookScope{"project", "group-one"}
	s.DraftPlaybook(scope, "bot-one", "group-one", "corrections", []string{"one", "two"}, nil)
	request := <-transport.requests
	var params map[string]json.RawMessage
	_ = json.Unmarshal(request.params, &params)
	if request.method != "playbooks.draft" || string(params["message_ids"]) != `["one","two"]` || string(params["kind"]) != `"corrections"` {
		t.Fatal(request.method, string(request.params))
	}
	(<-posts)()
	item := PlaybookSummary{ID: "playbook-one", Scope: scope, Revision: 4, Hash: "hash-four", Status: "saved"}
	s.RemovePlaybook(item, nil)
	request = <-transport.requests
	_ = json.Unmarshal(request.params, &params)
	if request.method != "playbooks.remove" || string(params["expected_hash"]) != `"hash-four"` || string(params["expected_revision"]) != "4" {
		t.Fatal(request.method, string(request.params))
	}
	(<-posts)()
	transport.response = json.RawMessage(`{"format":"lorca-playbook","version":1,"content":{"name":"public-example"},"files":[]}`)
	var portable json.RawMessage
	s.ExportPlaybook(item, func(value json.RawMessage, err error) {
		if err != nil {
			t.Fatal(err)
		}
		portable = value
	})
	request = <-transport.requests
	params = map[string]json.RawMessage{}
	_ = json.Unmarshal(request.params, &params)
	(<-posts)()
	if request.method != "playbooks.export" || len(params) != 2 || string(portable) != string(transport.response) {
		t.Fatal(request.method, string(request.params), string(portable))
	}
}
