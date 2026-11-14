package model

import (
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"testing"
	"time"
)

func TestWorkChatsReadFromTheWireAndStayOutOfConversations(t *testing.T) {
	var snapshot WireSnapshot
	if err := json.Unmarshal([]byte(`{"has_identity":true,"chats":[
		{"id":"talk","kind":"dm","bot_ids":["bot"],"channel":{"channel_id":"ch","service":"telegram","account_id":"a","chat_id":"-1"}},
		{"id":"work","kind":"dm","bot_ids":["bot"],"is_hidden":true,"channel":{"channel_id":"ch","service":"telegram","account_id":"a","chat_id":"-1","task_id":"task-1"}},
		{"id":"plain","kind":"dm","bot_ids":["bot"]}
	]}`), &snapshot); err != nil {
		t.Fatal(err)
	}
	s := NewStore(nil, func(fn func()) { fn() }, false)
	s.apply(snapshot)
	for id, want := range map[string]bool{"talk": false, "work": true, "plain": false} {
		if chat := s.Chat(id); chat == nil || chat.IsWorkChat() != want {
			t.Errorf("%s: %+v, want a work chat %v", id, chat, want)
		}
	}
	conversations := s.Conversations("ch")
	if len(conversations) != 1 || conversations[0].ID != "talk" {
		t.Errorf("conversations: %+v", conversations)
	}
	s.Chats = []*Chat{s.Chat("work"), s.Chat("plain")}
	if first := s.FirstChat(); first == nil || first.ID != "plain" {
		t.Errorf("first chat: %+v", first)
	}
	s.Chats = []*Chat{s.Chats[0]}
	if first := s.FirstChat(); first != nil {
		t.Errorf("first chat among work chats: %+v", first)
	}
}

func TestChannelReadsItsIntake(t *testing.T) {
	var wire WireChannel
	// The CLI leaves empty lists out.
	if err := json.Unmarshal([]byte(`{"id":"ch","state":"listening","intake":{"mode":"auto","rules":[{"id":"r","name":"Bugs","enabled":false,"instruction":"File them"}]}}`), &wire); err != nil {
		t.Fatal(err)
	}
	intake := ToChannel(wire).Intake
	if intake == nil || intake.Mode != IntakeAuto || len(intake.People) != 0 || len(intake.Rules) != 1 || intake.Rules[0].Enabled {
		t.Fatalf("intake: %+v", intake)
	}
	if err := json.Unmarshal([]byte(`{"id":"ch","state":"listening"}`), &wire); err != nil {
		t.Fatal(err)
	}
	wire.Intake = nil
	if ToChannel(wire).Intake != nil {
		t.Fatal("an intake from nowhere")
	}
}

func TestIntakeEditsLeaveTheOriginal(t *testing.T) {
	intake := ChannelIntake{Mode: IntakeObserve, People: []IntakePerson{{ID: "1", Name: "Ann"}}, Rules: []IntakeRule{{ID: "a", Name: "A"}, {ID: "b", Name: "B"}}}
	renamed := intake.WithRule(IntakeRule{ID: "a", Name: "Z"})
	if intake.Rules[0].Name != "A" || renamed.Rules[0].Name != "Z" || len(renamed.Rules) != 2 {
		t.Errorf("WithRule: %+v from %+v", renamed, intake)
	}
	if added := intake.WithRule(IntakeRule{ID: "c"}); len(added.Rules) != 3 || added.Rules[2].ID != "c" || len(intake.Rules) != 2 {
		t.Errorf("WithRule added: %+v", added)
	}
	if without := intake.WithoutRule("a"); len(without.Rules) != 1 || without.Rules[0].ID != "b" || intake.Rules[0].ID != "a" {
		t.Errorf("WithoutRule: %+v from %+v", without, intake)
	}
	if listed := intake.WithPerson("1", "Anna"); len(listed.People) != 1 || listed.People[0].Name != "Anna" || intake.People[0].Name != "Ann" {
		t.Errorf("WithPerson again: %+v", listed)
	}
	if without := intake.WithoutPerson("1"); len(without.People) != 0 || len(intake.People) != 1 {
		t.Errorf("WithoutPerson: %+v", without)
	}
	if !intake.Lists("1") || intake.Lists("") || intake.Lists("2") {
		t.Error("Lists")
	}
	if id := intake.NewRuleID(); !strings.HasPrefix(id, "rule-") || intake.Rule(id) != nil {
		t.Errorf("NewRuleID: %q", id)
	}
	if (IntakePerson{ID: "1"}).Title() != "1" || (IntakePerson{ID: "1", Name: "Ann"}).Title() != "Ann" {
		t.Error("Title")
	}
}

// intakeStore has a channel on a Runner, and runs what the store posts when asked.
func intakeStore(request func(string, any) (json.RawMessage, error)) (*Store, func()) {
	posted := make(chan func(), 16)
	s := NewStore(taskTransport{request: request}, func(fn func()) { posted <- fn }, false)
	s.Devices = []*Device{{ID: "runner", Channels: []Channel{{ID: "ch", State: ChannelListening}}}}
	next := func() {
		select {
		case fn := <-posted:
			fn()
		case <-time.After(5 * time.Second):
			panic("nothing posted")
		}
	}
	return s, next
}

func TestSetChannelIntakeSendsTheWholeIntakeInOrder(t *testing.T) {
	calls := make(chan string, 4)
	release := make(chan struct{})
	s, next := intakeStore(func(method string, params any) (json.RawMessage, error) {
		data, _ := json.Marshal(params)
		calls <- method + " " + string(data)
		<-release
		return json.RawMessage("{}"), nil
	})
	first := ChannelIntake{Mode: IntakeObserve}
	answers := 0
	done := func(err error) {
		if err != nil {
			t.Error(err)
		}
		answers++
	}
	s.SetChannelIntake("ch", &first, done)
	if intake := s.Channel("ch").Intake; intake == nil || intake.Mode != IntakeObserve {
		t.Fatalf("not shown at once: %+v", intake)
	}
	// The CLI refuses a list sent as null.
	if got, want := <-calls, `events.set_intake {"id":"ch","intake":{"mode":"observe","people":[],"rules":[]},"runner_id":"runner"}`; got != want {
		t.Errorf("sent %s, want %s", got, want)
	}
	second := first.WithRule(IntakeRule{ID: "r", Name: "Bugs", Enabled: true, Instruction: "File them"}).WithPerson("1", "").WithMode(IntakeAuto)
	s.SetChannelIntake("ch", &second, done)
	s.SetChannelIntake("ch", nil, done)
	select {
	case call := <-calls:
		t.Fatalf("%s went out before the first was answered", call)
	case <-time.After(20 * time.Millisecond):
	}
	release <- struct{}{}
	if got, want := <-calls, `events.set_intake {"id":"ch","intake":{"mode":"auto","people":[{"id":"1"}],"rules":[{"id":"r","name":"Bugs","enabled":true,"instruction":"File them"}]},"runner_id":"runner"}`; got != want {
		t.Errorf("sent %s, want %s", got, want)
	}
	release <- struct{}{}
	if got, want := <-calls, `events.set_intake {"id":"ch","intake":null,"runner_id":"runner"}`; got != want {
		t.Errorf("sent %s, want %s", got, want)
	}
	release <- struct{}{}
	for range 3 {
		next()
	}
	if answers != 3 || s.Channel("ch").Intake != nil || len(s.intakeWrites) != 0 {
		t.Errorf("answers %d, intake %+v, writes %d", answers, s.Channel("ch").Intake, len(s.intakeWrites))
	}
}

func TestSetChannelIntakeGoesBackWhenRefused(t *testing.T) {
	refused := errors.New("Give each rule a name of at most 60 characters.")
	s, next := intakeStore(func(string, any) (json.RawMessage, error) { return nil, refused })
	before := &ChannelIntake{Mode: IntakeObserve}
	s.Channel("ch").Intake = before
	var answer error
	s.SetChannelIntake("ch", &ChannelIntake{Mode: IntakeAuto}, func(err error) { answer = err })
	next()
	if answer == nil || answer.Error() != refused.Error() {
		t.Errorf("answer: %v", answer)
	}
	if s.Channel("ch").Intake != before {
		t.Errorf("intake: %+v", s.Channel("ch").Intake)
	}
}

func TestIntakeDecisionsFromTheChannelsRunner(t *testing.T) {
	var sent string
	s, next := intakeStore(func(method string, params any) (json.RawMessage, error) {
		data, _ := json.Marshal(params)
		sent = method + " " + string(data)
		return json.RawMessage(`{"decisions":[
			{"id":"d1","channel_id":"ch","at":1700000000,"kind":"accepted","conversation_id":"talk","message_ids":["m1"],"sender":"Maya","sender_id":"7","strength":"explicit","addressed":true,"title":"File it","scope":"Only the report","reason":"Asked","rules":[{"rule_id":"r","matched":true,"reason":"Fits"}],"evidence":["file it"],"task_id":"task-1","waiting":true,"copies":["c1"],"policy_version":"v"},
			{"id":"d2","channel_id":"ch","at":1699999000,"kind":"ignored","conversation_id":"talk","message_ids":["m2"],"sender":"","sender_id":"8","strength":null,"reason":"Chatter"}
		]}`), nil
	})
	var decisions []IntakeDecision
	s.IntakeDecisions("ch", func(answer []IntakeDecision, err error) {
		if err != nil {
			t.Error(err)
		}
		decisions = answer
	})
	next()
	if want := `events.decisions {"id":"ch","limit":100,"runner_id":"runner"}`; sent != want {
		t.Errorf("sent %s, want %s", sent, want)
	}
	if len(decisions) != 2 {
		t.Fatalf("decisions: %+v", decisions)
	}
	first, second := decisions[0], decisions[1]
	if first.Kind != DecisionAccepted || first.TaskID != "task-1" || first.Subject() != "File it" || first.SenderName() != "Maya" || first.Time().Unix() != 1700000000 {
		t.Errorf("first: %+v", first)
	}
	if second.TaskID != "" || second.Subject() != "Chatter" || second.SenderName() != "8" {
		t.Errorf("second: %+v", second)
	}
}

func TestIntakeDecisionsSayWhyTheyFailed(t *testing.T) {
	s, next := intakeStore(func(string, any) (json.RawMessage, error) { return nil, errors.New("Studio is offline.") })
	var failure error
	s.IntakeDecisions("ch", func(_ []IntakeDecision, err error) { failure = err })
	next()
	if failure == nil || ErrorText(failure) != "Studio is offline." {
		t.Errorf("failure: %v", failure)
	}
}

func TestShownDecisionsLeaveOutIgnoredUnlessAsked(t *testing.T) {
	var decisions []IntakeDecision
	for i := range 30 {
		kind := DecisionReplied
		if i%2 == 0 {
			kind = DecisionIgnored
		}
		decisions = append(decisions, IntakeDecision{ID: fmt.Sprint(i), Kind: kind})
	}
	shown := ShownDecisions(decisions, false)
	if len(shown) != DecisionsShown || shown[0].ID != "1" || slices.ContainsFunc(shown, func(d IntakeDecision) bool { return d.Kind == DecisionIgnored }) {
		t.Errorf("without ignored: %+v", shown)
	}
	if shown := ShownDecisions(decisions, true); len(shown) != DecisionsShown || shown[0].ID != "0" {
		t.Errorf("with ignored: %+v", shown)
	}
	if !HasIgnored(decisions) || HasIgnored(shown) {
		t.Error("HasIgnored")
	}
}

func TestDecisionKindsReadInWords(t *testing.T) {
	for kind, want := range map[DecisionKind]string{
		DecisionAccepted: "Took on", DecisionWouldAccept: "Would take on", DecisionAttached: "Went to its task", DecisionStopped: "Stopped",
		DecisionReplied: "Answered", DecisionIgnored: "Left alone", DecisionUnavailable: "Couldn't assess",
	} {
		if got := kind.Title(); got != want {
			t.Errorf("%s: %q, want %q", kind, got, want)
		}
	}
}

func TestTaskKeepsItsMandateAndFollows(t *testing.T) {
	var task DurableTask
	if err := json.Unmarshal([]byte(`{"id":"t","state":"queued","follows":["talk"],"mandate":{"channel_id":"ch","channel_name":"Community","channel_task":"T","service":"telegram","work_chat_id":"work","chat_id":"talk","message_id":"m","external_id":"e","sender_id":"7","sender":"Maya","strength":"inferred","rules":[{"id":"r","name":"Bugs","enabled":true,"instruction":"File them"},{"id":"s","name":"Crashes","enabled":true,"instruction":"Fix them"}],"scope":"Only the report","policy_version":"v","reason":"Fits","at":1}}`), &task); err != nil {
		t.Fatal(err)
	}
	clone := task.Clone()
	mandate := clone.Mandate
	if mandate == nil || mandate.WorkChatID != "work" || !slices.Equal(clone.Follows, []string{"talk"}) || mandate.Scope != "Only the report" {
		t.Fatalf("clone: %+v, %+v", clone, mandate)
	}
	if got := mandate.From(); got != "Maya · Community" {
		t.Errorf("from: %q", got)
	}
	if got := mandate.Why(); got != "Bugs, Crashes" {
		t.Errorf("why: %q", got)
	}
	direct := TaskMandate{SenderID: "7", Strength: "explicit"}
	if direct.From() != "7" || direct.Why() != "Asked directly" {
		t.Errorf("direct: %q, %q", direct.From(), direct.Why())
	}
}

func TestNothingToDoIsNoCancellation(t *testing.T) {
	reason := NothingToDoReason
	quiet := DurableTask{State: TaskCancelled, Reason: &reason}
	if !quiet.NothingToDo() || quiet.StateTitle() != "Nothing to do" || quiet.StateSymbol() == TaskCancelled.Symbol() {
		t.Errorf("quiet: %q %q", quiet.StateTitle(), quiet.StateSymbol())
	}
	stopped := CancelledByUser
	cancelled := DurableTask{State: TaskCancelled, Reason: &stopped}
	if cancelled.NothingToDo() || cancelled.StateTitle() != TaskCancelled.Title() {
		t.Errorf("cancelled: %q", cancelled.StateTitle())
	}
	if blocked := (DurableTask{State: TaskBlocked, Reason: &reason}); blocked.NothingToDo() {
		t.Error("a blocked task with those words")
	}
}

func TestAgo(t *testing.T) {
	now := time.Now()
	for elapsed, want := range map[time.Duration]string{
		10 * time.Second: "just now", 5 * time.Minute: "5m ago", 3 * time.Hour: "3h ago", 50 * time.Hour: "2d ago",
	} {
		if got := Ago(now.Add(-elapsed)); got != want {
			t.Errorf("%s: %q, want %q", elapsed, got, want)
		}
	}
}
