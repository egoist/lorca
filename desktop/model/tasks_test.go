package model

import (
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"testing"
)

type taskTransport struct {
	request func(string, any) (json.RawMessage, error)
}

func (t taskTransport) Request(method string, params any) (json.RawMessage, error) {
	return t.request(method, params)
}
func (t taskTransport) Reconnect() {}

func taskFixture() DurableTask {
	return DurableTask{ID: "task-00000000-0000-0000-0000-000000000001", Revision: 7, AuthorityRunnerID: "authority", OwnerBotID: "owner", RunnerID: "runner", Goal: "Verify delivery", AcceptanceCriteria: []string{"No duplicate run"}, NextAction: "Review proof", ChatIDs: []string{"group", "helper-dm"}, State: TaskAwaitingReview}
}

func TestTaskWireSnapshotAndEvidence(t *testing.T) {
	var snapshot WireSnapshot
	if err := json.Unmarshal([]byte(`{"has_identity":true,"tasks":[{"id":"task-id","revision":8,"authority_runner_id":"a","owner_bot_id":"bot","runner_id":"b","goal":"Verified","acceptance_criteria":["Passed"],"dependencies":[],"next_action":"Review","chat_ids":["group"],"state":"awaiting_review","links":[],"evidence":[{"kind":"output","label":"Result v2","chat_id":"group","message_id":"m2","output_id":"out-id","version":2}],"active_run":null,"reason":null,"result":null}]}`), &snapshot); err != nil {
		t.Fatal(err)
	}
	s := NewStore(nil, func(fn func()) { fn() }, false)
	s.apply(snapshot)
	task := s.DurableTask("task-id")
	if task == nil || task.Revision != 8 || task.RunnerID != "b" || task.AuthorityRunnerID != "a" || task.State != TaskAwaitingReview || len(task.Evidence) != 1 {
		t.Fatalf("wire task: %+v", task)
	}
	e := task.Evidence[0]
	if str(e.OutputID) != "out-id" || str(e.MessageID) != "m2" || e.Version == nil || *e.Version != 2 {
		t.Fatalf("evidence: %+v", e)
	}
	if !slices.Equal(task.ChatIDs, []string{"group"}) {
		t.Fatal("chat scope changed")
	}
}

func TestTaskEventsAndRepliesDoNotLoseNewerOwnership(t *testing.T) {
	s := NewStore(nil, func(fn func()) { fn() }, false)
	first := taskFixture()
	s.AcceptDurableTask(first)
	newer := first.Clone()
	newer.Revision = 9
	newer.OwnerBotID = "new-owner"
	newer.RunnerID = "new-runner"
	raw, _ := json.Marshal(struct {
		Task DurableTask `json:"task"`
	}{newer})
	s.handle("tasks.changed", raw)
	if s.AcceptDurableTask(first) {
		t.Fatal("accepted stale reply")
	}
	if got := s.DurableTask(first.ID); got.OwnerBotID != "new-owner" || got.RunnerID != "new-runner" || got.Revision != 9 {
		t.Fatalf("lost ownership: %+v", got)
	}
	draft := newer.Clone()
	draft.ChatIDs[0] = "changed"
	draft.AcceptanceCriteria[0] = "changed"
	if s.DurableTask(first.ID).ChatIDs[0] != "group" {
		t.Fatal("draft aliases projection")
	}
	s.apply(WireSnapshot{HasIdentity: true, Tasks: []DurableTask{first}})
	if s.DurableTask(first.ID).Revision != 9 {
		t.Fatal("older bootstrap lost newer reply/event")
	}
	s.apply(WireSnapshot{HasIdentity: true})
	if s.DurableTask(first.ID) == nil || s.DurableTask(first.ID).Revision != 9 {
		t.Fatal("bootstrap taken before creation removed a delivered task")
	}
	s.apply(WireSnapshot{HasIdentity: false})
	if len(s.DurableTasks) != 0 {
		t.Fatal("forgotten identity retained tasks")
	}
}

func TestTaskRequestReturnsNewerProjectionAndRejectsAnOldAccountReply(t *testing.T) {
	for _, changeIdentity := range []bool{false, true} {
		t.Run(fmt.Sprint(changeIdentity), func(t *testing.T) {
			posted := make(chan func(), 1)
			fixture := taskFixture()
			s := NewStore(taskTransport{request: func(string, any) (json.RawMessage, error) {
				return json.Marshal(fixture)
			}}, func(fn func()) { posted <- fn }, false)
			s.IdentityID = "first-account"
			called := false
			s.TaskRequest("tasks.get", map[string]any{"id": fixture.ID}, func(task DurableTask, err error) {
				called = true
				if changeIdentity {
					if err == nil || len(s.DurableTasks) != 0 {
						t.Error("previous account task leaked into the new projection")
					}
				} else if err != nil || task.Revision != 9 || task.OwnerBotID != "new-owner" {
					t.Errorf("reply lost newer ownership: %+v %v", task, err)
				}
			})
			fn := <-posted
			if changeIdentity {
				s.IdentityID = "second-account"
			} else {
				newer := fixture.Clone()
				newer.Revision, newer.OwnerBotID = 9, "new-owner"
				s.AcceptDurableTask(newer)
			}
			fn()
			if !called {
				t.Fatal("callback did not rejoin the main queue")
			}
		})
	}
}

func TestTaskRequestFreezesPayloadAndPostsBeforeStoreOrCallback(t *testing.T) {
	requested := make(chan map[string]any, 1)
	posted := make(chan func(), 1)
	fixture := taskFixture()
	s := NewStore(taskTransport{request: func(method string, params any) (json.RawMessage, error) {
		if method != "tasks.update" {
			return nil, errors.New("wrong method")
		}
		var decoded map[string]any
		_ = json.Unmarshal(params.(json.RawMessage), &decoded)
		requested <- decoded
		return json.Marshal(fixture)
	}}, func(fn func()) { posted <- fn }, false)
	ids := []string{"group"}
	params := map[string]any{"id": fixture.ID, "expected_revision": uint64(7), "request_id": "same-key", "chat_ids": ids}
	called := false
	s.TaskRequest("tasks.update", params, func(task DurableTask, err error) {
		called = true
		if err != nil || s.DurableTask(task.ID) == nil {
			t.Errorf("callback before projection: %v", err)
		}
	})
	ids[0] = "mutated"
	params["request_id"] = "mutated"
	request := <-requested
	if request["request_id"] != "same-key" || request["chat_ids"].([]any)[0] != "group" {
		t.Fatalf("mutable payload: %+v", request)
	}
	fn := <-posted
	if called || len(s.DurableTasks) != 0 {
		t.Fatal("worker mutated UI state")
	}
	fn()
	if !called {
		t.Fatal("callback not posted")
	}
}

func TestTaskAPIErrorIsPostedAndDoesNotUpdateProjection(t *testing.T) {
	posted := make(chan func(), 1)
	fixture := taskFixture()
	s := NewStore(taskTransport{request: func(_ string, _ any) (json.RawMessage, error) {
		return nil, errors.New("Task revision conflict: expected 7, current 8")
	}}, func(fn func()) { posted <- fn }, false)
	s.AcceptDurableTask(fixture)
	called := false
	s.TaskRequest("tasks.update", map[string]any{"id": fixture.ID}, func(_ DurableTask, err error) {
		called = true
		if err == nil || !strings.Contains(err.Error(), "revision conflict") {
			t.Errorf("lost error: %v", err)
		}
	})
	(<-posted)()
	if !called || s.DurableTask(fixture.ID).Revision != 7 {
		t.Fatal("error changed projection")
	}
}
