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

func TestOutputWirePreservesImmutableReferencesAndEvidence(t *testing.T) {
	m := ToMessage(decodeJSON[WireMessage](t, outputMessageFixture))
	if m.Output == nil || m.Output.ID != "out-result" || m.Output.Version != 2 || m.Output.PreviousMessageID != "msg-output-v1" {
		t.Fatalf("output version lost: %+v", m.Output)
	}
	if m.Output.BotID != m.Author.BotID || m.Output.ChatID != "chat-1" || m.Output.TaskID == "" {
		t.Fatalf("origin lost: %+v", m.Output)
	}
	if m.Output.Evidence.ExitCode == nil || *m.Output.Evidence.ExitCode != 0 || m.Output.Evidence.Title() != "Test result" || m.Output.Evidence.StatusText() != "Passed" {
		t.Fatalf("evidence lost: %+v", m.Output.Evidence)
	}
	if len(m.Attachments) != 1 || m.Attachments[0].ID != "att-output-v2" {
		t.Fatal("attachment identity lost")
	}
	legacy := ToMessage(decodeJSON[WireMessage](t, `{"id":"old","body":{"kind":"text","text":"hello"},"state":{"kind":"complete"}}`))
	if legacy.Output != nil {
		t.Fatal("legacy message became an output")
	}
}

func TestOutputListRepliesStayOnMainThreadAndKeepHistory(t *testing.T) {
	posts := make(chan func(), 4)
	called := make(chan map[string]any, 1)
	transport := outputTransport{func(method string, params any) (json.RawMessage, error) {
		if method != "outputs.list" {
			return nil, errors.New("unexpected method")
		}
		called <- params.(map[string]any)
		return json.RawMessage(`{"outputs":[` + outputMessageFixture + `],"has_more":true}`), nil
	}}
	s := NewStore(transport, func(run func()) { posts <- run }, false)
	var page OutputPage
	finished := false
	s.LoadOutputs("chat-1", "task-scope", func(result OutputPage, err error) {
		if err != nil {
			t.Error(err)
		}
		page = result
		finished = true
	})
	params := <-called
	if !reflect.DeepEqual(params, map[string]any{"chat_id": "chat-1", "task_id": "task-scope"}) {
		t.Fatalf("wrong scope: %v", params)
	}
	if finished {
		t.Fatal("callback ran on worker")
	}
	nextOutputPost(t, posts)
	if !finished || !page.HasMore || len(page.Messages) != 1 || page.Messages[0].Output.Version != 2 {
		t.Fatalf("wrong output page: %+v", page)
	}
}

func TestOutputAttachmentFailureRetryAndNamedRetrieval(t *testing.T) {
	posts := make(chan func(), 4)
	var calls atomic.Int32
	transport := outputTransport{func(method string, params any) (json.RawMessage, error) {
		if method != "files.path" || params.(map[string]any)["named"] != true {
			return nil, errors.New("not a named CLI retrieval")
		}
		if calls.Add(1) == 1 {
			return nil, errors.New("relay no longer has Tests.txt")
		}
		return json.RawMessage(`{"path":"/local/Tests.txt"}`), nil
	}}
	s := NewStore(transport, func(run func()) { posts <- run }, false)
	a := Attachment{ID: "att-test", Name: "Tests.txt", Mime: "text/plain", Size: 12}
	if s.LocalFile(a, "chat", "msg") != "" {
		t.Fatal("unfetched file had a path")
	}
	nextOutputPost(t, posts)
	if s.AttachmentError(a.ID) == "" {
		t.Fatal("unavailability not retained")
	}
	for range 3 {
		s.LocalFile(a, "chat", "msg")
	}
	if calls.Load() != 1 {
		t.Fatal("rendering repeated a failed transfer")
	}
	s.RetryAttachment(a, "chat", "msg")
	nextOutputPost(t, posts)
	if s.AttachmentError(a.ID) != "" || s.LocalFile(a, "chat", "msg") != "/local/Tests.txt" || calls.Load() != 2 {
		t.Fatal("Retry did not retrieve/clear the error")
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
