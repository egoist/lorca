package model

import (
	"encoding/json"
	"errors"
	"reflect"
	"sync/atomic"
	"testing"
	"time"
)

type outputTransport struct {
	request func(string, any) (json.RawMessage, error)
}

func (t outputTransport) Request(method string, params any) (json.RawMessage, error) {
	return t.request(method, params)
}
func (outputTransport) Reconnect() {}

func nextOutputPost(t *testing.T, posts <-chan func()) {
	t.Helper()
	select {
	case run := <-posts:
		run()
	case <-time.After(3 * time.Second):
		t.Fatal("no main-thread reply")
	}
}

const outputMessageFixture = `{"id":"msg-output-v2","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},"body":{"kind":"text","text":"Result","attachments":[{"id":"att-output-v2","name":"Tests.txt","mime":"text/plain","size":12}]},"state":{"kind":"complete"},"created_at":1728000000,"output":{"id":"out-result","name":"Tests.txt","mime":"text/plain","bot_id":"bot-1","chat_id":"chat-1","task_id":"task-3a249410-8034-4be2-bf42-9e68b4fe2c41","version":2,"previous_message_id":"msg-output-v1","evidence":{"kind":"test_result","summary":"Focused checks pass","status":"passed","command":"cargo test","exit_code":0}}}`

func TestOutputWireKeepsTheVersionAndCheck(t *testing.T) {
	m := ToMessage(decodeJSON[WireMessage](t, outputMessageFixture))
	if m.Output == nil || m.Output.ID != "out-result" || m.Output.Version != 2 || m.Output.BotID != "bot-1" {
		t.Fatalf("output version lost: %+v", m.Output)
	}
	if m.Output.Evidence.Command != "cargo test" || m.Output.Evidence.Title() != "Test result" || m.Output.Evidence.StatusText() != "Passed" {
		t.Fatalf("check lost: %+v", m.Output.Evidence)
	}
	if m.Output.Symbol() != "doc.text" || len(m.Attachments) != 1 || m.Attachments[0].ID != "att-output-v2" {
		t.Fatal("attachment lost")
	}
	legacy := ToMessage(decodeJSON[WireMessage](t, `{"id":"old","body":{"kind":"text","text":"hello"},"state":{"kind":"complete"}}`))
	if legacy.Output != nil {
		t.Fatal("a plain message became an output")
	}
}

func TestOutputsGroupVersionsNewestFirst(t *testing.T) {
	at := func(minutes int) time.Time { return time.Date(2026, 10, 8, 9, minutes, 0, 0, time.UTC) }
	version := func(id string, n uint32, minutes int) *Message {
		return &Message{ID: id + string(rune('0'+n)), CreatedAt: at(minutes), Output: &Output{ID: id, Name: id, Version: n}}
	}
	series := GroupOutputs([]*Message{version("out-report", 1, 1), version("out-chart", 1, 2), version("out-report", 2, 3), {ID: "plain"}})
	if len(series) != 2 || series[0].ID() != "out-report" || series[1].ID() != "out-chart" {
		t.Fatalf("wrong order: %+v", series)
	}
	if series[0].Output().Version != 2 || series[0].Versions[1].Output.Version != 1 {
		t.Fatal("versions not newest first")
	}
}

func TestOutputsListOnceThenFollowMessages(t *testing.T) {
	posts := make(chan func(), 4)
	var calls atomic.Int32
	transport := outputTransport{func(method string, params any) (json.RawMessage, error) {
		if method != "outputs.list" || !reflect.DeepEqual(params, map[string]any{"chat_id": "chat-1"}) {
			return nil, errors.New("unexpected request")
		}
		calls.Add(1)
		return json.RawMessage(`{"outputs":[` + outputMessageFixture + `]}`), nil
	}}
	s := NewStore(transport, func(run func()) { posts <- run }, false)
	s.Chats = []*Chat{{ID: "chat-1"}}
	if len(s.Outputs("chat-1")) != 0 || len(s.Outputs("chat-1")) != 0 {
		t.Fatal("outputs before the CLI answered")
	}
	nextOutputPost(t, posts)
	if got := s.Outputs("chat-1"); len(got) != 1 || got[0].Output().Version != 2 || calls.Load() != 1 {
		t.Fatalf("wrong outputs: %+v (%d calls)", got, calls.Load())
	}
	// A new version arrives as a message, and the list follows it without asking again.
	next := ToMessage(decodeJSON[WireMessage](t, outputMessageFixture))
	next.ID, next.Output = "msg-output-v3", &Output{ID: "out-result", Name: "Tests.txt", Version: 3}
	next.CreatedAt = next.CreatedAt.Add(time.Minute)
	s.upsert(next, "chat-1")
	if got := s.Outputs("chat-1"); len(got) != 1 || got[0].Output().Version != 3 || len(got[0].Versions) != 2 || calls.Load() != 1 {
		t.Fatalf("new version not followed: %+v", got)
	}
	// A resync lists the chat again, and what it had stays meanwhile.
	s.apply(WireSnapshot{Chats: []WireChat{{ID: "chat-1"}}})
	if got := s.Outputs("chat-1"); len(got) != 1 {
		t.Fatal("a resync emptied the outputs")
	}
	nextOutputPost(t, posts)
	if calls.Load() != 2 {
		t.Fatalf("a resync listed %d times", calls.Load())
	}
}

func TestOutputAttachmentFailureRetryAndNamedCopyForOpening(t *testing.T) {
	posts := make(chan func(), 4)
	var calls atomic.Int32
	transport := outputTransport{func(method string, params any) (json.RawMessage, error) {
		if method != "files.path" {
			return nil, errors.New("unexpected method")
		}
		if params.(map[string]any)["named"] == true {
			return json.RawMessage(`{"path":"/local/open/Tests.txt"}`), nil
		}
		if calls.Add(1) == 1 {
			return nil, errors.New("relay no longer has Tests.txt")
		}
		return json.RawMessage(`{"path":"/local/att-test"}`), nil
	}}
	s := NewStore(transport, func(run func()) { posts <- run }, false)
	a := Attachment{ID: "att-test", Name: "Tests.txt", Mime: "text/plain", Size: 12}
	if s.LocalFile(a, "chat", "msg") != "" {
		t.Fatal("unfetched file had a path")
	}
	nextOutputPost(t, posts)
	if s.AttachmentError(a.ID) == "" {
		t.Fatal("the failure was not kept")
	}
	for range 3 {
		s.LocalFile(a, "chat", "msg")
	}
	if calls.Load() != 1 {
		t.Fatal("drawing repeated a failed fetch")
	}
	s.RetryAttachment(a, "chat", "msg")
	nextOutputPost(t, posts)
	if s.AttachmentError(a.ID) != "" || s.LocalFile(a, "chat", "msg") != "/local/att-test" || calls.Load() != 2 {
		t.Fatal("Retry did not fetch the file")
	}
	var opened string
	s.OpenableFile(a, func(path string, err error) { opened = path })
	nextOutputPost(t, posts)
	if opened != "/local/open/Tests.txt" {
		t.Fatalf("opening used %q, not the named copy", opened)
	}
}

func TestOutputDocumentReferencesRejectExternalLaunchSchemes(t *testing.T) {
	for _, link := range []string{"file:///etc/passwd", "javascript:alert(1)", "http://example.com", "https://user:secret@example.com"} {
		if got := (Output{URL: link}).DocumentURL(); got != "" {
			t.Fatalf("accepted %q", link)
		}
	}
	if got := (Output{URL: "https://docs.example.com/report"}).DocumentURL(); got == "" {
		t.Fatal("valid document lost")
	}
}

func TestOutputAttachmentReplyCannotCrossIdentityChange(t *testing.T) {
	posts := make(chan func(), 2)
	requested := make(chan struct{})
	release := make(chan struct{})
	s := NewStore(outputTransport{func(method string, params any) (json.RawMessage, error) {
		close(requested)
		<-release
		return json.RawMessage(`{"path":"/old-account/Tests.txt"}`), nil
	}}, func(run func()) { posts <- run }, false)
	s.IdentityID = "old-account"
	attachment := Attachment{ID: "att-scope", Name: "Tests.txt", Mime: "text/plain"}
	s.LocalFile(attachment, "chat", "msg")
	<-requested
	identity := "new-account"
	s.apply(WireSnapshot{IdentityID: &identity})
	close(release)
	nextOutputPost(t, posts)
	if s.attachmentFiles[attachment.ID] != "" {
		t.Fatal("old-account reply entered the new account cache")
	}
}
