package model

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"
)

const exactReview = `{"id":"review-exact","runner_id":"runner-one","bot_id":"bot-one","origin":{"chat_id":"chat-one"},"target":{"account":"Sandbox","resource":"Draft"},"rationale":"Review before sending","payload":{"kind":"plugin","plugin_id":"mail-one","server_name":"main","tool":"send","future_payload_field":18446744073709551615,"arguments":{"account_id":9223372036854775807,"unsigned_id":18446744073709551615,"small_value":0.000000000000000123,"draft_body":{"subject_line":"Before"}}},"version":18446744073709551615,"revision":9,"preconditions":{"workdir":"/sandbox","files":[]},"state":"pending"}`

func TestReviewJSONKeepsKeysNumbersAndNewPayloadFields(t *testing.T) {
	item := decodeJSON[ReviewItem](t, exactReview)
	if item.Version != ^uint64(0) {
		t.Fatal("version lost precision")
	}
	edited, err := item.Payload.EditedJSON(strings.Replace(item.Payload.EditorText(), "Before", "After", 1))
	if err != nil {
		t.Fatal(err)
	}
	for _, token := range []string{`"account_id":9223372036854775807`, `"unsigned_id":18446744073709551615`, `"small_value":0.000000000000000123`, `"subject_line":"After"`, `"future_payload_field":18446744073709551615`} {
		if !strings.Contains(string(edited), token) {
			t.Errorf("lost %s in %s", token, edited)
		}
	}
	for _, invalid := range []string{`[]`, `null`, `"text"`, `{bad`} {
		if _, err := item.Payload.EditedJSON(invalid); err == nil {
			t.Errorf("accepted %s", invalid)
		}
	}
	shell := decodeJSON[ReviewPayload](t, `{"kind":"shell","arguments":{"command":"git push","description":"Push","timeout_ms":120000}}`)
	if shell.EditorText() != "git push" {
		t.Fatalf("shell editor text: %q", shell.EditorText())
	}
	command, err := shell.EditedJSON("git push --tags")
	if err != nil || !strings.Contains(string(command), `"command":"git push --tags"`) || !strings.Contains(string(command), `"description":"Push"`) || !strings.Contains(string(command), `"timeout_ms":120000`) {
		t.Fatalf("shell edit: %s %v", command, err)
	}
	draft := decodeJSON[ReviewPayload](t, `{"kind":"draft","text":"original"}`)
	text, err := draft.EditedJSON("new\ntext")
	if err != nil || !strings.Contains(string(text), `"text":"new\ntext"`) {
		t.Fatalf("draft edit: %s %v", text, err)
	}
}

func TestReviewSnapshotAndEventsKeepNewestProjection(t *testing.T) {
	s := NewStore(nil, func(fn func()) { fn() }, false)
	s.apply(decodeJSON[WireSnapshot](t, `{"has_identity":true,"reviews":[`+exactReview+`]}`))
	s.isBootstrapping = false
	if s.Review("review-exact").Version != ^uint64(0) {
		t.Fatal("snapshot rounded its version")
	}
	var events []Event
	s.Subscribe(func(e Event) { events = append(events, e) })
	next := s.Review("review-exact").Clone()
	next.Revision, next.State = 10, "uncertain"
	data, _ := json.Marshal(map[string]any{"item": next, "change": "interrupted"})
	s.HandleEvent("reviews.changed", data)
	old := json.RawMessage(`{"item":` + exactReview + `}`)
	s.HandleEvent("reviews.changed", old)
	if s.Review(next.ID).State != "uncertain" || len(events) != 1 || events[0].Kind != EventReviewsChanged {
		t.Fatalf("regressed projection or event: %+v", events)
	}
	if len(s.OpenReviewsFor("chat-one")) != 0 {
		t.Fatal("a decided item is still open")
	}
	cloned := s.Review(next.ID).Clone()
	cloned.Payload.Arguments[0] = '!'
	if !json.Valid(s.Review(next.ID).Payload.Arguments) {
		t.Fatal("view mutated the stored payload")
	}
	s.apply(decodeJSON[WireSnapshot](t, `{"has_identity":false}`))
	if len(s.Reviews) != 0 {
		t.Fatal("identity snapshot retained reviews")
	}
}

type reviewRequest struct {
	method string
	params json.RawMessage
}
type reviewAnswer struct {
	data json.RawMessage
	err  error
}
type reviewTransport struct {
	requests chan reviewRequest
	answers  chan reviewAnswer
}

func (r *reviewTransport) Reconnect() {}
func (r *reviewTransport) Request(method string, params any) (json.RawMessage, error) {
	data, err := json.Marshal(params)
	if err != nil {
		return nil, err
	}
	r.requests <- reviewRequest{method, data}
	a := <-r.answers
	return a.data, a.err
}

func waitReviewPost(t *testing.T, posts <-chan func()) func() {
	t.Helper()
	select {
	case fn := <-posts:
		return fn
	case <-time.After(3 * time.Second):
		t.Fatal("no main-thread reply")
		return nil
	}
}

func TestApprovingAnEditSavesItThenApprovesThatVersion(t *testing.T) {
	transport := &reviewTransport{make(chan reviewRequest, 1), make(chan reviewAnswer, 1)}
	posts := make(chan func(), 1)
	s := NewStore(transport, func(fn func()) { posts <- fn }, false)
	item := decodeJSON[ReviewItem](t, exactReview)
	item.Version = 6
	s.Reviews = []*ReviewItem{&item}
	payload, _ := item.Payload.EditedJSON(strings.Replace(item.Payload.EditorText(), "Before", "Edited", 1))
	var approved ReviewItem
	s.ApproveReview(item.Clone(), payload, func(result ReviewItem, err error) {
		if err != nil {
			t.Error(err)
		}
		approved = result
	})
	req := <-transport.requests
	if req.method != "reviews.edit" || !strings.Contains(string(req.params), `"expected_version":6`) || !strings.Contains(string(req.params), `"account_id":9223372036854775807`) || !strings.Contains(string(req.params), `"subject_line":"Edited"`) {
		t.Fatalf("wrong edit: %s %s", req.method, req.params)
	}
	if strings.Contains(string(req.params), `"runner_id"`) || strings.Contains(string(req.params), `"target"`) {
		t.Fatal("the edit carried more than the payload")
	}
	edited := item.Clone()
	edited.Version, edited.Revision = 7, 10
	encoded, _ := json.Marshal(edited)
	transport.answers <- reviewAnswer{data: encoded}
	waitReviewPost(t, posts)()
	req = <-transport.requests
	if req.method != "reviews.approve" || !strings.Contains(string(req.params), `"expected_version":7`) {
		t.Fatalf("approved another version: %s %s", req.method, req.params)
	}
	done := edited.Clone()
	done.State, done.Revision = "approved", 11
	encoded, _ = json.Marshal(done)
	transport.answers <- reviewAnswer{data: encoded}
	waitReviewPost(t, posts)()
	if approved.State != "approved" || s.Review(item.ID).Revision != 11 {
		t.Fatalf("approval not applied: %+v", approved)
	}
}

func TestReviewRepliesComeInOrderAndNeverRegress(t *testing.T) {
	transport := &reviewTransport{make(chan reviewRequest, 1), make(chan reviewAnswer, 1)}
	posts := make(chan func(), 1)
	s := NewStore(transport, func(fn func()) { posts <- fn }, false)
	item := decodeJSON[ReviewItem](t, exactReview)
	s.Reviews = []*ReviewItem{&item}
	called := false
	s.RejectReview(item.Clone(), func(_ ReviewItem, err error) { called = err == nil })
	req := <-transport.requests
	var fields map[string]json.RawMessage
	if err := json.Unmarshal(req.params, &fields); err != nil {
		t.Fatal(err)
	}
	if req.method != "reviews.reject" || string(fields["expected_version"]) != "18446744073709551615" || len(fields) != 2 {
		t.Fatalf("unexpected decision parameters: %s %s", req.method, req.params)
	}
	reply := item.Clone()
	reply.Revision = 11
	encoded, _ := json.Marshal(reply)
	transport.answers <- reviewAnswer{data: encoded}
	fn := waitReviewPost(t, posts)
	if called || s.Review(item.ID).Revision != 9 {
		t.Fatal("reply touched state before its main-thread post")
	}
	newer := item.Clone()
	newer.Revision, newer.State = 12, "rejected"
	s.upsertReview(newer)
	fn()
	if !called || s.Review(item.ID).Revision != 12 {
		t.Fatal("late response overwrote newer state")
	}
}

func TestReviewRepliesCannotLeakIntoAnotherIdentity(t *testing.T) {
	transport := &reviewTransport{make(chan reviewRequest, 1), make(chan reviewAnswer, 1)}
	posts := make(chan func(), 1)
	s := NewStore(transport, func(fn func()) { posts <- fn }, false)
	s.IdentityID = "first"
	item := decodeJSON[ReviewItem](t, exactReview)
	called := false
	s.RejectReview(item, func(_ ReviewItem, err error) {
		called = true
		if err == nil {
			t.Error("accepted a previous identity's reply")
		}
	})
	<-transport.requests
	s.IdentityID = "second"
	transport.answers <- reviewAnswer{data: json.RawMessage(exactReview)}
	waitReviewPost(t, posts)()
	if !called || len(s.Reviews) != 0 {
		t.Fatal("reply populated the new account")
	}
}

func TestOfflineRunnerLeavesTheItemAsItWas(t *testing.T) {
	transport := &reviewTransport{make(chan reviewRequest, 1), make(chan reviewAnswer, 1)}
	posts := make(chan func(), 1)
	s := NewStore(transport, func(fn func()) { posts <- fn }, false)
	item := decodeJSON[ReviewItem](t, exactReview)
	s.Reviews = []*ReviewItem{&item}
	var failed error
	s.ApproveReview(item.Clone(), nil, func(_ ReviewItem, err error) { failed = err })
	<-transport.requests
	transport.answers <- reviewAnswer{err: errors.New("Workbench is offline.")}
	waitReviewPost(t, posts)()
	if failed == nil || s.Review(item.ID).Revision != 9 || s.Review(item.ID).State != "pending" {
		t.Fatal("an error changed the saved item")
	}
}

func TestReviewMockResetDoesNotClearALiveProjection(t *testing.T) {
	s := NewStore(nil, func(fn func()) { fn() }, false)
	item := decodeJSON[ReviewItem](t, exactReview)
	s.Reviews = []*ReviewItem{&item}
	s.ResetMockData()
	if s.Review(item.ID) == nil {
		t.Fatal("mock-only reset removed a live review")
	}
}
