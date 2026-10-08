package model

import (
	"encoding/json"
	"errors"
	"sync"
	"testing"
	"time"
)

type projectTransport struct {
	request func(string, json.RawMessage) (json.RawMessage, error)
}

func (p projectTransport) Reconnect() {}
func (p projectTransport) Request(method string, params any) (json.RawMessage, error) {
	raw, err := json.Marshal(params)
	if err != nil {
		return nil, err
	}
	return p.request(method, raw)
}
func projectStore(transport Transport) (*Store, chan func()) {
	queue := make(chan func(), 16)
	s := NewStore(transport, func(fn func()) { queue <- fn }, false)
	s.Chats = []*Chat{{ID: "project", Kind: ChatGroup}, {ID: "dm", Kind: ChatDM}}
	return s, queue
}
func posted(t *testing.T, queue chan func()) {
	t.Helper()
	select {
	case fn := <-queue:
		fn()
	case <-time.After(3 * time.Second):
		t.Fatal("no main-thread reply")
	}
}

func TestProjectContextPagesAndImmutableProvenance(t *testing.T) {
	var mu sync.Mutex
	var calls []string
	transport := projectTransport{request: func(method string, params json.RawMessage) (json.RawMessage, error) {
		var input map[string]any
		if err := json.Unmarshal(params, &input); err != nil {
			return nil, err
		}
		if input["chat_id"] != "project" {
			t.Errorf("wrong scope: %s", params)
		}
		mu.Lock()
		calls = append(calls, method)
		mu.Unlock()
		if method == "projects.get" {
			if input["after"] == nil {
				return json.RawMessage(`{"chat_id":"project","revision":"r1","has_more":true,"max_context_bytes":8000,"conflicts":{},"entries":[{"id":"a","kind":"fact","title":"Build","text":"verified report","verification":"verified","freshness":"verified","current":true,"updated_at":100,"verified_at":90,"source":{"kind":"output","label":"Build report","output":{"chat_id":"project","message_id":"version-message","output_id":"output-1","version":2,"task_id":"task-uuid"}}}]}`), nil
			}
			if input["after"] != "a" {
				t.Errorf("wrong cursor: %s", params)
			}
			return json.RawMessage(`{"chat_id":"project","revision":"r1","has_more":false,"max_context_bytes":8000,"conflicts":{},"entries":[{"id":"b","kind":"asset","title":"Evidence","text":"","verification":"agreed","current":true,"updated_at":101,"asset":{"id":"att-b","name":"evidence.pdf","mime":"application/pdf","size":40},"source":{"kind":"user","label":"User"}}]}`), nil
		}
		if method == "projects.save" {
			if input["expected_revision"] != "r1" {
				t.Errorf("missing revision: %s", params)
			}
			source := input["source"].(map[string]any)
			ref := source["output"].(map[string]any)
			if ref["message_id"] != "version-message" || ref["task_id"] != "task-uuid" || ref["version"] != float64(2) {
				t.Errorf("provenance lost: %s", params)
			}
			return json.RawMessage(`{"id":"c","kind":"fact","title":"Corrected","text":"updated","source":{"kind":"user","label":"User"},"verification":"agreed","updated_at":110}`), nil
		}
		return nil, errors.New("unexpected call")
	}}
	s, queue := projectStore(transport)
	var page ProjectContext
	done := false
	s.ProjectContext("project", false, func(value ProjectContext, err error) {
		if err != nil {
			t.Error(err)
		}
		page, done = value, true
	})
	// Work may finish, but a reply does not mutate the UI before the ordered post is drained.
	time.Sleep(10 * time.Millisecond)
	if done {
		t.Fatal("callback ran outside the main-thread queue")
	}
	posted(t, queue)
	if len(page.Entries) != 2 || page.MaxContextBytes != 8000 || page.Entries[1].Asset.Name != "evidence.pdf" {
		t.Fatalf("bad page: %+v", page)
	}
	s.SaveProjectContext("project", ProjectContextSave{Kind: "fact", Title: "Corrected", Source: page.Entries[0].Source, Supersedes: []string{"a"}, ExpectedRevision: page.Revision}, func(entry ProjectEntry, err error) {
		if err != nil || entry.ID != "c" {
			t.Errorf("save: %+v %v", entry, err)
		}
	})
	posted(t, queue)
	mu.Lock()
	defer mu.Unlock()
	if len(calls) != 3 {
		t.Fatalf("calls: %v", calls)
	}
}

func TestProjectContextRejectsMixedScopeAndRevision(t *testing.T) {
	for _, second := range []string{
		`{"chat_id":"different-project","revision":"r1","entries":[],"has_more":false}`,
		`{"chat_id":"project","revision":"r2","entries":[],"has_more":false}`,
		`{"chat_id":"project","revision":"r1","entries":[],"has_more":true}`,
	} {
		calls := 0
		s, queue := projectStore(projectTransport{request: func(_ string, _ json.RawMessage) (json.RawMessage, error) {
			calls++
			if calls == 1 {
				return json.RawMessage(`{"chat_id":"project","revision":"r1","has_more":true,"entries":[{"id":"a","source":{"kind":"user","label":"User"}}]}`), nil
			}
			return json.RawMessage(second), nil
		}})
		s.ProjectContext("project", false, func(_ ProjectContext, err error) {
			if err == nil {
				t.Error("accepted mixed or non-advancing context")
			}
		})
		posted(t, queue)
	}
}

func TestProjectContextScopeAndActionWire(t *testing.T) {
	calls := make(chan string, 8)
	s, queue := projectStore(projectTransport{request: func(method string, params json.RawMessage) (json.RawMessage, error) {
		var input map[string]any
		_ = json.Unmarshal(params, &input)
		if input["chat_id"] != "project" {
			t.Errorf("wrong scope: %s", params)
		}
		calls <- method
		if method == "projects.asset_path" {
			return json.RawMessage(`{"path":"/fixture/reference.pdf"}`), nil
		}
		return json.RawMessage(`{"id":"entry","kind":"fact","title":"Reference","source":{"kind":"url","label":"Source"},"verification":"unavailable","updated_at":100,"refresh_error":"offline"}`), nil
	}})
	s.RefreshProjectContext("project", "entry", func(entry ProjectEntry, err error) {
		if err != nil || entry.Verification != "unavailable" || entry.RefreshError != "offline" {
			t.Error("refresh state lost", err)
		}
	})
	posted(t, queue)
	s.AddProjectAsset("project", "/fixture/reference.pdf", func(_ ProjectEntry, err error) {
		if err != nil {
			t.Error(err)
		}
	})
	posted(t, queue)
	s.ProjectAssetPath("project", "entry", func(path string, err error) {
		if err != nil || path != "/fixture/reference.pdf" {
			t.Error(path, err)
		}
	})
	posted(t, queue)
	for _, name := range []string{"projects.refresh", "projects.asset", "projects.asset_path"} {
		if got := <-calls; got != name {
			t.Error(got, name)
		}
	}
	s.ProjectContext("dm", false, func(_ ProjectContext, err error) {
		if err == nil {
			t.Error("DM accepted as project")
		}
	})
	posted(t, queue)
	select {
	case method := <-calls:
		t.Error("DM invoked", method)
	default:
	}
	s.isBootstrapping = false
	seen := ""
	s.Subscribe(func(event Event) {
		if event.Kind == EventProjectContextChanged {
			seen = event.ChatID
		}
	})
	s.HandleEvent("projects.changed", json.RawMessage(`{"chat_id":"project","entry_id":"entry"}`))
	if seen != "project" {
		t.Error("project event not routed", seen)
	}
}
