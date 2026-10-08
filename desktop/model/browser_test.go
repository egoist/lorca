package model

import (
	"encoding/json"
	"errors"
	"testing"
	"time"
)

type browserCall struct {
	method string
	params map[string]any
	answer chan browserAnswer
}
type browserAnswer struct {
	value any
	err   error
}
type browserTransport struct{ calls chan browserCall }

func (b browserTransport) Reconnect() {}
func (b browserTransport) Request(method string, params any) (json.RawMessage, error) {
	var copied map[string]any
	bytes, _ := json.Marshal(params)
	_ = json.Unmarshal(bytes, &copied)
	call := browserCall{method: method, params: copied, answer: make(chan browserAnswer, 1)}
	b.calls <- call
	answer := <-call.answer
	bytes, err := json.Marshal(answer.value)
	if answer.err != nil {
		err = answer.err
	}
	return bytes, err
}
func nextBrowserCall(t *testing.T, calls <-chan browserCall) browserCall {
	t.Helper()
	select {
	case call := <-calls:
		return call
	case <-time.After(time.Second):
		t.Fatal("browser request never arrived")
		return browserCall{}
	}
}
func runBrowserReply(t *testing.T, posted <-chan func()) {
	t.Helper()
	select {
	case fn := <-posted:
		fn()
	case <-time.After(time.Second):
		t.Fatal("browser reply was not posted to the main queue")
	}
}

func TestBrowserWireKeepsOwnershipRevisionAndExplicitCapabilities(t *testing.T) {
	list := decodeJSON[BrowserSessionList](t, `{"sessions":[{"id":"s1","bot_id":"b1","runner_id":"r1","account":"Work","profile":"Research","state":"human","selected":true,"revision":9007199254740993}],"capabilities":{"visible_open":false,"local_input":false,"pause":true,"resume":true,"stop":true,"screenshot":true,"remote_input":false,"remote_live_view":false,"native_input":false}}`)
	expect(t, list.Sessions[0].Revision, uint64(9007199254740993))
	expect(t, []string{list.Sessions[0].BotID, list.Sessions[0].RunnerID, list.Sessions[0].Account, list.Sessions[0].Profile}, []string{"b1", "r1", "Work", "Research"})
	expect(t, []bool{list.Capabilities.VisibleOpen, list.Capabilities.LocalInput, list.Capabilities.Stop, list.Capabilities.RemoteInput, list.Capabilities.NativeInput}, []bool{false, false, true, false, false})
	missing := decodeJSON[BrowserSessionList](t, `{"sessions":[]}`)
	expect(t, missing.Capabilities, BrowserCapabilities{})
}

func TestBrowserMethodsUseBoundWireArgumentsAndOrderedReplies(t *testing.T) {
	calls, posted := make(chan browserCall, 8), make(chan func(), 8)
	s := NewStore(browserTransport{calls}, func(fn func()) { posted <- fn }, false)
	session := BrowserSession{ID: "s1", BotID: "b1", RunnerID: "r1", State: BrowserHuman, Revision: 19}
	for _, op := range []BrowserOperation{BrowserCreate, BrowserOpen, BrowserTakeover, BrowserResume, BrowserStop, BrowserScreenshot} {
		called := false
		s.BrowserAction(op, "b1", "chat1", session, "Work", "Personal", func(reply BrowserReply, err error) {
			if err != nil {
				t.Error(err)
			}
			called = true
		})
		call := nextBrowserCall(t, calls)
		expect(t, call.method, string(op))
		expect(t, call.params["bot_id"], "b1")
		if op == BrowserCreate {
			expect(t, call.params["account"], "Work")
			expect(t, call.params["profile"], "Personal")
			if _, sent := call.params["session_id"]; sent {
				t.Fatal("create forwarded an unrelated session")
			}
		} else {
			expect(t, call.params["session_id"], "s1")
		}
		if op == BrowserResume {
			expect[any](t, call.params["revision"], float64(19))
		}
		if op == BrowserScreenshot {
			expect(t, call.params["chat_id"], "chat1")
		}
		if _, sent := call.params["runner_id"]; sent {
			t.Fatal("UI overrides the bot's assigned Runner")
		}
		if called {
			t.Fatal("worker invoked UI callback before the main queue")
		}
		call.answer <- browserAnswer{value: BrowserReply{Session: &session, MessageID: "evidence-1"}}
		runBrowserReply(t, posted)
		if !called {
			t.Fatal("posted callback never ran")
		}
	}
}

func TestBrowserListAndFailuresReachMainQueue(t *testing.T) {
	calls, posted := make(chan browserCall, 2), make(chan func(), 2)
	s := NewStore(browserTransport{calls}, func(fn func()) { posted <- fn }, false)
	var answerErr error
	s.BrowserSessions("b1", func(_ BrowserSessionList, err error) { answerErr = err })
	call := nextBrowserCall(t, calls)
	expect(t, call.method, "browser.sessions")
	expect(t, call.params["bot_id"], "b1")
	call.answer <- browserAnswer{err: errors.New("Runner is offline")}
	runBrowserReply(t, posted)
	if answerErr == nil || ErrorText(answerErr) != "Runner is offline" {
		t.Fatal(answerErr)
	}
}

func TestBrowserForeignSessionNeverReachesTransport(t *testing.T) {
	calls, posted := make(chan browserCall, 1), make(chan func(), 1)
	s := NewStore(browserTransport{calls}, func(fn func()) { posted <- fn }, false)
	var answerErr error
	s.BrowserAction(BrowserResume, "owner", "chat1", BrowserSession{ID: "foreign", BotID: "another"}, "", "", func(_ BrowserReply, err error) { answerErr = err })
	runBrowserReply(t, posted)
	if answerErr == nil {
		t.Fatal("foreign profile allowed")
	}
	select {
	case <-calls:
		t.Fatal("foreign profile forwarded to transport")
	default:
	}
}
