package model

import (
	"encoding/json"
	"errors"
	"reflect"
	"slices"
	"testing"
)

func TestPermissionsWireKeepsFullAndEmptyGrantsApart(t *testing.T) {
	base := `{"id":"b","name":"Inbox","provider":"deepseek","created_at":1,"permissions":%s}`
	full := ToBot(decodeJSON[WireBot](t, sprintf(base, `{}`))).Permissions
	if full == nil || full.Connections != nil || !full.Shell || full.Filesystem != AccessWrite || full.Level("github") != AccessWrite || full.Summary() != L("Full access") {
		t.Fatalf("full policy: %+v", full)
	}
	none := ToBot(decodeJSON[WireBot](t, sprintf(base, `{"connections":{},"filesystem":"none","shell":false}`))).Permissions
	data, err := json.Marshal(none)
	if err != nil {
		t.Fatal(err)
	}
	var wire map[string]any
	_ = json.Unmarshal(data, &wire)
	if !reflect.DeepEqual(wire["connections"], map[string]any{}) || wire["shell"] != false || wire["filesystem"] != "none" {
		t.Fatalf("empty grant widened: %s", data)
	}
	if none.Level("github") != AccessNone || none.Summary() != L("Limited") {
		t.Fatal("a plugin the map leaves out should be off")
	}
	if ToBot(decodeJSON[WireBot](t, sprintf(base, `null`))).Permissions != nil {
		t.Fatal("a bot with no policy should have none")
	}
	copy := none.Clone()
	(*copy.Connections)["work"] = ConnectionPermissions{Capabilities: []string{"read"}}
	if len(*none.Connections) != 0 {
		t.Fatal("a clone shares its grants")
	}
}

func TestLevelsTakeInTheOnesBelowThem(t *testing.T) {
	if LevelOf([]string{"read", "draft"}) != AccessDraft || LevelOf(nil) != AccessNone {
		t.Fatal("level of capabilities")
	}
	if !AccessDraft.Allows("read") || AccessDraft.Allows("write") || !reflect.DeepEqual(AccessWrite.Capabilities(), []string{"read", "draft", "write"}) {
		t.Fatal("levels")
	}
}

func TestAccessRequestOnlyOpensAccessOrIsDismissed(t *testing.T) {
	store := NewStore(nil, func(fn func()) { fn() }, true)
	store.Start()
	request := &PermissionRequest{PluginID: "computer", Tool: "access", Summary: "Shell commands", Decision: DecisionPending}
	store.Append(&Message{ID: "access", Author: BotAuthor("bot-nova"), Body: Body{Kind: BodyPermission, Request: request}}, "chat-nova")
	if request.IsShell() || !reflect.DeepEqual(request.Choices(), []Answer{{L("Edit Access…"), "access"}, {L("Dismiss"), "deny"}}) {
		t.Fatal("an access request offers to allow the call")
	}
	if request.ShownSummary() != L("Shell commands") || request.ShownReason() != L("Not allowed in this bot's Access settings.") {
		t.Fatal("access request wording")
	}
	store.AnswerPermission("chat-nova", "access", "deny")
	messages := store.Chat("chat-nova").Messages
	if messages[len(messages)-1].Body.Request.Decision != DecisionDismissed {
		t.Fatal("Dismiss should dismiss the request")
	}
}

func TestAnswerQuestionSendsAnswersByQuestion(t *testing.T) {
	posts := make(chan func(), 4)
	requests := make(chan map[string]any, 4)
	transport := templateTransport{request: func(method string, params any) (json.RawMessage, error) {
		data, _ := json.Marshal(params)
		var payload map[string]any
		_ = json.Unmarshal(data, &payload)
		payload["method"] = method
		requests <- payload
		return json.RawMessage(`{}`), nil
	}}
	store := NewStore(transport, func(fn func()) { posts <- fn }, false)
	store.Chats = []*Chat{{ID: "chat-1", Messages: []*Message{{ID: "ask", Author: BotAuthor("bot-1"), Body: Body{Kind: BodyPermission, Request: &PermissionRequest{
		PluginID: "coding_agent", PluginName: "Claude Code", Tool: "question", Summary: "Which database?", Decision: DecisionPending,
		Question: &QuestionAsk{Questions: []Question{{ID: "Which database?", Text: "Which database?"}}},
	}}}}}}
	answers := map[string]string{"Which database?": "SQLite"}
	var answered error = errors.New("not yet")
	store.AnswerQuestion("chat-1", "ask", answers, func(err error) { answered = err })
	answers["Which database?"] = "later edit"
	templatePosted(t, posts)()
	payload := <-requests
	want := map[string]any{"method": "chats.permission", "chat_id": "chat-1", "message_id": "ask", "decision": "allow", "answers": map[string]any{"Which database?": "SQLite"}}
	if !reflect.DeepEqual(payload, want) {
		t.Fatalf("payload %+v", payload)
	}
	if answered != nil {
		t.Fatalf("answer failed: %v", answered)
	}
	request := store.Chat("chat-1").Messages[0].Body.Request
	if request.Decision != DecisionAllowed || request.Question.Answers["Which database?"] != "SQLite" || request.DecisionText() != L("Answered") {
		t.Fatalf("the card did not take the answer: %+v", request)
	}
}

func TestQuestionWording(t *testing.T) {
	request := &PermissionRequest{PluginID: "computer", Tool: "question", Question: &QuestionAsk{}, Decision: DecisionDenied}
	if request.IsShell() || request.DecisionText() != L("Skipped") {
		t.Fatal("a question reads as a shell card")
	}
	request.Decision = DecisionExpired
	if request.DecisionText() != L("Expired") {
		t.Fatal("expired question")
	}
	if CodingAgentName("lorca") != L("Subagent") || slices.Contains(CodingAgentKinds, "lorca") {
		t.Fatal("a sub agent is not a coding agent to pick")
	}
}

func TestMockWriterAccessMatchesTheCatalog(t *testing.T) {
	access := mockWriterAccess()
	if access.Level("github") != AccessDraft || access.Level("linear") != AccessNone || access.Shell {
		t.Fatalf("writer access: %+v", access)
	}
	if len(MockAccessCatalog().Connections) != 5 {
		t.Fatal("the catalog lists Workbench's enabled plugins")
	}
}
