package model

import (
	"encoding/json"
	"testing"
)

// What `feedback.list` answers, as the CLI writes it.
const feedbackList = `{"feedback":[{"id":"feedback-2","kind":"edited","origin":{"chat_id":"chat-1","message_id":"m-2","routine_id":null},"note":"","example":"Lead with decisions.","target":{"kind":"routine_prompt","id":"rt-1"},"created_at":1760000100.0},
  {"id":"feedback-1","kind":"ignored_alert","origin":{"chat_id":"chat-1","message_id":"m-1"},"note":"","example":"","target":null,"created_at":1760000000.0}],
 "feedback_count":2,
 "proposals":[{"id":"proposal-1","target":{"kind":"routine_prompt","id":"rt-1"},"before":{"content":"Summarize.","hash":"h","revision":0},"after":"Summarize. Decisions first.","evidence":["feedback-2"],"origins":[],"explanation":"You moved decisions first.","diff":"--- current\n+++ proposed\n@@ -1,1 +1,1 @@\n-Summarize.\n+Summarize. Decisions first.\n","diff_hash":"d","state":"pending","created_at":1760000200.0}],
 "revisions":[{"id":"revision-1","target":{"kind":"playbook","id":"playbook-1","scope":{"kind":"bot","id":"bot-1"}},"created_at":1760000300.0,"rollback_of":null,"diff":"+Check the tests.\n","can_rollback":true,"current_hash":"c"}],
 "settings":{"review_every_secs":604800},
 "targets":[{"target":{"kind":"routine_prompt","id":"rt-1"},"name":"Morning brief"},{"target":{"kind":"playbook","id":"playbook-1","scope":{"kind":"bot","id":"bot-1"}},"name":"launch-brief"}]}`

func TestFeedbackListReadsIntoTheModel(t *testing.T) {
	var wire WireFeedback
	if err := json.Unmarshal([]byte(feedbackList), &wire); err != nil {
		t.Fatal(err)
	}
	f := wire.ToBotFeedback()
	if len(f.Notes) != 2 || f.Notes[0].Kind != FeedbackEdited || f.Notes[0].Text != "Lead with decisions." || f.Notes[0].ChatID != "chat-1" {
		t.Fatalf("notes %+v", f.Notes)
	}
	if len(f.Suggestions) != 1 || f.Suggestions[0].DiffHash != "d" || f.Suggestions[0].Evidence[0] != "feedback-2" {
		t.Fatalf("suggestions %+v", f.Suggestions)
	}
	if len(f.Changes) != 1 || !f.Changes[0].CanUndo || f.Changes[0].CurrentHash != "c" || f.Changes[0].IsUndo {
		t.Fatalf("changes %+v", f.Changes)
	}
	if f.ReviewEvery == nil || *f.ReviewEvery != 604800 || f.IsEmpty() || !(BotFeedback{}).IsEmpty() {
		t.Fatalf("settings %+v", f)
	}
	s := NewStore(nil, func(fn func()) { fn() }, false)
	if name := s.FeedbackTargetName(f, f.Changes[0].Target); name != "launch-brief" {
		t.Fatalf("name %q", name)
	}
	encoded, _ := json.Marshal(f.Changes[0].Target)
	if string(encoded) != `{"kind":"playbook","id":"playbook-1","scope":{"kind":"bot","id":"bot-1"}}` {
		t.Fatalf("a target goes back as it came: %s", encoded)
	}
}

func TestADiffShowsOnlyItsChangedLines(t *testing.T) {
	lines := DiffLines("--- current\n+++ proposed\n@@ -1,2 +1,2 @@\n-Old line\n+New line\n+\n")
	want := []DiffLine{{Removed: true, Text: "Old line"}, {Text: "New line"}, {Text: ""}}
	if len(lines) != len(want) {
		t.Fatalf("lines %+v", lines)
	}
	for i := range want {
		if lines[i] != want[i] {
			t.Fatalf("lines %+v", lines)
		}
	}
}
