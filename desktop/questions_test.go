package main

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A coding agent's question reads from the CLI's shape: its questions, their options, and the
// answers once given; it notifies as a question, never as a shell card.
func TestQuestionRequestDecodes(t *testing.T) {
	var wire model.WireMessage
	raw := `{"id":"msg-1","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},"state":{"kind":"complete"},"created_at":1,
		"body":{"kind":"permission","plugin_id":"coding_agent","plugin_name":"Claude Code","tool":"question","summary":"Which database?","decision":"pending",
		"reason":"Claude Code, the coding agent on “Storage”, asks you.","question":{"questions":[
			{"id":"Which database?","header":"Database","question":"Which database?","options":[{"label":"Postgres","description":"The usual"},{"label":"SQLite"}],"multi_select":false},
			{"id":"Anything else?","header":"","question":"Anything else?","options":[]}]}}}`
	if err := json.Unmarshal([]byte(raw), &wire); err != nil {
		t.Fatal(err)
	}
	message := model.ToMessage(wire)
	request := message.Body.Request
	if !request.IsQuestion() || request.IsShell() || request.IsSecret() {
		t.Fatalf("not read as a question: %+v", request)
	}
	questions := request.Question.Questions
	if len(questions) != 2 || questions[0].Header != "Database" || questions[0].Text != "Which database?" || questions[0].MultiSelect ||
		questions[0].Options[0] != (model.QuestionOption{Label: "Postgres", Description: "The usual"}) || questions[0].Options[1].Description != "" ||
		len(questions[1].Options) != 0 || request.Question.Answers != nil {
		t.Fatalf("questions %+v", request.Question)
	}
	if note := notificationFor(message); note == nil || note.kind != notifyPermission || note.body != "Asks: Which database?" {
		t.Errorf("notification %+v", note)
	}
	if got := request.VerbPhrase(); got != "has a question from Claude Code" {
		t.Errorf("verb phrase %q", got)
	}
	request.Decision = model.DecisionDenied
	if got := questionCaption(request); got != "Skipped · Which database?" {
		t.Errorf("skipped caption %q", got)
	}
	if notificationFor(message) != nil {
		t.Error("a skipped question notifies")
	}

	answered := `{"id":"msg-1","chat_id":"chat-1","author":{"kind":"bot","bot_id":"bot-1"},"state":{"kind":"complete"},"created_at":1,
		"body":{"kind":"permission","plugin_id":"coding_agent","plugin_name":"Codex","tool":"question","summary":"Which?","decision":"allowed",
		"question":{"questions":[{"id":"q","header":"","question":"Which?","options":[]}],"answers":{"q":"SQLite"}}}}`
	if err := json.Unmarshal([]byte(answered), &wire); err != nil {
		t.Fatal(err)
	}
	if got := model.ToMessage(wire).Body.Request.Question.Answers["q"]; got != "SQLite" {
		t.Errorf("answer %q", got)
	}
}

// askQuestions puts a coding agent's two questions at the end of the Developer's chat: one of
// several options, and one in the user's own words.
func askQuestions() {
	store.Append(&model.Message{
		ID:     "msg-question",
		Author: model.BotAuthor("bot-patch"),
		Body: model.Body{Kind: model.BodyPermission, Request: &model.PermissionRequest{
			PluginID: "coding_agent", PluginName: "Claude Code", Tool: "question", Summary: "Which pages? (+1 more)", Decision: model.DecisionPending,
			Reason: "Claude Code, the coding agent on “Fix the broken docs link on the download page”, asks you.",
			Question: &model.QuestionAsk{Questions: []model.Question{
				{ID: "pages", Header: "Pages", Text: "Which pages?", MultiSelect: true, Options: []model.QuestionOption{
					{Label: "Download", Description: "The download page"}, {Label: "Docs"}, {Label: "Blog"},
				}},
				{ID: "note", Text: "Anything to add to the commit message?"},
			}},
		}},
		State:     model.MessageState{Kind: model.StateComplete},
		CreatedAt: time.Now(),
	}, "chat-patch")
}

// The demo's question offers its options; Submit waits for an answer, and the card then shows it.
func TestQuestionCardAnswers(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	for _, text := range []string{"Developer has a question from Codex", "Placement", "Where should the dark mode toggle go?", "After Appearance", "One click from every page"} {
		if !tt.HasText(text) {
			t.Fatalf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "question-card")
	if err := tt.Click("Submit"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if tt.HasText("Answered: In the header") || !tt.HasText("After Appearance") {
		t.Fatal("Submit went with nothing picked")
	}
	if err := tt.Click("In the header"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.Click("Submit"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText("Answered: In the header") || tt.HasText("After Appearance") {
		t.Fatalf("the card did not show its answer: %q", tt.Texts())
	}
}

// Skip skips the question, and the card says so.
func TestQuestionCardSkips(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	if err := tt.Click("Skip"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText("Skipped · Where should the dark mode toggle go?") || tt.HasText("After Appearance") {
		t.Fatalf("the question was not skipped: %q", tt.Texts())
	}
}

// Several options checked and the user's own words go as one answer each.
func TestQuestionCardChecksSeveral(t *testing.T) {
	m := demoWindow(t)
	// The demo's own question is answered, so this card's buttons are the only ones.
	for _, message := range store.Chat("chat-patch").Messages {
		if message.Body.Kind == model.BodyPermission && message.Body.Request.IsQuestion() {
			store.AnswerPermission("chat-patch", message.ID, "deny")
		}
	}
	askQuestions()
	m.selectChat("chat-patch")
	tt := ui.NewTester(m.frame(m.view), 1180, 1100)
	settle(tt)
	for _, text := range []string{"Developer has a question from Claude Code", "Which pages?", "The download page", "Anything to add to the commit message?"} {
		if !tt.HasText(text) {
			t.Fatalf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "question-card-several")
	for _, label := range []string{"Download", "Blog"} {
		if err := tt.Click(label); err != nil {
			t.Fatal(err)
		}
		settle(tt)
	}
	if err := tt.Click("Type your answer"); err != nil {
		t.Fatal(err)
	}
	tt.Type("Mention the test")
	settle(tt)
	if err := tt.Click("Submit"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	for _, text := range []string{"Answered: Download, Blog", "Answered: Mention the test"} {
		if !tt.HasText(text) {
			t.Fatalf("no %q in %q", text, tt.Texts())
		}
	}
}

// A bot's own sub agent is a Subagent, with Lorca's mark.
func TestSubagentNameAndMark(t *testing.T) {
	if got := model.CodingAgentName("lorca"); got != "Subagent" {
		t.Errorf("name %q", got)
	}
	if agentMark("lorca") == nil {
		t.Error("no mark for a sub agent")
	}
}
