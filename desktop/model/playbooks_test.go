package model

import (
	"encoding/json"
	"errors"
	"testing"
	"time"
)

type playbookTransport struct {
	method string
	params map[string]any
	reply  json.RawMessage
	err    error
}

func (t *playbookTransport) Reconnect() {}
func (t *playbookTransport) Request(method string, params any) (json.RawMessage, error) {
	data, _ := json.Marshal(params)
	t.method, t.params = method, nil
	_ = json.Unmarshal(data, &t.params)
	return t.reply, t.err
}

func waitPosts(posts chan func()) {
	select {
	case fn := <-posts:
		fn()
	case <-time.After(2 * time.Second):
	}
}

// An edit carries the revision and hash it was opened at and says it is an edit, never the
// draft's source chat, which may be gone; a refusal for a newer version reads as one.
func TestSavePlaybookGuards(t *testing.T) {
	posts := make(chan func(), 4)
	transport := &playbookTransport{reply: json.RawMessage(`{"id":"playbook-one","scope":{"kind":"bot","id":"bot-one"},"status":"saved","revision":3,"hash":"h3"}`)}
	s := NewStore(transport, func(fn func()) { posts <- fn }, false)
	content := PlaybookContent{Name: "weekly-report", Description: "For Fridays", Instructions: "Compare"}
	over := &PlaybookRecord{ID: "playbook-one", Scope: BotScope("bot-one"), Revision: 2, Hash: "h2"}
	var saved PlaybookRecord
	s.SavePlaybook(over.Scope, content, over, func(record PlaybookRecord, err error) { saved = record })
	waitPosts(posts)
	if transport.method != "playbooks.save" || transport.params["id"] != "playbook-one" || transport.params["expected_revision"] != 2.0 || transport.params["expected_hash"] != "h2" {
		t.Fatalf("save params %v", transport.params)
	}
	if provenance := transport.params["provenance"].(map[string]any); len(provenance) != 1 || provenance["kind"] != "edit" {
		t.Errorf("provenance %v", provenance)
	}
	if references := transport.params["content"].(map[string]any)["references"]; references == nil {
		t.Error("no references array")
	}
	if saved.Revision != 3 {
		t.Errorf("saved %+v", saved)
	}

	s.SavePlaybook(BotScope("bot-one"), content, nil, func(PlaybookRecord, error) {})
	waitPosts(posts)
	if _, ok := transport.params["id"]; ok || transport.params["expected_revision"] != 0.0 || transport.params["expected_hash"] != "" {
		t.Errorf("new skill params %v", transport.params)
	}

	transport.err = errors.New("Playbook changed since you opened it; reload before saving")
	var got error
	s.SavePlaybook(over.Scope, content, over, func(_ PlaybookRecord, err error) { got = err })
	waitPosts(posts)
	if !errors.Is(got, ErrPlaybookChanged) {
		t.Errorf("error %v", got)
	}
}

// The roster lists every skill without its body; a bot's list puts its drafts first, then the
// latest changed.
func TestSkillsFromTheRoster(t *testing.T) {
	s := NewStore(&playbookTransport{}, func(fn func()) { fn() }, false)
	s.handle("roster.changed", json.RawMessage(`{"devices":[],"bots":[],"chats":[],"playbooks":[
		{"id":"a","scope":{"kind":"bot","id":"bot-one"},"name":"old","description":"","status":"saved","revision":1,"hash":"","updated_at":10},
		{"id":"b","scope":{"kind":"bot","id":"bot-one"},"name":"new","description":"","status":"saved","revision":1,"hash":"","updated_at":20},
		{"id":"c","scope":{"kind":"bot","id":"bot-one"},"name":"draft","description":"","status":"draft","revision":1,"hash":"","updated_at":5},
		{"id":"d","scope":{"kind":"project","id":"room"},"name":"group","description":"","status":"saved","revision":1,"hash":"","updated_at":30}]}`))
	var names []string
	for _, skill := range s.Skills(BotScope("bot-one")) {
		names = append(names, skill.Name)
	}
	if len(names) != 3 || names[0] != "draft" || names[1] != "new" || names[2] != "old" {
		t.Errorf("skills %v", names)
	}
	s.handle("roster.changed", json.RawMessage(`{"devices":[],"bots":[],"chats":[]}`))
	if len(s.Playbooks) != 4 {
		t.Error("a roster without skills dropped them")
	}
}

// Save as Skill offers the user's and the bot's messages up to the one clicked, with the request
// before it picked; Save as Standing Instruction offers the user's alone.
func TestCaptureSources(t *testing.T) {
	at := time.Now()
	message := func(id string, author Author) *Message {
		at = at.Add(time.Second)
		return &Message{ID: id, Author: author, Body: Body{Kind: BodyText, Text: id}, CreatedAt: at}
	}
	you, chef, writer := Author{Kind: AuthorYou}, Author{Kind: AuthorBot, BotID: "chef"}, Author{Kind: AuthorBot, BotID: "writer"}
	chat := &Chat{ID: "room", Kind: ChatGroup, BotIDs: []string{"chef", "writer"}, Messages: []*Message{
		message("ask", you), message("other", writer), message("reply", chef), message("fix", you), message("again", you)}}
	botID, kind, sources, picked := CaptureSources(chat, chat.Messages[2])
	if botID != "chef" || kind != "workflow" || len(sources) != 2 || !picked["ask"] || !picked["reply"] {
		t.Errorf("workflow %s %s %d %v", botID, kind, len(sources), picked)
	}
	_, kind, sources, picked = CaptureSources(chat, chat.Messages[4])
	if kind != "corrections" || len(sources) != 3 || len(picked) != 1 {
		t.Errorf("corrections %s %d %v", kind, len(sources), picked)
	}
}
