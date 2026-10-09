package model

import (
	"encoding/json"
	"testing"
	"time"
)

// What the store sends the CLI for a change, through a transport that records each call.

type recordedCall struct {
	method string
	params string
}

type recordingTransport struct{ calls chan recordedCall }

func (r recordingTransport) Request(method string, params any) (json.RawMessage, error) {
	data, _ := json.Marshal(params)
	r.calls <- recordedCall{method, string(data)}
	return json.RawMessage("{}"), nil
}

func (recordingTransport) Reconnect() {}

func (r recordingTransport) next(t *testing.T) recordedCall {
	t.Helper()
	select {
	case call := <-r.calls:
		return call
	case <-time.After(2 * time.Second):
		t.Fatal("no call")
		return recordedCall{}
	}
}

// A review model is a patch of one provider's, and the provider and the switch leave the models
// out, so no change resets another's.
func TestReviewModelsAreSentAsAPatch(t *testing.T) {
	transport := recordingTransport{calls: make(chan recordedCall, 8)}
	s := NewStore(transport, func(fn func()) { fn() }, false)
	s.AutoReview = AutoReview{IsEnabled: true, Provider: "anthropic", Models: map[ProviderKind]string{"anthropic": "claude-opus-5"}}

	s.SetReviewModel("jev-1.13", "custom:typesafe")
	expect(t, transport.next(t), recordedCall{"auto_review.set", `{"models":{"custom:typesafe":"jev-1.13"}}`})
	expect(t, s.AutoReview.Models, map[ProviderKind]string{"anthropic": "claude-opus-5", "custom:typesafe": "jev-1.13"})

	s.SetReviewModel("", "anthropic")
	expect(t, transport.next(t), recordedCall{"auto_review.set", `{"models":{"anthropic":null}}`})
	expect(t, s.AutoReview.Models, map[ProviderKind]string{"custom:typesafe": "jev-1.13"})

	s.SetReviewProvider("")
	expect(t, transport.next(t), recordedCall{"auto_review.set", `{"provider":null}`})
	s.SetReviewProvider("custom:typesafe")
	expect(t, transport.next(t), recordedCall{"auto_review.set", `{"provider":"custom:typesafe"}`})

	next := s.AutoReview
	next.IsEnabled = false
	s.SetAutoReview(next)
	expect(t, transport.next(t), recordedCall{"auto_review.set", `{"is_enabled":false,"rules":[]}`})
	expect(t, []any{s.AutoReview.Provider, s.AutoReview.Models["custom:typesafe"]}, []any{"custom:typesafe", "jev-1.13"})
}
