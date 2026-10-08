package model

import (
	"encoding/json"
	"errors"
	"slices"
)

// Workflow feedback is a Runner-owned CLI projection. The desktop keeps only the view it
// requested; decisions never write workflow text, permissions, budgets or schedules locally.
type FeedbackOrigin struct {
	ChatID    string `json:"chat_id"`
	MessageID string `json:"message_id"`
	RoutineID string `json:"routine_id,omitempty"`
	ReviewID  string `json:"review_id,omitempty"`
	TaskID    string `json:"task_id,omitempty"`
}
type FeedbackScope struct {
	Kind string `json:"kind"`
	ID   string `json:"id"`
}
type FeedbackTarget struct {
	Kind     string         `json:"kind"`
	ID       string         `json:"id,omitempty"`
	PluginID string         `json:"plugin_id,omitempty"`
	Name     string         `json:"name,omitempty"`
	Scope    *FeedbackScope `json:"scope,omitempty"`
}
type FeedbackSnapshot struct {
	Content  json.RawMessage `json:"content"`
	Hash     string          `json:"hash"`
	Revision uint64          `json:"revision"`
}
type FeedbackExample struct {
	ID        string          `json:"id"`
	Kind      string          `json:"kind"`
	Origin    FeedbackOrigin  `json:"origin"`
	Link      string          `json:"link"`
	Note      string          `json:"note"`
	Example   string          `json:"example"`
	Before    *string         `json:"before"`
	After     *string         `json:"after"`
	Target    *FeedbackTarget `json:"target"`
	Excluded  bool            `json:"excluded"`
	CreatedAt float64         `json:"created_at"`
}
type FeedbackProposal struct {
	ID          string           `json:"id"`
	Target      FeedbackTarget   `json:"target"`
	Before      FeedbackSnapshot `json:"before"`
	After       json.RawMessage  `json:"after"`
	Evidence    []string         `json:"evidence"`
	Origins     []FeedbackOrigin `json:"origins"`
	Explanation string           `json:"explanation"`
	Diff        string           `json:"diff"`
	DiffHash    string           `json:"diff_hash"`
	State       string           `json:"state"`
}
type FeedbackRevision struct {
	ID           string           `json:"id"`
	Version      uint64           `json:"version"`
	ProposalID   string           `json:"proposal_id"`
	Target       FeedbackTarget   `json:"target"`
	Before       FeedbackSnapshot `json:"before"`
	After        json.RawMessage  `json:"after"`
	State        string           `json:"state"`
	CurrentHash  string           `json:"current_hash"`
	CanRollback  bool             `json:"can_rollback"`
	RollbackDiff string           `json:"rollback_diff"`
	RollbackOf   *string          `json:"rollback_of"`
}
type FeedbackSettings struct {
	ReviewEverySecs *int64           `json:"review_every_secs"`
	ExcludedChats   []string         `json:"excluded_chats"`
	ExcludedTargets []FeedbackTarget `json:"excluded_targets"`
}
type FeedbackTargetChoice struct {
	Name   string         `json:"name"`
	Target FeedbackTarget `json:"target"`
}
type WorkflowFeedback struct {
	Feedback  []FeedbackExample      `json:"feedback"`
	Proposals []FeedbackProposal     `json:"proposals"`
	Revisions []FeedbackRevision     `json:"revisions"`
	Settings  FeedbackSettings       `json:"settings"`
	Targets   []FeedbackTargetChoice `json:"targets"`
}
type FeedbackInput struct {
	Kind     string          `json:"kind"`
	Origin   FeedbackOrigin  `json:"origin"`
	Note     string          `json:"note"`
	Before   *string         `json:"before,omitempty"`
	After    *string         `json:"after,omitempty"`
	Target   *FeedbackTarget `json:"target,omitempty"`
	Excluded bool            `json:"excluded"`
	EventID  string          `json:"event_id,omitempty"`
}
type FeedbackExclusion struct {
	ChatID string          `json:"chat_id,omitempty"`
	ID     string          `json:"id,omitempty"`
	Target *FeedbackTarget `json:"target,omitempty"`
}

func (s *Store) WorkflowFeedback(botID string, done func(WorkflowFeedback, error)) {
	if s.IsMock {
		s.post(func() { done(WorkflowFeedback{}, nil) })
		return
	}
	Async(s, func() (WorkflowFeedback, error) {
		return call[WorkflowFeedback](s, "feedback.list", map[string]any{"bot_id": botID})
	}, done)
}

// feedbackRequest encodes on the main thread before launching work. Input pointers, maps and
// slices from a sheet are never read by a worker after its next build pass.
func (s *Store) feedbackRequest(method string, params any, done func(error)) {
	if s.IsMock {
		s.post(func() { done(errors.New(L("Feedback requires a connected Runner."))) })
		return
	}
	encoded, err := json.Marshal(params)
	if err != nil {
		s.post(func() { done(err) })
		return
	}
	Async(s, func() (struct{}, error) { return struct{}{}, s.request(method, json.RawMessage(encoded), nil) }, func(_ struct{}, err error) { done(err) })
}
func (s *Store) RecordFeedback(botID string, input FeedbackInput, done func(error)) {
	s.feedbackRequest("feedback.record", struct {
		BotID    string        `json:"bot_id"`
		Feedback FeedbackInput `json:"feedback"`
	}{botID, input}, done)
}
func (s *Store) DecideFeedback(botID string, proposal FeedbackProposal, accept bool, done func(error)) {
	if proposal.State != "pending" || proposal.ID == "" || proposal.DiffHash == "" || proposal.Diff == "" {
		s.post(func() { done(errors.New(L("Reload the proposal before deciding."))) })
		return
	}
	method := "feedback.reject"
	if accept {
		method = "feedback.accept"
	}
	s.feedbackRequest(method, map[string]any{"bot_id": botID, "id": proposal.ID, "diff_hash": proposal.DiffHash}, done)
}
func (s *Store) RollbackFeedback(botID string, revision FeedbackRevision, done func(error)) {
	if revision.State != "applied" || !revision.CanRollback || revision.ID == "" || revision.CurrentHash == "" || revision.RollbackDiff == "" {
		s.post(func() { done(errors.New(L("Reload the revision before rolling it back."))) })
		return
	}
	s.feedbackRequest("feedback.rollback", map[string]any{"bot_id": botID, "id": revision.ID, "expected_hash": revision.CurrentHash}, done)
}
func (s *Store) SetFeedbackReview(botID string, interval *int64, done func(error)) {
	if interval != nil && (*interval < 86400 || *interval > 30*86400) {
		s.post(func() { done(errors.New(L("Choose a review interval from one day to thirty days."))) })
		return
	}
	s.feedbackRequest("feedback.settings", map[string]any{"bot_id": botID, "review_every_secs": interval}, done)
}
func (s *Store) ExcludeFeedback(botID string, exclusion FeedbackExclusion, done func(error)) {
	if exclusion.ChatID == "" && exclusion.ID == "" && exclusion.Target == nil {
		s.post(func() { done(errors.New(L("Choose material to exclude."))) })
		return
	}
	s.feedbackRequest("feedback.exclude", struct {
		BotID string `json:"bot_id"`
		FeedbackExclusion
	}{botID, exclusion}, done)
}
func (s *Store) ReviewFeedback(botID string, done func(error)) {
	s.feedbackRequest("feedback.review", map[string]any{"bot_id": botID}, done)
}

// LoadFeedbackOrigin pages through source history in order. It merges on the main thread,
// handles another page loading meanwhile, and reports deletion/offline errors to the sheet.
func (s *Store) LoadFeedbackOrigin(origin FeedbackOrigin, done func(error)) {
	s.loadFeedbackOrigin(origin, 0, done)
}
func (s *Store) loadFeedbackOrigin(origin FeedbackOrigin, pages int, done func(error)) {
	chat := s.Chat(origin.ChatID)
	if chat == nil {
		done(errors.New(L("The originating chat is no longer available.")))
		return
	}
	if slices.ContainsFunc(chat.Messages, func(m *Message) bool { return m.ID == origin.MessageID }) {
		done(nil)
		return
	}
	if s.IsMock || !chat.HasMore || len(chat.Messages) == 0 || pages >= 100 {
		done(errors.New(L("The originating message could not be loaded.")))
		return
	}
	first := chat.Messages[0].ID
	Async(s, func() (WireMessagePage, error) {
		return call[WireMessagePage](s, "chats.messages", map[string]any{"chat_id": origin.ChatID, "before": first})
	}, func(page WireMessagePage, err error) {
		if err != nil {
			done(err)
			return
		}
		current := s.Chat(origin.ChatID)
		if current == nil {
			done(errors.New(L("The originating chat is no longer available.")))
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
				s.noteCommand(m, origin.ChatID)
			}
			s.emit(Event{Kind: EventOlderMessagesLoaded, ChatID: origin.ChatID})
			if len(older) == 0 {
				done(errors.New(L("The originating message could not be loaded.")))
				return
			}
		}
		s.loadFeedbackOrigin(origin, pages+1, done)
	})
}
