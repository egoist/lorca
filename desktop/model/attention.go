package model

import "slices"

// Attention is a decrypted projection from the local CLI. Tasks, reviews, resolutions,
// encryption and coordinator execution retain their Rust owners.
type AttentionRevision struct {
	Counter  uint64 `json:"counter"`
	DeviceID string `json:"device_id"`
}
type AttentionSource struct {
	ChatID    string `json:"chat_id"`
	TaskID    string `json:"task_id,omitempty"`
	MessageID string `json:"message_id,omitempty"`
	ReviewID  string `json:"review_id,omitempty"`
}
type AttentionItem struct {
	ID               string            `json:"id"`
	Key              string            `json:"key"`
	Category         string            `json:"category"`
	Title            string            `json:"title"`
	Summary          string            `json:"summary"`
	NextAction       string            `json:"next_action"`
	CoordinatorBotID string            `json:"coordinator_bot_id"`
	Sources          []AttentionSource `json:"sources"`
	Reporters        []string          `json:"reporters"`
	Urgent           bool              `json:"urgent"`
	Resolved         bool              `json:"resolved"`
	Revision         AttentionRevision `json:"revision"`
	UpdatedAt        float64           `json:"updated_at"`
}
type AttentionBrief struct {
	CoordinatorBotID string            `json:"coordinator_bot_id"`
	ChatID           string            `json:"chat_id"`
	Decisions        []string          `json:"decisions"`
	Changes          []string          `json:"changes"`
	NextAction       string            `json:"next_action"`
	ItemIDs          []string          `json:"item_ids"`
	MessageID        string            `json:"message_id"`
	Revision         AttentionRevision `json:"revision"`
	UpdatedAt        float64           `json:"updated_at"`
}
type AttentionPreferences struct {
	Summaries               bool              `json:"summaries"`
	UrgentDirect            bool              `json:"urgent_direct"`
	DefaultCoordinatorBotID *string           `json:"default_coordinator_bot_id"`
	Coordinators            map[string]string `json:"coordinators"`
}
type AttentionView struct {
	Items       []AttentionItem      `json:"items"`
	Briefs      []AttentionBrief     `json:"briefs"`
	Preferences AttentionPreferences `json:"preferences"`
}

func DefaultAttention() AttentionView {
	return AttentionView{Preferences: AttentionPreferences{Summaries: true, UrgentDirect: true, Coordinators: map[string]string{}}}
}
func (s *Store) applyAttention(view AttentionView) {
	s.Attention = view
	s.attentionGeneration++
	s.emit(Event{Kind: EventAttentionChanged})
}

// RefreshAttention ignores a read that became stale while a newer projection or account
// arrived. Both the CLI event and reply use the store's ordered main-thread queue.
func (s *Store) RefreshAttention(done func(error)) {
	if s.IsMock {
		if done != nil {
			done(nil)
		}
		return
	}
	generation, identity := s.attentionGeneration, s.IdentityID
	Async(s, func() (AttentionView, error) { return call[AttentionView](s, "attention.list", nil) }, func(view AttentionView, err error) {
		if err == nil && identity == s.IdentityID && generation == s.attentionGeneration {
			s.applyAttention(view)
		}
		if done != nil {
			done(err)
		}
	})
}

// ResolveAttention changes only the projection; it does not approve a review or complete
// a task. The expected revision binds the user's action to the item they inspected.
func (s *Store) ResolveAttention(id string, revision AttentionRevision, done func(error)) {
	if s.IsMock {
		s.Attention.Items = slices.DeleteFunc(slices.Clone(s.Attention.Items), func(item AttentionItem) bool { return item.ID == id })
		s.applyAttention(s.Attention)
		if done != nil {
			done(nil)
		}
		return
	}
	params := map[string]any{"id": id, "expected_revision": revision}
	Async(s, func() (struct{}, error) { return struct{}{}, s.request("attention.resolve", params, nil) }, func(_ struct{}, err error) {
		if done != nil {
			done(err)
		}
	})
}

// SetAttentionPreferences writes the three visible preferences. Per-chat coordinator
// bindings remain with the CLI; this form does not replace that map.
func (s *Store) SetAttentionPreferences(summaries, urgent bool, coordinatorID string, done func(error)) {
	var coordinator *string
	if coordinatorID != "" {
		coordinator = &coordinatorID
	}
	if s.IsMock {
		s.Attention.Preferences.Summaries = summaries
		s.Attention.Preferences.UrgentDirect = urgent
		s.Attention.Preferences.DefaultCoordinatorBotID = coordinator
		s.applyAttention(s.Attention)
		if done != nil {
			done(nil)
		}
		return
	}
	params := map[string]any{"summaries": summaries, "urgent_direct": urgent, "default_coordinator_bot_id": coordinator}
	Async(s, func() (struct{}, error) { return struct{}{}, s.request("attention.preferences", params, nil) }, func(_ struct{}, err error) {
		if done != nil {
			done(err)
		}
	})
}

// NotificationTag is carried on structured text messages by the CLI. Quiet messages never
// alert; summary and urgent alerts follow independent synced account preferences.
type NotificationTag string

const (
	NotificationSummary NotificationTag = "summary"
	NotificationUrgent  NotificationTag = "urgent"
	NotificationQuiet   NotificationTag = "quiet"
)
