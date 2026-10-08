package model

import (
	"encoding/json"
	"errors"
	"strings"
	"testing"
	"time"
)

type templateTransport struct {
	request func(string, any) (json.RawMessage, error)
}

func (transport templateTransport) Request(method string, params any) (json.RawMessage, error) {
	return transport.request(method, params)
}
func (templateTransport) Reconnect() {}

func templatePosted(t *testing.T, queue <-chan func()) func() {
	t.Helper()
	select {
	case fn := <-queue:
		return fn
	case <-time.After(time.Second):
		t.Fatal("no ordered reply posted")
		return nil
	}
}

func TestTemplateRepliesReadTheCLIsShape(t *testing.T) {
	contents := decodeJSON[TemplateContents](t, `{"profile":{"id":"profile","content":{"name":"Reviewer","description":"Read carefully","symbol_name":"checklist","accent":"blue"},"flags":["credential"]},"memories":[{"id":"memory-1","content":"- Email ops@example.com","flags":["email"]}],"routines":[{"id":"rt-1","content":{"name":"Morning","schedule":"every 2h","prompt":"Read inbox"}}],"requirements":[{"service_id":"github","name":"GitHub"}]}`)
	if contents.Profile.Content.Name != "Reviewer" || contents.Profile.Flags[0] != "credential" || contents.Memories[0].Flags[0] != "email" || contents.Routines[0].ID != "rt-1" || contents.Requirements[0].Name != "GitHub" {
		t.Fatalf("contents lost a field: %+v", contents)
	}
	preview := decodeJSON[TemplatePreview](t, `{"digest":"reviewed","can_import":false,"issues":[],"template":{"profile":{"name":"Reviewer"},"routines":[{"name":"Morning","schedule":"every 2h","schedule_text":"Every 2 hours"}]},"requirements":[{"service_id":"google-drive","name":"Google Drive","selected":"google-drive-work","candidates":[{"id":"google-drive-work","name":"Drive · Work","state":"needs_auth"}]}]}`)
	if preview.CanImport || preview.Digest != "reviewed" || preview.Template.Routines[0].ScheduleText != "Every 2 hours" || preview.Requirements[0].Ready() {
		t.Fatalf("lost blocker/digest/schedule: %+v", preview)
	}
	preview.Requirements[0].Candidates[0].State = "ready"
	if !preview.Requirements[0].Ready() {
		t.Fatal("a ready pick is not ready")
	}
	unsupported := decodeJSON[TemplatePreview](t, `{"digest":"v2","can_import":false,"issues":["Unsupported template version 2"],"requirements":[]}`)
	if unsupported.Template != nil || unsupported.CanImport || unsupported.Issues[0] != "Unsupported template version 2" {
		t.Fatal("unsupported file became importable")
	}
}

func TestTemplateRequestsFreezeFieldsAndWaitForMainThread(t *testing.T) {
	posts := make(chan func(), 4)
	requests := make(chan map[string]any, 4)
	transport := templateTransport{request: func(method string, params any) (json.RawMessage, error) {
		data, _ := json.Marshal(params)
		var payload map[string]any
		_ = json.Unmarshal(data, &payload)
		payload["method"] = method
		requests <- payload
		return json.RawMessage(`{"digest":"same","can_import":false,"issues":["Select your own connection"],"requirements":[]}`), nil
	}}
	store := NewStore(transport, func(fn func()) { posts <- fn }, false)
	mappings := map[string]string{"google-drive": "recipient-owned-instance"}
	called := false
	store.PreviewTemplateImport(TemplateImportOptions{Path: "C:\\Demo\\reviewer.lorca-template", RunnerID: "recipient-runner", Name: "Reviewed name", Mappings: mappings}, func(preview TemplatePreview, err error) {
		called = true
		if err != nil || preview.CanImport {
			t.Fatal("incorrect preview reply")
		}
	})
	mappings["google-drive"] = "other-instance"
	fn := templatePosted(t, posts)
	if called {
		t.Fatal("callback ran on the worker")
	}
	fn()
	payload := <-requests
	if payload["name"] != "Reviewed name" || payload["path"] != "C:\\Demo\\reviewer.lorca-template" || payload["mappings"].(map[string]any)["google-drive"] != "recipient-owned-instance" || payload["method"] != "templates.import.preview" {
		t.Fatalf("mutable params leaked into request: %+v", payload)
	}
	if !called {
		t.Fatal("posted callback did not run")
	}
	selection := TemplateSelection{Profile: true, MemoryIDs: []string{"selected-memory"}}
	store.PreviewTemplateExport("source-bot", selection, func(TemplatePreview, error) {})
	selection.MemoryIDs[0] = "later-edit"
	templatePosted(t, posts)()
	export := <-requests
	if export["selection"].(map[string]any)["memory_ids"].([]any)[0] != "selected-memory" {
		t.Fatal("selection did not freeze")
	}
}

func TestTemplateImportReconcilesIndependentBotAfterOrderedReplyOnly(t *testing.T) {
	posts := make(chan func(), 4)
	var sent map[string]any
	transport := templateTransport{request: func(method string, params any) (json.RawMessage, error) {
		if method != "templates.import" {
			return nil, errors.New("unexpected mutation " + method)
		}
		data, _ := json.Marshal(params)
		_ = json.Unmarshal(data, &sent)
		return json.RawMessage(`{"bot":{"id":"bot-recipient-new","name":"Independent","description":"Review","symbol_name":"checklist","accent":"blue","runner_id":"recipient-runner","provider":"deepseek","created_at":1},"chat_id":"dm-recipient-new"}`), nil
	}}
	store := NewStore(transport, func(fn func()) { posts <- fn }, false)
	store.Bots = []*Bot{{ID: "source-bot", Name: "Original", RunnerID: "source-runner"}}
	store.Chats = []*Chat{{ID: "source-dm", Kind: ChatDM, BotIDs: []string{"source-bot"}, Messages: []*Message{{ID: "private-message"}}}}
	store.Routines = []*Routine{{ID: "original-routine", BotID: "source-bot", IsEnabled: true}}
	var chatID string
	var events []EventKind
	store.Subscribe(func(event Event) { events = append(events, event.Kind) })
	store.ImportTemplate(TemplateImportOptions{Path: "reviewer.lorca-template", RunnerID: "recipient-runner", Mappings: map[string]string{"github": "own-account"}, ExpectedDigest: "reviewed-file", Reviewed: true}, func(id string, err error) {
		if err != nil {
			t.Fatal(err)
		}
		chatID = id
	})
	fn := templatePosted(t, posts)
	if len(store.Bots) != 1 || chatID != "" {
		t.Fatal("worker changed the store before posting")
	}
	fn()
	if chatID != "dm-recipient-new" || store.Bot("bot-recipient-new") == nil || len(store.Chat(chatID).Messages) != 0 || len(store.Routines) != 1 || !store.Routines[0].IsEnabled {
		t.Fatal("import copied history, altered source state or duplicated routines")
	}
	if len(events) != 2 || events[0] != EventRosterChanged || events[1] != EventChatsChanged {
		t.Fatalf("wrong ordered events: %v", events)
	}
	if sent["expected_digest"] != "reviewed-file" || sent["reviewed"] != true || sent["mappings"].(map[string]any)["github"] != "own-account" {
		t.Fatal("review/mapping lost")
	}
	// A repeated roster/response does not append the same identities again.
	store.ImportTemplate(TemplateImportOptions{}, func(string, error) {})
	templatePosted(t, posts)()
	if len(store.Bots) != 2 || len(store.Chats) != 2 {
		t.Fatal("reply reconciliation duplicated records")
	}
}

func TestTemplateErrorsRemainVisibleWithoutCreatingRecords(t *testing.T) {
	posts := make(chan func(), 1)
	store := NewStore(templateTransport{request: func(string, any) (json.RawMessage, error) {
		return nil, errors.New("The contents changed since the preview. Preview and review them again.")
	}}, func(fn func()) { posts <- fn }, false)
	store.ImportTemplate(TemplateImportOptions{Reviewed: true, ExpectedDigest: "stale"}, func(id string, err error) {
		if id != "" || err == nil || !strings.Contains(ErrorText(err), "changed since") {
			t.Fatal("stale review was accepted")
		}
	})
	templatePosted(t, posts)()
	if len(store.Bots)+len(store.Chats) > 0 {
		t.Fatal("failed import created records")
	}
}
