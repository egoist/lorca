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
	Category         string            `json:"category"`
	Title            string            `json:"title"`
	Summary          string            `json:"summary"`
	NextAction       string            `json:"next_action"`
	CoordinatorBotID string            `json:"coordinator_bot_id"`
	Sources          []AttentionSource `json:"sources"`
	Urgent           bool              `json:"urgent"`
	Revision         AttentionRevision `json:"revision"`
}
type AttentionBrief struct {
	CoordinatorBotID string   `json:"coordinator_bot_id"`
	ChatID           string   `json:"chat_id"`
	Decisions        []string `json:"decisions"`
	Changes          []string `json:"changes"`
	NextAction       string   `json:"next_action"`
	UpdatedAt        float64  `json:"updated_at"`
}
type AttentionPreferences struct {
	Summaries               bool    `json:"summaries"`
	UrgentDirect            bool    `json:"urgent_direct"`
	DefaultCoordinatorBotID *string `json:"default_coordinator_bot_id"`
}
type AttentionView struct {
	Items       []AttentionItem      `json:"items"`
	Briefs      []AttentionBrief     `json:"briefs"`
	Preferences AttentionPreferences `json:"preferences"`
}

func DefaultAttention() AttentionView {
	return AttentionView{Preferences: AttentionPreferences{Summaries: true, UrgentDirect: true}}
}
func (s *Store) applyAttention(view AttentionView) {
	s.Attention = view
	s.emit(Event{Kind: EventAttentionChanged})
}

// ResolveAttention takes an item off the list. A coordinator resolves its items itself; this is
// the user saying it is done, and leaves the task or review it links to as it is. The revision
// binds the action to the item the user saw.
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

// SetAttentionPreference writes one of the account's preferences: "summaries", "urgent_direct",
// or "default_coordinator_bot_id" (a bot's id, or nil for each chat's own coordinator).
func (s *Store) SetAttentionPreference(key string, value any, done func(error)) {
	if s.IsMock {
		switch key {
		case "summaries":
			s.Attention.Preferences.Summaries, _ = value.(bool)
		case "urgent_direct":
			s.Attention.Preferences.UrgentDirect, _ = value.(bool)
		default:
			s.Attention.Preferences.DefaultCoordinatorBotID = nil
			if id, ok := value.(string); ok {
				s.Attention.Preferences.DefaultCoordinatorBotID = &id
			}
		}
		s.applyAttention(s.Attention)
		if done != nil {
			done(nil)
		}
		return
	}
	params := map[string]any{key: value}
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
