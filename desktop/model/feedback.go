package model

import (
	"errors"
	"slices"
	"strings"
	"time"
)

// Workflow feedback, after the macOS app's Feedback.swift: what the user says about a bot's
// messages, the changes to its routines and skills the bot suggests from it, and the changes the
// user accepted. The bot's Runner keeps all of it and applies every decision; the app asks.

// FeedbackTarget is what a note, a suggestion, or a change is about: a routine's task, or a saved
// skill of the bot's or of a group it is in. The CLI names them; the app sends one back as it
// came. It is comparable, so a scope is a value.
type FeedbackTarget struct {
	Kind  string         `json:"kind"`
	ID    string         `json:"id,omitempty"`
	Scope *FeedbackScope `json:"scope,omitempty"`
}

// FeedbackScope is a skill's scope: a bot, or a group's project.
type FeedbackScope struct {
	Kind string `json:"kind"`
	ID   string `json:"id"`
}

// Same is whether two targets name the same routine or skill.
func (t FeedbackTarget) Same(o FeedbackTarget) bool {
	return t.Kind == o.Kind && t.ID == o.ID && (t.Scope == nil) == (o.Scope == nil) && (t.Scope == nil || *t.Scope == *o.Scope)
}

// The kinds of a note: how the user found a message, or a routine run that failed.
const (
	FeedbackAccepted       = "accepted"
	FeedbackRejected       = "rejected"
	FeedbackEdited         = "edited"
	FeedbackExplicit       = "explicit"
	FeedbackRoutineFailure = "routine_failure"
	FeedbackIgnoredAlert   = "ignored_alert"
)

// FeedbackNote is one thing the user said about a bot's work, or a routine run that failed, with
// the message it is about.
type FeedbackNote struct {
	ID        string
	Kind      string
	ChatID    string
	MessageID string
	// Text is the user's words, else the start of the message it is about.
	Text      string
	Target    *FeedbackTarget
	CreatedAt time.Time
}

// FeedbackSuggestion is a change the bot suggests, waiting for the user. A decision sends back
// DiffHash, so it is about the change the user saw.
type FeedbackSuggestion struct {
	ID          string
	Target      FeedbackTarget
	Explanation string
	Diff        string
	DiffHash    string
	Evidence    []string
	CreatedAt   time.Time
}

// FeedbackChange is a change the user accepted, or the undo of one. It can be undone while the
// routine or skill still reads as the change left it; CurrentHash guards that.
type FeedbackChange struct {
	ID          string
	Target      FeedbackTarget
	Diff        string
	IsUndo      bool
	CanUndo     bool
	CurrentHash string
	CreatedAt   time.Time
}

// FeedbackTargetName is a routine or skill a note can be about, with its name.
type FeedbackTargetName struct {
	Name   string         `json:"name"`
	Target FeedbackTarget `json:"target"`
}

// BotFeedback is a bot's workflow feedback as its Runner reports it: the newest notes, the
// suggestions waiting, and the newest changes, each newest first.
type BotFeedback struct {
	Notes       []FeedbackNote
	NoteCount   int
	Suggestions []FeedbackSuggestion
	Changes     []FeedbackChange
	// ReviewEvery is how often, in seconds, the bot looks for changes to suggest; nil when it
	// does only when asked.
	ReviewEvery *int64
	Targets     []FeedbackTargetName
}

// IsEmpty is nothing to show yet: the inspector leaves the section out.
func (f BotFeedback) IsEmpty() bool {
	return f.NoteCount == 0 && len(f.Suggestions) == 0 && len(f.Changes) == 0 && f.ReviewEvery == nil
}

// Note is the note with this id, when the list has it.
func (f BotFeedback) Note(id string) *FeedbackNote {
	for i := range f.Notes {
		if f.Notes[i].ID == id {
			return &f.Notes[i]
		}
	}
	return nil
}

// FeedbackTargetName is what the user calls a target: its routine's or skill's name.
func (s *Store) FeedbackTargetName(f BotFeedback, target FeedbackTarget) string {
	for _, known := range f.Targets {
		if known.Target.Same(target) {
			return known.Name
		}
	}
	if routine := s.Routine(target.ID); target.Kind == "routine_prompt" && routine != nil {
		return routine.Name
	}
	return firstNonEmpty(target.ID, L("Workflow"))
}

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}

// DiffLine is a line of a change: taken out, or put in.
type DiffLine struct {
	Removed bool
	Text    string
}

// DiffLines are the changed lines of the CLI's diff, without its file and hunk headers.
func DiffLines(diff string) []DiffLine {
	var lines []DiffLine
	for _, line := range strings.Split(diff, "\n") {
		switch {
		case strings.HasPrefix(line, "---"), strings.HasPrefix(line, "+++"), strings.HasPrefix(line, "@@"):
		case strings.HasPrefix(line, "-"):
			lines = append(lines, DiffLine{Removed: true, Text: line[1:]})
		case strings.HasPrefix(line, "+"):
			lines = append(lines, DiffLine{Text: line[1:]})
		}
	}
	return lines
}

// MARK: - Wire

type wireFeedbackProposal struct {
	ID          string         `json:"id"`
	Target      FeedbackTarget `json:"target"`
	Explanation string         `json:"explanation"`
	Diff        string         `json:"diff"`
	DiffHash    string         `json:"diff_hash"`
	Evidence    []string       `json:"evidence"`
	CreatedAt   float64        `json:"created_at"`
}

// WireFeedback is what `feedback.list` answers.
type WireFeedback struct {
	Feedback []struct {
		ID     string `json:"id"`
		Kind   string `json:"kind"`
		Origin struct {
			ChatID    string `json:"chat_id"`
			MessageID string `json:"message_id"`
		} `json:"origin"`
		Note      string          `json:"note"`
		Example   string          `json:"example"`
		Target    *FeedbackTarget `json:"target"`
		CreatedAt float64         `json:"created_at"`
	} `json:"feedback"`
	FeedbackCount int                    `json:"feedback_count"`
	Proposals     []wireFeedbackProposal `json:"proposals"`
	Revisions     []struct {
		ID          string         `json:"id"`
		Target      FeedbackTarget `json:"target"`
		CreatedAt   float64        `json:"created_at"`
		RollbackOf  *string        `json:"rollback_of"`
		Diff        string         `json:"diff"`
		CanRollback bool           `json:"can_rollback"`
		CurrentHash *string        `json:"current_hash"`
	} `json:"revisions"`
	Settings struct {
		ReviewEverySecs *int64 `json:"review_every_secs"`
	} `json:"settings"`
	Targets []FeedbackTargetName `json:"targets"`
}

// ToBotFeedback reads the list into the model, newest first.
func (w WireFeedback) ToBotFeedback() BotFeedback {
	f := BotFeedback{NoteCount: w.FeedbackCount, ReviewEvery: w.Settings.ReviewEverySecs, Targets: w.Targets}
	for _, note := range w.Feedback {
		text := note.Note
		if text == "" {
			text = note.Example
		}
		f.Notes = append(f.Notes, FeedbackNote{
			ID: note.ID, Kind: note.Kind, ChatID: note.Origin.ChatID, MessageID: note.Origin.MessageID,
			Text: strings.TrimSpace(text), Target: note.Target, CreatedAt: seconds(note.CreatedAt),
		})
	}
	for _, p := range slices.Backward(w.Proposals) {
		f.Suggestions = append(f.Suggestions, FeedbackSuggestion{
			ID: p.ID, Target: p.Target, Explanation: p.Explanation, Diff: p.Diff, DiffHash: p.DiffHash,
			Evidence: p.Evidence, CreatedAt: seconds(p.CreatedAt),
		})
	}
	for _, r := range w.Revisions {
		change := FeedbackChange{ID: r.ID, Target: r.Target, Diff: r.Diff, IsUndo: r.RollbackOf != nil, CanUndo: r.CanRollback, CreatedAt: seconds(r.CreatedAt)}
		if r.CurrentHash != nil {
			change.CurrentHash = *r.CurrentHash
		}
		f.Changes = append(f.Changes, change)
	}
	return f
}

// MARK: - Calls

// Feedback is a bot's notes, suggestions, and changes, from its Runner, through the relay when
// that is another Device.
func (s *Store) Feedback(botID string, done func(BotFeedback, error)) {
	if s.IsMock {
		f := s.demoFeedback(botID)
		s.post(func() { done(f, nil) })
		return
	}
	Async(s, func() (BotFeedback, error) {
		wire, err := call[WireFeedback](s, "feedback.list", map[string]any{"bot_id": botID})
		return wire.ToBotFeedback(), err
	}, done)
}

// FeedbackInput is what the user said about one of a bot's messages.
type FeedbackInput struct {
	Kind      string
	ChatID    string
	MessageID string
	Note      string
	// Before and After are the message and the user's version of it, for an edit.
	Before, After string
	Target        *FeedbackTarget
}

// RecordFeedback records what the user said about a message of the bot's.
func (s *Store) RecordFeedback(botID string, input FeedbackInput, done func(error)) {
	feedback := map[string]any{"kind": input.Kind, "origin": map[string]any{"chat_id": input.ChatID, "message_id": input.MessageID}, "note": input.Note}
	if input.Kind == FeedbackEdited {
		feedback["before"], feedback["after"] = input.Before, input.After
	}
	if input.Target != nil {
		feedback["target"] = *input.Target
	}
	s.changeFeedback(botID, "feedback.record", map[string]any{"feedback": feedback}, func(f *BotFeedback) {
		text := input.Note
		if text == "" {
			if chat := s.Chat(input.ChatID); chat != nil {
				for _, m := range chat.Messages {
					if m.ID == input.MessageID {
						text = MessageText(m)
					}
				}
			}
		}
		f.Notes = slices.Insert(f.Notes, 0, FeedbackNote{ID: NewMessageID(), Kind: input.Kind, ChatID: input.ChatID, MessageID: input.MessageID, Text: text, Target: input.Target, CreatedAt: time.Now()})
		f.NoteCount++
	}, done)
}

// DecideSuggestion accepts or rejects a suggestion, as the user saw it.
func (s *Store) DecideSuggestion(botID string, suggestion FeedbackSuggestion, accept bool, done func(error)) {
	method := "feedback.reject"
	if accept {
		method = "feedback.accept"
	}
	s.changeFeedback(botID, method, map[string]any{"id": suggestion.ID, "diff_hash": suggestion.DiffHash}, func(f *BotFeedback) {
		f.Suggestions = slices.DeleteFunc(f.Suggestions, func(each FeedbackSuggestion) bool { return each.ID == suggestion.ID })
		if accept {
			f.Changes = slices.Insert(f.Changes, 0, FeedbackChange{ID: NewMessageID(), Target: suggestion.Target, Diff: suggestion.Diff, CanUndo: true, CurrentHash: "mock", CreatedAt: time.Now()})
		}
	}, done)
}

// UndoChange puts back what a change replaced, while nothing has changed it since.
func (s *Store) UndoChange(botID string, change FeedbackChange, done func(error)) {
	s.changeFeedback(botID, "feedback.rollback", map[string]any{"id": change.ID, "expected_hash": change.CurrentHash}, func(f *BotFeedback) {
		var reversed []string
		for _, line := range DiffLines(change.Diff) {
			if line.Removed {
				reversed = append(reversed, "+"+line.Text)
			} else {
				reversed = append(reversed, "-"+line.Text)
			}
		}
		for i := range f.Changes {
			f.Changes[i].CanUndo = false
		}
		f.Changes = slices.Insert(f.Changes, 0, FeedbackChange{ID: NewMessageID(), Target: change.Target, Diff: strings.Join(reversed, "\n"), IsUndo: true, CanUndo: true, CurrentHash: "mock", CreatedAt: time.Now()})
	}, done)
}

// SetFeedbackReview is how often the bot looks through new feedback for changes to suggest; nil
// turns it off.
func (s *Store) SetFeedbackReview(botID string, every *int64, done func(error)) {
	var value any
	if every != nil {
		value = *every
	}
	s.changeFeedback(botID, "feedback.settings", map[string]any{"review_every_secs": value}, func(f *BotFeedback) {
		f.ReviewEvery = every
	}, done)
}

// ExcludeFeedback takes a note's words out of the bot's feedback, or every note from its chat,
// now and later.
func (s *Store) ExcludeFeedback(botID string, note FeedbackNote, wholeChat bool, done func(error)) {
	params := map[string]any{"id": note.ID}
	if wholeChat {
		params = map[string]any{"chat_id": note.ChatID}
	}
	s.changeFeedback(botID, "feedback.exclude", params, func(f *BotFeedback) {
		f.Notes = slices.DeleteFunc(f.Notes, func(each FeedbackNote) bool {
			return each.ID == note.ID || wholeChat && each.ChatID == note.ChatID
		})
		f.NoteCount = len(f.Notes)
	}, done)
}

// SuggestChanges has the bot look through its new feedback now, and answers how many changes it
// suggests.
func (s *Store) SuggestChanges(botID string, done func(int, error)) {
	if s.IsMock {
		s.later(time.Second, func() { done(0, nil) })
		return
	}
	Async(s, func() (int, error) {
		review, err := call[struct {
			Proposals []wireFeedbackProposal `json:"proposals"`
		}](s, "feedback.review", map[string]any{"bot_id": botID})
		return len(review.Proposals), err
	}, func(found int, err error) {
		if err == nil {
			s.emit(Event{Kind: EventFeedbackChanged, BotID: botID})
		}
		done(found, err)
	})
}

// changeFeedback is a call that changes a bot's feedback. The Runner announces its own bots'
// changes; a bot on another Device answers only the call, so the change is announced here too.
// `params` is built before the call and not touched after; the demo applies `mock` instead.
func (s *Store) changeFeedback(botID, method string, params map[string]any, mock func(*BotFeedback), done func(error)) {
	finish := func(err error) {
		if err == nil {
			s.emit(Event{Kind: EventFeedbackChanged, BotID: botID})
		}
		if done != nil {
			done(err)
		}
	}
	if s.IsMock {
		f := s.demoFeedback(botID)
		mock(&f)
		s.mockFeedback[botID] = f
		s.post(func() { finish(nil) })
		return
	}
	params["bot_id"] = botID
	Async(s, func() (struct{}, error) { return struct{}{}, s.request(method, params, nil) }, func(_ struct{}, err error) { finish(err) })
}

var (
	errChatGone    = errors.New("the chat is gone")
	errMessageGone = errors.New("the message is gone")
)

// LoadMessage loads older pages of a chat until `messageID` is in it, so a note can show the
// message it is about. The app beeps for one that is gone.
func (s *Store) LoadMessage(chatID, messageID string, done func(error)) {
	s.loadMessage(chatID, messageID, 0, done)
}

func (s *Store) loadMessage(chatID, messageID string, pages int, done func(error)) {
	chat := s.Chat(chatID)
	if chat == nil {
		done(errChatGone)
		return
	}
	if slices.ContainsFunc(chat.Messages, func(m *Message) bool { return m.ID == messageID }) {
		done(nil)
		return
	}
	if s.IsMock || !chat.HasMore || len(chat.Messages) == 0 || pages >= 100 {
		done(errMessageGone)
		return
	}
	first := chat.Messages[0].ID
	Async(s, func() (WireMessagePage, error) {
		return call[WireMessagePage](s, "chats.messages", map[string]any{"chat_id": chatID, "before": first})
	}, func(page WireMessagePage, err error) {
		if err != nil {
			done(err)
			return
		}
		current := s.Chat(chatID)
		if current == nil {
			done(errChatGone)
			return
		}
		if len(current.Messages) > 0 && current.Messages[0].ID == first {
			var older []*Message
			for _, wire := range page.Messages {
				message := ToMessage(wire)
				if !slices.ContainsFunc(current.Messages, func(m *Message) bool { return m.ID == message.ID }) {
					older = append(older, message)
				}
			}
			current.Messages = append(older, current.Messages...)
			current.HasMore = page.HasMore
			for _, m := range older {
				s.noteCommand(m, chatID)
			}
			s.emit(Event{Kind: EventOlderMessagesLoaded, ChatID: chatID})
			if len(older) == 0 {
				done(errMessageGone)
				return
			}
		}
		s.loadMessage(chatID, messageID, pages+1, done)
	})
}

// MARK: - Demo

// demoFeedback is the demo's feedback for a bot, changed in place by the same calls: Project
// Manager's on its briefs, with one change suggested and one applied.
func (s *Store) demoFeedback(botID string) BotFeedback {
	if f, ok := s.mockFeedback[botID]; ok {
		return f
	}
	chat := s.Chat("chat-nova")
	if botID != "bot-nova" || chat == nil {
		return BotFeedback{}
	}
	find := func(match func(*Message) bool) *Message {
		for _, m := range chat.Messages {
			if match(m) {
				return m
			}
		}
		return nil
	}
	marker := find(func(m *Message) bool { return m.Body.Text == "Routine · Morning brief" })
	report := find(func(m *Message) bool { return strings.HasPrefix(m.Body.Text, "**Today's focus") })
	handoff := find(func(m *Message) bool {
		return m.Author.BotID == "bot-nova" && strings.HasPrefix(m.Body.Text, "Writer has the brief.")
	})
	if marker == nil || report == nil || handoff == nil {
		return BotFeedback{}
	}
	brief := FeedbackTarget{Kind: "routine_prompt", ID: "rt-brief"}
	checklist := FeedbackTarget{Kind: "routine_prompt", ID: "rt-checklist"}
	week := int64(7 * 86_400)
	notes := []FeedbackNote{
		{ID: "fb-good", Kind: FeedbackAccepted, ChatID: "chat-nova", MessageID: handoff.ID, Text: handoff.Body.Text, CreatedAt: minutesAgo(30)},
		{ID: "fb-edit", Kind: FeedbackEdited, ChatID: "chat-nova", MessageID: report.ID, Text: "Lead with what needs my decision, then blockers.", Target: &brief, CreatedAt: minutesAgo(150)},
		{ID: "fb-short", Kind: FeedbackExplicit, ChatID: "chat-nova", MessageID: report.ID, Text: "Keep the brief to five bullets or fewer.", Target: &brief, CreatedAt: minutesAgo(60 * 26)},
		{ID: "fb-failed", Kind: FeedbackRoutineFailure, ChatID: "chat-nova", MessageID: marker.ID, Text: "The launch checklist was not in the workspace.", Target: &checklist, CreatedAt: minutesAgo(60 * 50)},
	}
	return BotFeedback{
		Notes: notes, NoteCount: len(notes), ReviewEvery: &week,
		Suggestions: []FeedbackSuggestion{{
			ID: "sg-brief", Target: brief,
			Explanation: "You moved decisions to the top of two briefs and asked for five bullets at most. Each brief would open with what needs you, then blockers.",
			Diff:        "--- current\n+++ proposed\n@@ -1,1 +1,1 @@\n-Read the recent messages in every chat you are in and the launch checklist in the workspace. Post a short brief: what changed, what needs a decision, and what the team will do first.\n+Read the recent messages in every chat you are in and the launch checklist in the workspace. Post a brief of five bullets at most: what needs my decision first, then blockers, then what changed.\n",
			DiffHash:    "mock", Evidence: []string{"fb-edit", "fb-short"}, CreatedAt: minutesAgo(20),
		}},
		Changes: []FeedbackChange{{
			ID: "ch-checklist", Target: checklist, CanUndo: true, CurrentHash: "mock", CreatedAt: minutesAgo(60 * 24 * 4),
			Diff: "--- current\n+++ proposed\n@@ -1,1 +1,1 @@\n-Review the launch checklist and the team's replies. Report anything new.\n+Review the launch checklist in the workspace and the latest team replies. Report new blockers or completed milestones, or PASS when nothing changed.\n",
		}},
		Targets: []FeedbackTargetName{
			{Name: "Morning brief", Target: brief}, {Name: "Launch checklist", Target: checklist},
			{Name: "Review requests", Target: FeedbackTarget{Kind: "routine_prompt", ID: "rt-reviews"}},
			{Name: "launch-post", Target: FeedbackTarget{Kind: "playbook", ID: "playbook-launch-post", Scope: &FeedbackScope{Kind: "project", ID: "chat-relay"}}},
		},
	}
}
