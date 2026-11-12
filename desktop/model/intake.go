package model

import (
	"crypto/rand"
	"fmt"
	"slices"
	"strings"
	"time"
)

// A channel's intake: what it takes on by itself, in the owner's words. Its Runner keeps it and
// lists it in the machine blob; a change replaces the whole of it there. What it decided about each
// message stays on that Runner, and any Device asks for it with `events.decisions`.

// IntakeMode is what an assessment that matches a rule does.
type IntakeMode string

const (
	// IntakeObserve records what the bot would have taken on.
	IntakeOff     IntakeMode = "off"
	IntakeObserve IntakeMode = "observe"
	// IntakeAuto takes it on.
	IntakeAuto IntakeMode = "auto"
)

// IntakePerson may ask the bot for work directly, by the service's own user id.
type IntakePerson struct {
	ID   string `json:"id"`
	Name string `json:"name,omitempty"`
}

// Title is the person's name, else their id.
func (p IntakePerson) Title() string {
	if p.Name != "" {
		return p.Name
	}
	return p.ID
}

// IntakeRule is a kind of work the bot takes on when nobody asked it directly.
type IntakeRule struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	Enabled     bool   `json:"enabled"`
	Instruction string `json:"instruction"`
}

// ChannelIntake is one channel's intake, as `events.set_intake` takes it. The CLI refuses a list
// sent as null, so a change goes through `clone`, which gives both an array.
type ChannelIntake struct {
	Mode   IntakeMode     `json:"mode"`
	People []IntakePerson `json:"people"`
	Rules  []IntakeRule   `json:"rules"`
}

func (i ChannelIntake) clone() ChannelIntake {
	return ChannelIntake{Mode: i.Mode, People: append([]IntakePerson{}, i.People...), Rules: append([]IntakeRule{}, i.Rules...)}
}

// WithMode is the intake in that mode.
func (i ChannelIntake) WithMode(mode IntakeMode) ChannelIntake {
	next := i.clone()
	next.Mode = mode
	return next
}

// Lists is whether someone can ask directly: only their id counts.
func (i ChannelIntake) Lists(senderID string) bool {
	return senderID != "" && slices.ContainsFunc(i.People, func(person IntakePerson) bool { return person.ID == senderID })
}

// WithPerson is the intake with this person listed, their name updated when they already are.
func (i ChannelIntake) WithPerson(id, name string) ChannelIntake {
	next := i.clone()
	if at := slices.IndexFunc(next.People, func(person IntakePerson) bool { return person.ID == id }); at >= 0 {
		next.People[at].Name = name
		return next
	}
	next.People = append(next.People, IntakePerson{ID: id, Name: name})
	return next
}

// WithoutPerson is the intake without this person.
func (i ChannelIntake) WithoutPerson(id string) ChannelIntake {
	next := i.clone()
	next.People = slices.DeleteFunc(next.People, func(person IntakePerson) bool { return person.ID == id })
	return next
}

// WithRule is the intake with this rule, in place of the one with its id or after the others.
func (i ChannelIntake) WithRule(rule IntakeRule) ChannelIntake {
	next := i.clone()
	if at := slices.IndexFunc(next.Rules, func(known IntakeRule) bool { return known.ID == rule.ID }); at >= 0 {
		next.Rules[at] = rule
		return next
	}
	next.Rules = append(next.Rules, rule)
	return next
}

// WithoutRule is the intake without this rule.
func (i ChannelIntake) WithoutRule(id string) ChannelIntake {
	next := i.clone()
	next.Rules = slices.DeleteFunc(next.Rules, func(rule IntakeRule) bool { return rule.ID == id })
	return next
}

// Rule finds a rule by its id.
func (i ChannelIntake) Rule(id string) *IntakeRule {
	for at := range i.Rules {
		if i.Rules[at].ID == id {
			return &i.Rules[at]
		}
	}
	return nil
}

// NewRuleID names a new rule in the form the CLI gives one, so the row keeps its id while the
// Runner's next list is on its way, and two rules can share a name.
func (i ChannelIntake) NewRuleID() string {
	for {
		var value [4]byte
		if _, err := rand.Read(value[:]); err != nil {
			panic(err)
		}
		if id := fmt.Sprintf("rule-%x", value); i.Rule(id) == nil {
			return id
		}
	}
}

// DecisionKind is what intake made of a message.
type DecisionKind string

const (
	DecisionAccepted    DecisionKind = "accepted"
	DecisionWouldAccept DecisionKind = "would_accept"
	DecisionAttached    DecisionKind = "attached"
	DecisionStopped     DecisionKind = "stopped"
	DecisionReplied     DecisionKind = "replied"
	DecisionIgnored     DecisionKind = "ignored"
	DecisionUnavailable DecisionKind = "unavailable"
)

// Title is the kind in the app's words.
func (kind DecisionKind) Title() string {
	switch kind {
	case DecisionAccepted:
		return L("Took on")
	case DecisionWouldAccept:
		return L("Would take on")
	case DecisionAttached:
		return L("Went to its task")
	case DecisionStopped:
		return L("Stopped")
	case DecisionReplied:
		return L("Answered")
	case DecisionIgnored:
		return L("Left alone")
	case DecisionUnavailable:
		return L("Couldn't assess")
	}
	return string(kind)
}

// IntakeDecision is one record of what intake decided, as `events.decisions` answers it. Its title,
// reason, and sender come from outside Lorca: the views show them as plain text.
type IntakeDecision struct {
	ID             string       `json:"id"`
	At             float64      `json:"at"`
	Kind           DecisionKind `json:"kind"`
	ConversationID string       `json:"conversation_id"`
	Sender         string       `json:"sender"`
	SenderID       string       `json:"sender_id"`
	Title          string       `json:"title"`
	Reason         string       `json:"reason"`
	TaskID         string       `json:"task_id"`
}

// Subject is what the decision is about: its title, else its reason.
func (d IntakeDecision) Subject() string {
	if d.Title != "" {
		return d.Title
	}
	return d.Reason
}

// SenderName is who wrote the message: their name, else their id.
func (d IntakeDecision) SenderName() string {
	if d.Sender != "" {
		return d.Sender
	}
	return d.SenderID
}

// Time is when it was decided.
func (d IntakeDecision) Time() time.Time { return seconds(d.At) }

// decisionsAsked is how many decisions a sheet asks for, more than it shows: left-alone messages
// are most of a busy group's, and they shouldn't push out the few it lists.
const decisionsAsked = 100

// DecisionsShown is the most decisions a sheet lists.
const DecisionsShown = 10

// ShownDecisions are the newest decisions a sheet lists, those left alone only with `ignored`.
func ShownDecisions(decisions []IntakeDecision, ignored bool) []IntakeDecision {
	var shown []IntakeDecision
	for _, decision := range decisions {
		if decision.Kind == DecisionIgnored && !ignored {
			continue
		}
		if shown = append(shown, decision); len(shown) == DecisionsShown {
			break
		}
	}
	return shown
}

// HasIgnored is whether any of the decisions left its message alone.
func HasIgnored(decisions []IntakeDecision) bool {
	return slices.ContainsFunc(decisions, func(decision IntakeDecision) bool { return decision.Kind == DecisionIgnored })
}

// IntakeDecisions asks the channel's Runner what its intake decided lately, newest first. It fails
// with why, such as the Runner being offline.
func (s *Store) IntakeDecisions(channelID string, done func([]IntakeDecision, error)) {
	runner := s.channelRunner(channelID)
	if s.IsMock {
		s.post(func() {
			if device := s.Device(runner); device != nil && device.Status == StatusOffline {
				done(nil, &RequestError{device.Name + " is offline."})
				return
			}
			done(mockDecisions(channelID), nil)
		})
		return
	}
	Async(s, func() ([]IntakeDecision, error) {
		var answer struct {
			Decisions []IntakeDecision `json:"decisions"`
		}
		err := s.request("events.decisions", map[string]any{"runner_id": runner, "id": channelID, "limit": decisionsAsked}, &answer)
		return answer.Decisions, err
	}, done)
}

// SetChannelIntake replaces the channel's whole intake on its Runner; nil turns it off. The change
// shows at once and goes back if the Runner refuses it, unless a later change has replaced it.
// Since each call carries the whole intake, the calls go out one at a time, in the order made.
func (s *Store) SetChannelIntake(id string, intake *ChannelIntake, done func(error)) {
	channel, runner := s.Channel(id), s.channelRunner(id)
	if channel == nil {
		return
	}
	before := channel.Intake
	var next *ChannelIntake
	if intake != nil {
		copy := intake.clone()
		next = &copy
	}
	channel.Intake = next
	s.emit(Event{Kind: EventRosterChanged})
	if s.IsMock {
		if done != nil {
			s.post(func() { done(nil) })
		}
		return
	}
	earlier := s.intakeWrites[id]
	written := make(chan struct{})
	s.intakeWrites[id] = written
	Async(s, func() (struct{}, error) {
		defer close(written)
		if earlier != nil {
			<-earlier
		}
		return struct{}{}, s.request("events.set_intake", map[string]any{"runner_id": runner, "id": id, "intake": next}, nil)
	}, func(_ struct{}, err error) {
		if s.intakeWrites[id] == written {
			delete(s.intakeWrites, id)
		}
		if err != nil {
			if current := s.Channel(id); current != nil && current.Intake == next {
				current.Intake = before
				s.emit(Event{Kind: EventRosterChanged})
			}
		}
		if done != nil {
			done(err)
		}
	})
}

// TaskMandate is why a channel's intake took a task on, and how far the work may go. The Runner sets
// it once; nothing here edits it. Its sender and scope come from outside Lorca.
type TaskMandate struct {
	ChannelID   string       `json:"channel_id"`
	ChannelName string       `json:"channel_name"`
	WorkChatID  string       `json:"work_chat_id"`
	ChatID      string       `json:"chat_id"`
	SenderID    string       `json:"sender_id"`
	Sender      string       `json:"sender"`
	Strength    string       `json:"strength"`
	Rules       []IntakeRule `json:"rules,omitempty"`
	Scope       string       `json:"scope"`
	Reason      string       `json:"reason"`
}

// From is who asked, and in which channel: "Maya · Community feedback".
func (m TaskMandate) From() string {
	sender := m.Sender
	if sender == "" {
		sender = m.SenderID
	}
	var parts []string
	for _, part := range []string{sender, m.ChannelName} {
		if part != "" {
			parts = append(parts, part)
		}
	}
	return strings.Join(parts, " · ")
}

// Why is what let the task be taken on: someone listed asked, else the rules it matched.
func (m TaskMandate) Why() string {
	if m.Strength == "explicit" {
		return L("Asked directly")
	}
	var names []string
	for _, rule := range m.Rules {
		names = append(names, rule.Name)
	}
	return strings.Join(names, Lc(", ", "list"))
}
