package model

import (
	"bytes"
	"encoding/json"
	"errors"
	"slices"
	"strings"
)

// ReviewPayload retains the exact JSON keys and number tokens received from the CLI.
// The original object also preserves fields a newer Runner may add to a known payload.
type ReviewPayload struct {
	Kind       string          `json:"kind"`
	Text       string          `json:"text,omitempty"`
	PluginID   string          `json:"plugin_id,omitempty"`
	ServerName string          `json:"server_name,omitempty"`
	Tool       string          `json:"tool,omitempty"`
	Arguments  json.RawMessage `json:"arguments,omitempty"`
	raw        json.RawMessage
}

func (p *ReviewPayload) UnmarshalJSON(data []byte) error {
	type fields ReviewPayload
	var next fields
	if err := json.Unmarshal(data, &next); err != nil {
		return err
	}
	*p = ReviewPayload(next)
	p.raw = bytes.Clone(data)
	return nil
}

func (p ReviewPayload) MarshalJSON() ([]byte, error) {
	if len(p.raw) > 0 {
		return bytes.Clone(p.raw), nil
	}
	type fields ReviewPayload
	return json.Marshal(fields(p))
}

func (p ReviewPayload) Supported() bool {
	return p.Kind == "draft" || p.Kind == "shell" || p.Kind == "plugin"
}

func prettyReviewJSON(raw json.RawMessage) string {
	var out bytes.Buffer
	if json.Indent(&out, raw, "", "  ") == nil {
		return out.String()
	}
	return string(raw)
}

func (p ReviewPayload) EditorText() string {
	if p.Kind == "draft" {
		return p.Text
	}
	if !p.Supported() {
		return prettyReviewJSON(p.raw)
	}
	return prettyReviewJSON(p.Arguments)
}

// EditedJSON replaces only the editable value, without a float64 or key-name conversion.
func (p ReviewPayload) EditedJSON(text string) (json.RawMessage, error) {
	raw, err := p.MarshalJSON()
	if err != nil {
		return nil, err
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(raw, &object); err != nil || object == nil {
		return nil, errors.New(L("The proposed call needs a JSON object of arguments."))
	}
	switch p.Kind {
	case "draft":
		object["text"], err = json.Marshal(text)
	case "shell", "plugin":
		var arguments map[string]json.RawMessage
		if json.Unmarshal([]byte(text), &arguments) != nil || arguments == nil {
			return nil, errors.New(L("The proposed call needs a JSON object of arguments."))
		}
		object["arguments"] = json.RawMessage(text)
	default:
		return nil, errors.New(L("This app cannot edit this review payload. Update Lorca to review it."))
	}
	if err != nil {
		return nil, err
	}
	return json.Marshal(object)
}

type ReviewOrigin struct {
	ChatID    string `json:"chat_id"`
	MessageID string `json:"message_id,omitempty"`
	RoutineID string `json:"routine_id,omitempty"`
	TaskID    string `json:"task_id,omitempty"`
}

type ReviewTarget struct {
	Account  string `json:"account"`
	Resource string `json:"resource"`
}

type ReviewPreconditions struct {
	Workdir string `json:"workdir"`
	Files   []struct {
		Path string  `json:"path"`
		Hash *string `json:"hash"`
	} `json:"files"`
}

type ReviewOutcome struct {
	Summary   string          `json:"summary"`
	MessageID string          `json:"message_id"`
	Result    json.RawMessage `json:"result"`
}

// ReviewItem is a projection of the CLI's encrypted record; execution stays on its Runner.
type ReviewItem struct {
	ID            string              `json:"id"`
	RunnerID      string              `json:"runner_id"`
	BotID         string              `json:"bot_id"`
	Origin        ReviewOrigin        `json:"origin"`
	Target        ReviewTarget        `json:"target"`
	Rationale     string              `json:"rationale"`
	Payload       ReviewPayload       `json:"payload"`
	Version       uint64              `json:"version"`
	Revision      uint64              `json:"revision"`
	Preconditions ReviewPreconditions `json:"preconditions"`
	State         string              `json:"state"`
	Outcome       *ReviewOutcome      `json:"outcome"`
}

func (r ReviewItem) Editable() bool { return r.State == "pending" || r.State == "approved" }

func (r ReviewItem) StateText() string {
	switch r.State {
	case "pending":
		return L("Needs review")
	case "approved":
		return L("Approved")
	case "executing":
		return L("Executing")
	case "succeeded":
		return L("Completed")
	case "failed":
		return L("Failed")
	case "rejected":
		return L("Rejected")
	case "cancelled":
		return L("Cancelled")
	case "uncertain":
		return L("Check the outcome")
	}
	return r.State
}

func (r ReviewItem) Clone() ReviewItem {
	r.Payload.Arguments = bytes.Clone(r.Payload.Arguments)
	r.Payload.raw = bytes.Clone(r.Payload.raw)
	r.Preconditions.Files = slices.Clone(r.Preconditions.Files)
	for i, file := range r.Preconditions.Files {
		if file.Hash != nil {
			hash := *file.Hash
			r.Preconditions.Files[i].Hash = &hash
		}
	}
	if r.Outcome != nil {
		outcome := *r.Outcome
		outcome.Result = bytes.Clone(outcome.Result)
		r.Outcome = &outcome
	}
	return r
}

func (s *Store) Review(id string) *ReviewItem {
	for _, item := range s.Reviews {
		if item.ID == id {
			return item
		}
	}
	return nil
}

func (s *Store) ReviewsFor(chatID string) []ReviewItem {
	var out []ReviewItem
	for _, item := range s.Reviews {
		if item.Origin.ChatID == chatID {
			out = append(out, item.Clone())
		}
	}
	slices.SortFunc(out, func(a, b ReviewItem) int {
		if a.Editable() != b.Editable() {
			if a.Editable() {
				return -1
			}
			return 1
		}
		return strings.Compare(a.ID, b.ID)
	})
	return out
}

func (s *Store) upsertReview(item ReviewItem) ReviewItem {
	if current := s.Review(item.ID); current != nil && current.Revision > item.Revision {
		return current.Clone()
	}
	copy := item.Clone()
	for i, current := range s.Reviews {
		if current.ID == item.ID {
			s.Reviews[i] = &copy
			s.emit(Event{Kind: EventReviewsChanged, ChatID: item.Origin.ChatID})
			return copy.Clone()
		}
	}
	s.Reviews = append(slices.Clone(s.Reviews), &copy)
	s.emit(Event{Kind: EventReviewsChanged, ChatID: item.Origin.ChatID})
	return copy.Clone()
}

// RefreshReview reads through the local CLI. The reply cannot replace a newer synced revision.
func (s *Store) RefreshReview(id string, done func(ReviewItem, error)) {
	if s.IsMock {
		s.post(func() {
			if item := s.Review(id); item != nil {
				done(item.Clone(), nil)
			} else {
				done(ReviewItem{}, ErrNotFound)
			}
		})
		return
	}
	identity := s.IdentityID
	Async(s, func() (ReviewItem, error) { return call[ReviewItem](s, "reviews.get", map[string]any{"id": id}) }, func(item ReviewItem, err error) {
		if s.IdentityID != identity {
			done(ReviewItem{}, errors.New(L("The account changed while this review was loading.")))
			return
		}
		if err == nil && (item.ID != id || item.Version == 0) {
			err = errors.New(L("The Runner returned a different review item."))
		}
		if err == nil {
			item = s.upsertReview(item)
		}
		done(item, err)
	})
}

type ReviewEdits struct {
	Payload   json.RawMessage `json:"payload"`
	Target    ReviewTarget    `json:"target"`
	Rationale string          `json:"rationale"`
}

// ChangeReview names the version actually displayed. No optimistic decision grants authority.
func (s *Store) ChangeReview(item ReviewItem, action string, edits *ReviewEdits, done func(ReviewItem, error)) {
	if !slices.Contains([]string{"edit", "approve", "reject", "cancel"}, action) {
		s.post(func() { done(ReviewItem{}, errors.New(L("Unknown review action"))) })
		return
	}
	params := map[string]any{"id": item.ID, "expected_version": item.Version}
	if action == "edit" {
		if edits == nil {
			s.post(func() { done(ReviewItem{}, errors.New(L("A review edit needs its saved payload."))) })
			return
		}
		params["payload"], params["target"], params["rationale"] = json.RawMessage(bytes.Clone(edits.Payload)), edits.Target, edits.Rationale
	}
	if s.IsMock {
		s.post(func() {
			current := s.Review(item.ID)
			if current == nil {
				done(ReviewItem{}, ErrNotFound)
				return
			}
			if current.Version != item.Version {
				done(ReviewItem{}, errors.New(L("This review changed. Reload it before deciding.")))
				return
			}
			if !current.Editable() {
				done(ReviewItem{}, errors.New(L("This review has already started or ended.")))
				return
			}
			next := current.Clone()
			switch action {
			case "edit":
				if err := json.Unmarshal(params["payload"].(json.RawMessage), &next.Payload); err != nil {
					done(ReviewItem{}, err)
					return
				}
				next.Target, next.Rationale = params["target"].(ReviewTarget), params["rationale"].(string)
				next.Version++
				next.State = "pending"
				next.Outcome = nil
			case "approve":
				next.State = "approved"
			case "reject":
				next.State = "rejected"
				next.Outcome = &ReviewOutcome{Summary: L("Rejected")}
			case "cancel":
				next.State = "cancelled"
				next.Outcome = &ReviewOutcome{Summary: L("Cancelled")}
			}
			next.Revision++
			done(s.upsertReview(next), nil)
		})
		return
	}
	identity := s.IdentityID
	Async(s, func() (ReviewItem, error) { return call[ReviewItem](s, "reviews."+action, params) }, func(next ReviewItem, err error) {
		if s.IdentityID != identity {
			done(ReviewItem{}, errors.New(L("The account changed while this review was loading.")))
			return
		}
		if err == nil && (next.ID != item.ID || next.Version == 0) {
			err = errors.New(L("The Runner returned a different review item."))
		}
		if err == nil {
			next = s.upsertReview(next)
		}
		done(next, err)
	})
}
