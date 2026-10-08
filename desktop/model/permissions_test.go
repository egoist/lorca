package model

import (
	"encoding/json"
	"errors"
	"reflect"
	"testing"
	"time"
)

func TestPermissionsWirePreservesFullAndEmptyGrants(t *testing.T) {
	base := `{"id":"b","name":"Inbox","provider":"deepseek","created_at":1,"permissions":%s}`
	full := ToBot(decodeJSON[WireBot](t, sprintf(base, `{}`))).Permissions
	if full == nil || full.Tools != nil || full.Connections != nil || !full.Shell || full.Filesystem != "write" {
		t.Fatalf("full policy: %+v", full)
	}
	none := ToBot(decodeJSON[WireBot](t, sprintf(base, `{"connections":{},"tools":[],"filesystem":"none","shell":false}`))).Permissions
	data, err := json.Marshal(none)
	if err != nil {
		t.Fatal(err)
	}
	var wire map[string]any
	_ = json.Unmarshal(data, &wire)
	if !reflect.DeepEqual(wire["connections"], map[string]any{}) || !reflect.DeepEqual(wire["tools"], []any{}) || wire["shell"] != false {
		t.Fatalf("empty grant widened: %s", data)
	}
	if ToBot(decodeJSON[WireBot](t, sprintf(base, `null`))).Permissions != nil {
		t.Fatal("legacy policy should stay absent")
	}
	copy := none.Clone()
	(*copy.Connections)["work"] = ConnectionPermissions{Capabilities: []string{"read"}}
	if len(*none.Connections) != 0 {
		t.Fatal("editor clone modified stored grants")
	}
}

func TestAccessDraftKeepsEditedChoicesAndUnavailableExactNames(t *testing.T) {
	policy := decodeJSON[BotPermissions](t, `{"connections":{"gone":{"capabilities":["draft"],"tools":["saved_tool"]}},"tools":["future_tool"],"filesystem":"read","shell":false}`)
	draft := NewAccessDraft(&policy)
	draft.MergeCatalog(MockBotPermissionCatalog())
	work := draft.Connections["gmail-11111111111111111111111111111111"]
	if work.Read || work.Write || work.Draft {
		t.Fatal("newly discovered connection inherited a grant")
	}
	work.Read, work.AllTools = true, false
	work.Tools["list_messages"].Selected = true
	draft.MergeCatalog(MockBotPermissionCatalog())
	if !work.Read || work.AllTools || !work.Tools["list_messages"].Selected {
		t.Fatal("catalog reload erased edits")
	}
	saved := draft.Policy()
	if saved.Shell || saved.Filesystem != "read" || !reflect.DeepEqual((*saved.Connections)["gone"].Tools, (*policy.Connections)["gone"].Tools) {
		t.Fatal("lost saved grants")
	}
	if !reflect.DeepEqual(*saved.Tools, []string{"future_tool"}) {
		t.Fatal("lost unknown local name")
	}
}

type permissionTransport struct {
	request func(string, any) (json.RawMessage, error)
}

func (t *permissionTransport) Request(method string, params any) (json.RawMessage, error) {
	return t.request(method, params)
}
func (*permissionTransport) Reconnect() {}

func permissionPost(t *testing.T, posts chan func()) {
	t.Helper()
	select {
	case fn := <-posts:
		fn()
	case <-time.After(3 * time.Second):
		t.Fatal("missing ordered reply")
	}
}

func TestPermissionSaveUsesCLIAndOrderedReplyWithoutOverwritingProfile(t *testing.T) {
	posts := make(chan func(), 8)
	entered := make(chan map[string]any, 1)
	release := make(chan struct{})
	transport := &permissionTransport{request: func(method string, params any) (json.RawMessage, error) {
		if method != "bots.update" {
			return nil, errors.New("unexpected method")
		}
		data, _ := json.Marshal(params)
		var request map[string]any
		_ = json.Unmarshal(data, &request)
		entered <- request
		<-release
		return json.RawMessage(`{"bot":{"id":"b","permissions":{"connections":{},"tools":[],"filesystem":"none","shell":false}}}`), nil
	}}
	store := NewStore(transport, func(fn func()) { posts <- fn }, false)
	store.Bots = []*Bot{{ID: "b", Name: "Inbox"}}
	policy := NewAccessDraft(nil)
	policy.AllConnections, policy.AllTools, policy.Shell, policy.Filesystem = false, false, false, "none"
	called := false
	store.SetBotPermissions("b", policy.Policy(), func(err error) {
		if err != nil {
			t.Error(err)
		}
		called = true
	})
	request := <-entered
	if request["id"] != "b" || request["permissions"].(map[string]any)["shell"] != false {
		t.Fatalf("wrong RPC: %#v", request)
	}
	if store.Bot("b").Permissions != nil || called {
		t.Fatal("worker mutated main-thread state")
	}
	store.Bot("b").Name = "Newer name"
	close(release)
	permissionPost(t, posts)
	if !called || store.Bot("b").Permissions.Shell || store.Bot("b").Name != "Newer name" {
		t.Fatal("save lost newer profile or policy")
	}
}

func TestOldPermissionReplyDoesNotOverwriteNewerRosterPolicy(t *testing.T) {
	posts := make(chan func(), 8)
	transport := &permissionTransport{request: func(string, any) (json.RawMessage, error) {
		return json.RawMessage(`{"bot":{"id":"b","permissions":{"filesystem":"none","shell":false}}}`), nil
	}}
	store := NewStore(transport, func(fn func()) { posts <- fn }, false)
	store.Bots = []*Bot{{ID: "b"}}
	var response error
	store.SetBotPermissions("b", &BotPermissions{Filesystem: "none"}, func(err error) { response = err })
	store.Bot("b").Permissions = &BotPermissions{Filesystem: "read", Shell: true}
	permissionPost(t, posts)
	if response == nil || !store.Bot("b").Permissions.Shell {
		t.Fatal("late save widened/replaced newer policy")
	}
}

func TestAccessRequestCannotBeAnsweredAsAnActionApproval(t *testing.T) {
	store := NewStore(nil, func(fn func()) { fn() }, true)
	store.Start()
	request := &PermissionRequest{PluginID: "computer", Tool: "access", Decision: DecisionPending}
	store.Append(&Message{ID: "access", Author: BotAuthor("bot-nova"), Body: Body{Kind: BodyPermission, Request: request}}, "chat-nova")
	if request.IsShell() || !reflect.DeepEqual(request.Choices(), []Answer{{L("Edit Access…"), "access"}, {L("Dismiss"), "deny"}}) {
		t.Fatal("access card offers authority")
	}
	for _, answer := range []string{"allow", "always", "access"} {
		store.AnswerPermission("chat-nova", "access", answer)
	}
	if store.Chat("chat-nova").Messages[len(store.Chat("chat-nova").Messages)-1].Body.Request.Decision != DecisionPending {
		t.Fatal("card granted action")
	}
	store.AnswerPermission("chat-nova", "access", "deny")
	if store.Chat("chat-nova").Messages[len(store.Chat("chat-nova").Messages)-1].Body.Request.Decision != DecisionDenied {
		t.Fatal("cannot dismiss request")
	}
}
