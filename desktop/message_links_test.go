package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/url"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

type handoffPageReply struct {
	data json.RawMessage
	err  error
}
type handoffPageCall struct {
	params map[string]any
	reply  chan handoffPageReply
}
type handoffPageTransport struct{ requests chan handoffPageCall }

func (h *handoffPageTransport) Reconnect() {}
func (h *handoffPageTransport) Request(method string, params any) (json.RawMessage, error) {
	if method != "chats.messages" {
		return json.RawMessage(`{}`), nil
	}
	request := handoffPageCall{params: params.(map[string]any), reply: make(chan handoffPageReply, 1)}
	h.requests <- request
	reply := <-request.reply
	return reply.data, reply.err
}

type handoffUIFixture struct {
	m         *mainWindow
	tt        *ui.Tester
	transport *handoffPageTransport
	mu        sync.Mutex
	posts     []func()
}

func handoffFixture(t *testing.T, needsPages bool) *handoffUIFixture {
	t.Helper()
	m := demoWindow(t)
	seed := store
	f := &handoffUIFixture{m: m, transport: &handoffPageTransport{requests: make(chan handoffPageCall, 8)}}
	store = model.NewStore(f.transport, func(fn func()) { f.mu.Lock(); f.posts = append(f.posts, fn); f.mu.Unlock() }, false)
	store.Devices, store.Models = seed.Devices, seed.Models
	store.IsStarting, store.IsConnected, store.HasIdentity = false, true, seed.HasIdentity
	store.Bots = []*model.Bot{
		{ID: "fixture-chef", Name: "Chef", SymbolName: "sparkles", Accent: "indigo", RunnerID: "dev-workbench", Provider: "deepseek"},
		{ID: "fixture-scout", Name: "Scout", SymbolName: "magnifyingglass", Accent: "teal", RunnerID: "dev-studio", Provider: "deepseek"},
	}
	at := time.Now().Add(-5 * time.Minute)
	report := "Handoff handoff-00000000-0000-4000-8000-000000000074 · completed\n\nParser review complete. The empty-field fixture now returns a clear error.\n\n- [Recipient response](lorca://message?chat_id=fixture-scout-chat&message_id=fixture-report)\n\nEvidence:\n- The failing input reproduced the unchecked index.\n- Three synthetic fixture checks passed."
	store.Chats = []*model.Chat{
		{ID: "fixture-chef-chat", Kind: model.ChatDM, BotIDs: []string{"fixture-chef"}, Messages: []*model.Message{
			handoffMessage("fixture-request", model.You, "Ask Scout to review the parser. I need a report and a failing input.", at),
			{ID: "fixture-delivery", Author: model.BotAuthor("fixture-chef"), Body: model.Body{Kind: model.BodyTool, Tool: &model.ToolInvocation{Name: "message_bot", Summary: "Messaged Scout", Detail: "Delegated with expected output and acceptance criteria", TargetBotID: "fixture-scout"}}, State: model.MessageState{Kind: model.StateComplete}, CreatedAt: at.Add(time.Second)},
			handoffMessage("fixture-result", model.BotAuthor("fixture-scout"), report, at.Add(2*time.Second)),
		}},
		{ID: "fixture-scout-chat", Kind: model.ChatDM, BotIDs: []string{"fixture-scout"}, HasMore: needsPages},
	}
	target := store.Chat("fixture-scout-chat")
	if needsPages {
		target.Messages = handoffRows("recent", 18, at.Add(time.Minute))
	} else {
		target.Messages = []*model.Message{handoffResponse(at)}
	}
	m.userWantsInspector = false
	app.main = m
	store.Subscribe(m.storeChanged)
	m.selectChat("fixture-chef-chat")
	f.tt = ui.NewTester(m.frame(m.view), 1180, 760)
	f.frames()
	return f
}
func handoffMessage(id string, author model.Author, text string, at time.Time) *model.Message {
	return &model.Message{ID: id, Author: author, Body: model.Body{Kind: model.BodyText, Text: text}, CreatedAt: at, State: model.MessageState{Kind: model.StateComplete}}
}
func handoffResponse(at time.Time) *model.Message {
	return handoffMessage("fixture-report", model.BotAuthor("fixture-scout"), "Parser review complete.\n\nThe empty-field fixture reproduced an unchecked index. The fix checks the field count before indexing.\n\n- Failing input: `name,,status`\n- Added the regression case to the fixture suite.\n- The three fixture checks pass.", at)
}
func handoffRows(prefix string, count int, at time.Time) []*model.Message {
	var rows []*model.Message
	for i := range count {
		rows = append(rows, handoffMessage(fmt.Sprintf("%s-%02d", prefix, i), model.BotAuthor("fixture-scout"), fmt.Sprintf("Synthetic context note %d for this parser review.", i+1), at.Add(time.Duration(i)*time.Second)))
	}
	return rows
}
func handoffWirePage(rows []*model.Message, more bool) json.RawMessage {
	messages := make([]map[string]any, 0, len(rows))
	for _, row := range rows {
		messages = append(messages, map[string]any{"id": row.ID, "chat_id": "fixture-scout-chat", "author": map[string]string{"kind": "bot", "bot_id": "fixture-scout"}, "body": map[string]string{"kind": "text", "text": row.Body.Text}, "state": map[string]string{"kind": "complete"}, "created_at": float64(row.CreatedAt.Unix())})
	}
	data, _ := json.Marshal(map[string]any{"messages": messages, "has_more": more})
	return data
}
func (f *handoffUIFixture) frames() {
	for range 6 {
		f.mu.Lock()
		posts := f.posts
		f.posts = nil
		f.mu.Unlock()
		for _, fn := range posts {
			fn()
		}
		f.tt.Frame()
	}
}
func (f *handoffUIFixture) wait(t *testing.T, what string, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for !condition() {
		if time.Now().After(deadline) {
			t.Fatalf("waiting for %s; text %q", what, f.tt.Texts())
		}
		f.frames()
		time.Sleep(5 * time.Millisecond)
	}
	f.frames()
}
func (f *handoffUIFixture) take(t *testing.T) handoffPageCall {
	t.Helper()
	select {
	case request := <-f.transport.requests:
		return request
	case <-time.After(3 * time.Second):
		t.Fatal("no history request")
		return handoffPageCall{}
	}
}
func (f *handoffUIFixture) open(t *testing.T) {
	t.Helper()
	clickHandoffLink(t, f.tt, "Recipient response")
	f.frames()
	if f.m.selection.ChatID != "fixture-scout-chat" {
		t.Fatal("link did not select the recipient chat")
	}
}

func TestHandoffLinkOpensLoadedResponseWithoutSystemOrHistoryRequest(t *testing.T) {
	f := handoffFixture(t, false)
	renderTo(t, f.tt, "desktop-handoff-completed")
	f.open(t)
	if f.m.chat.flashID != "fixture-report" || f.m.chat.linkedMessageID != "" || !f.tt.HasText("Parser review complete.") {
		t.Fatalf("no revealed response: %q", f.tt.Texts())
	}
	select {
	case <-f.transport.requests:
		t.Fatal("loaded response requested history")
	default:
	}
}

func TestHandoffLinkLoadsPagesAndRevealsMessageByStableID(t *testing.T) {
	f := handoffFixture(t, true)
	f.open(t)
	first := f.take(t)
	if first.params["chat_id"] != "fixture-scout-chat" || first.params["before"] != "recent-00" {
		t.Fatalf("first params %+v", first.params)
	}
	if !f.tt.HasText(L("Loading linked message…")) {
		t.Fatal("no paging state")
	}
	renderTo(t, f.tt, "desktop-handoff-loading-history")
	middle := handoffRows("middle", 8, time.Now().Add(-4*time.Minute))
	first.reply <- handoffPageReply{data: handoffWirePage(middle, true)}
	f.wait(t, "second history page", func() bool { return len(f.transport.requests) > 0 })
	second := f.take(t)
	if second.params["before"] != "middle-00" {
		t.Fatalf("second params %+v", second.params)
	}
	second.reply <- handoffPageReply{data: handoffWirePage([]*model.Message{handoffResponse(time.Now().Add(-5 * time.Minute))}, false)}
	f.wait(t, "linked response", func() bool { return f.m.chat.linkedMessageID == "" })
	if f.m.chat.flashID != "fixture-report" || !f.tt.HasText("Parser review complete.") || f.m.chat.list.AtEnd() {
		t.Fatalf("did not reveal older response: %q", f.tt.Texts())
	}
	if store.Chat("fixture-scout-chat").HasMore {
		t.Fatal("history did not end")
	}
	// Capture after the native reveal pulse, at full opacity.
	time.Sleep(1150 * time.Millisecond)
	f.frames()
	renderTo(t, f.tt, "desktop-handoff-opened-response")
}

func TestHandoffLinkMissingMessageEndsPagingAndShowsUnavailable(t *testing.T) {
	f := handoffFixture(t, true)
	f.open(t)
	request := f.take(t)
	request.reply <- handoffPageReply{data: handoffWirePage(nil, false)}
	f.wait(t, "unavailable message", f.m.hasSheet)
	if !f.tt.HasText(L("The linked message is unavailable.")) || f.m.chat.linkedMessageID != "" {
		t.Fatal("missing-message state absent")
	}
	time.Sleep(250 * time.Millisecond)
	f.frames()
	renderTo(t, f.tt, "desktop-handoff-unavailable")
}

func TestHandoffLinkPageFailureStopsAndCanBeRetried(t *testing.T) {
	f := handoffFixture(t, true)
	f.open(t)
	request := f.take(t)
	request.reply <- handoffPageReply{err: errors.New("Local CLI is offline (synthetic fixture)")}
	f.wait(t, "paging error", f.m.hasSheet)
	if !f.tt.HasText(L("Could not load the linked message.")) {
		t.Fatal("no paging error")
	}
	settleTransitions(f.tt)
	if err := f.tt.Click(L("OK")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(f.tt)
	f.m.openMessage(model.MessageLink{ChatID: "fixture-scout-chat", MessageID: "fixture-report"})
	f.frames()
	retry := f.take(t)
	retry.reply <- handoffPageReply{data: handoffWirePage([]*model.Message{handoffResponse(time.Now().Add(-5 * time.Minute))}, false)}
	f.wait(t, "retried response", func() bool { return f.m.chat.linkedMessageID == "" })
	if !f.tt.HasText("Parser review complete.") {
		t.Fatal("retry did not reveal response")
	}
}

func TestHandoffLinkReplyAfterNavigationDoesNotMoveTheWindow(t *testing.T) {
	f := handoffFixture(t, true)
	f.open(t)
	request := f.take(t)
	f.m.selectChat("fixture-chef-chat")
	f.frames()
	request.reply <- handoffPageReply{data: handoffWirePage([]*model.Message{handoffResponse(time.Now().Add(-5 * time.Minute))}, false)}
	f.wait(t, "old chat page", func() bool { return !store.IsLoadingOlder("fixture-scout-chat") })
	if f.m.selection.ChatID != "fixture-chef-chat" || f.m.hasSheet() {
		t.Fatal("late reply changed the current window")
	}
}

func TestHandoffLinkChangedWhilePagingRevealsOnlyNewestTarget(t *testing.T) {
	f := handoffFixture(t, true)
	f.open(t)
	request := f.take(t)
	f.m.openMessage(model.MessageLink{ChatID: "fixture-scout-chat", MessageID: "second-target"})
	f.frames()
	rows := []*model.Message{handoffResponse(time.Now().Add(-6 * time.Minute)), handoffMessage("second-target", model.BotAuthor("fixture-scout"), "Newest selected result.", time.Now().Add(-5*time.Minute))}
	request.reply <- handoffPageReply{data: handoffWirePage(rows, false)}
	f.wait(t, "replacement link target", func() bool { return f.m.chat.linkedMessageID == "" })
	if f.m.chat.flashID != "second-target" || !f.tt.HasText("Newest selected result.") {
		t.Fatal("older callback replaced latest link")
	}
}

func TestHandoffLinkArrivingThroughSyncDoesNotWaitForFailedPaging(t *testing.T) {
	f := handoffFixture(t, true)
	f.open(t)
	request := f.take(t)
	store.Append(handoffResponse(time.Now()), "fixture-scout-chat")
	f.frames()
	if f.m.chat.linkedMessageID != "" || !f.tt.HasText("Parser review complete.") {
		t.Fatal("synced target not revealed")
	}
	request.reply <- handoffPageReply{err: errors.New("stale failure")}
	f.wait(t, "stale reply", func() bool { return !store.IsLoadingOlder("fixture-scout-chat") })
	if f.m.hasSheet() {
		t.Fatal("stale page failure hid an already-revealed result")
	}
}

func TestHandoffLinkUnknownChatAndExternalAllowlist(t *testing.T) {
	f := handoffFixture(t, false)
	f.m.openMessage(model.MessageLink{ChatID: "unknown", MessageID: "m"})
	f.frames()
	if f.m.selection.ChatID != "fixture-chef-chat" || !f.tt.HasText(L("The linked chat is unavailable.")) {
		t.Fatal("unknown chat changed selection")
	}
	// The same external host boundary continues to reject internal and unrelated schemes.
	for _, href := range []string{"lorca://message?chat_id=c&message_id=m", "file:///private", "javascript:alert(1)"} {
		if err := openExternal(href); err == nil {
			t.Fatalf("system opener accepted %q", href)
		}
	}
}

func TestHandoffBlockerReportRendersWithInternalRequestLink(t *testing.T) {
	f := handoffFixture(t, false)
	report := store.Chat("fixture-chef-chat").Messages[2]
	report.Body.Text = "Handoff handoff-00000000-0000-4000-8000-000000000075 · blocked\n\nI need the schema version before I can verify the mapping.\n\n- [Delegated request](lorca://message?chat_id=fixture-scout-chat&message_id=fixture-report)\n\nEvidence:\n- The supplied context contains no schema version.\n- Verification remains incomplete; no completion is claimed."
	f.frames()
	if !f.tt.HasText("Delegated request") || !strings.Contains(strings.Join(f.tt.Texts(), " "), "blocked") {
		t.Fatal("blocker/report link missing")
	}
	renderTo(t, f.tt, "desktop-handoff-blocked")
	clickHandoffLink(t, f.tt, "Delegated request")
	f.frames()
	if f.m.selection.ChatID != "fixture-scout-chat" {
		t.Fatal("blocker link did not navigate")
	}
}

func TestHandoffLoadingStateUsesCurrentLanguage(t *testing.T) {
	f := handoffFixture(t, true)
	l10n.Set("zh-Hans", "zh-CN")
	t.Cleanup(func() { l10n.Set("en", "en-US") })
	f.open(t)
	request := f.take(t)
	if !f.tt.HasText("正在加载关联的消息…") {
		t.Fatal("loading state not localized")
	}
	request.reply <- handoffPageReply{data: handoffWirePage([]*model.Message{handoffResponse(time.Now())}, false)}
	f.wait(t, "localized navigation", func() bool { return f.m.chat.linkedMessageID == "" })
}

func TestHandoffMessageIDsAreOpaqueAfterURLDecoding(t *testing.T) {
	f := handoffFixture(t, false)
	message := store.Chat("fixture-scout-chat").Messages[0]
	message.ID = "opaque/id+百分号%"
	values := url.Values{"chat_id": []string{"fixture-scout-chat"}, "message_id": []string{message.ID}}
	f.m.openTranscriptLink("lorca://message?" + values.Encode())
	f.frames()
	if f.m.chat.flashID != message.ID {
		t.Fatal("encoded message id was changed")
	}
}

// RichText's tester label has the paragraph's width; hit testing follows the linked glyphs.
func clickHandoffLink(t *testing.T, tt *ui.Tester, label string) {
	t.Helper()
	r, ok := tt.Find(label)
	if !ok {
		t.Fatalf("no link %q", label)
	}
	tt.ClickAt(r.X+4, r.Y+r.H/2)
}

func TestHandoffHiddenToolReferenceDoesNotReadMoreHistory(t *testing.T) {
	f := handoffFixture(t, false)
	target := store.Chat("fixture-scout-chat")
	target.Messages = append(target.Messages, &model.Message{ID: "hidden-tool", Author: model.BotAuthor("fixture-scout"), Body: model.Body{Kind: model.BodyTool, Tool: &model.ToolInvocation{Name: "read", Summary: "Read fixture", IsRunning: false}}, State: model.MessageState{Kind: model.StateComplete}, CreatedAt: time.Now()})
	target.HasMore = true
	f.m.openMessage(model.MessageLink{ChatID: target.ID, MessageID: "hidden-tool"})
	f.frames()
	if !f.tt.HasText(L("The linked message is unavailable.")) {
		t.Fatal("hidden tool not reported unavailable")
	}
	select {
	case <-f.transport.requests:
		t.Fatal("read history for a known hidden tool")
	default:
	}
}

func TestHandoffFailedPagingHasNativeRetryForTheTranscript(t *testing.T) {
	f := handoffFixture(t, true)
	f.open(t)
	request := f.take(t)
	request.reply <- handoffPageReply{err: errors.New("Unavailable fixture")}
	f.wait(t, "paging failure", f.m.hasSheet)
	settleTransitions(f.tt)
	if err := f.tt.Click(L("OK")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(f.tt)
	f.frames()
	if !f.tt.HasText(L("Could not load older messages.")) {
		t.Fatal("history failure has no retry state")
	}
	if err := f.tt.Click(L("Retry")); err != nil {
		t.Fatal(err)
	}
	f.frames()
	retry := f.take(t)
	retry.reply <- handoffPageReply{data: handoffWirePage([]*model.Message{handoffResponse(time.Now().Add(-5 * time.Minute))}, false)}
	f.wait(t, "retried transcript page", func() bool { return !store.IsLoadingOlder("fixture-scout-chat") })
	if store.OlderMessagesFailed("fixture-scout-chat") || f.tt.HasText(L("Could not load older messages.")) {
		t.Fatal("retry state did not clear")
	}
}
