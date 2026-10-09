package model

import (
	"bytes"
	"cmp"
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

// EditorText is what the sheet edits: a draft's text, a shell command, or a call's arguments as
// JSON.
func (p ReviewPayload) EditorText() string {
	switch p.Kind {
	case "draft":
		return p.Text
	case "shell":
		var arguments struct {
			Command string `json:"command"`
		}
		_ = json.Unmarshal(p.Arguments, &arguments)
		return arguments.Command
	case "plugin":
		return prettyReviewJSON(p.Arguments)
	}
	return prettyReviewJSON(p.raw)
}

// EditedJSON is the payload with `text` in place of what EditorText showed, without a float64 or
// key-name conversion: a command's other arguments and a call's server and tool stay as they were.
func (p ReviewPayload) EditedJSON(text string) (json.RawMessage, error) {
	raw, err := p.MarshalJSON()
	if err != nil {
		return nil, err
	}
	var object map[string]json.RawMessage
	if err := json.Unmarshal(raw, &object); err != nil || object == nil {
		return nil, errors.New(L("The arguments need to be a JSON object."))
	}
	switch p.Kind {
	case "draft":
		object["text"], err = json.Marshal(text)
	case "shell":
		var arguments map[string]json.RawMessage
		if json.Unmarshal(p.Arguments, &arguments) != nil || arguments == nil {
			arguments = map[string]json.RawMessage{}
		}
		if arguments["command"], err = json.Marshal(text); err == nil {
			object["arguments"], err = json.Marshal(arguments)
		}
	case "plugin":
		var arguments map[string]json.RawMessage
		if json.Unmarshal([]byte(text), &arguments) != nil || arguments == nil {
			return nil, errors.New(L("The arguments need to be a JSON object."))
		}
		object["arguments"] = json.RawMessage(text)
	default:
		return nil, errors.New(L("Update Lorca to review this."))
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
	CreatedAt     float64             `json:"created_at"`
}

func (r ReviewItem) Pending() bool { return r.State == "pending" }

// Open is waiting for the user, or approved and about to run.
func (r ReviewItem) Open() bool {
	return r.Pending() || r.State == "approved" || r.State == "executing"
}

// StateText is how it ended or where it stands, in a word or two; empty while it waits for the
// user.
func (r ReviewItem) StateText() string {
	switch r.State {
	case "approved", "executing":
		return L("Running…")
	case "succeeded":
		if r.Payload.Kind == "draft" {
			return L("Accepted")
		}
		return L("Done")
	case "failed":
		return L("Failed")
	case "rejected":
		return L("Rejected")
	case "cancelled":
		return L("Cancelled")
	case "uncertain":
		return L("Didn't finish")
	}
	return ""
}

// Headline is the command, or what a draft is for; the inspector names a call by its plugin
// instead.
func (r ReviewItem) Headline() string {
	if r.Payload.Kind == "shell" {
		return FirstLine(r.Payload.EditorText())
	}
	return r.Target.Resource
}

// Output is what the call printed or returned, or the accepted draft.
func (r ReviewItem) Output() string {
	if r.Outcome == nil {
		return ""
	}
	var result struct {
		Text string `json:"text"`
	}
	_ = json.Unmarshal(r.Outcome.Result, &result)
	return result.Text
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

// OpenReviewsFor is what a chat's bots left for the user that waits or runs, oldest first. How
// each ended stays in the chat.
func (s *Store) OpenReviewsFor(chatID string) []ReviewItem {
	var out []ReviewItem
	for _, item := range s.Reviews {
		if item.Origin.ChatID == chatID && item.Open() {
			out = append(out, item.Clone())
		}
	}
	slices.SortFunc(out, func(a, b ReviewItem) int {
		if a.CreatedAt != b.CreatedAt {
			return cmp.Compare(a.CreatedAt, b.CreatedAt)
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

// ApproveReview approves the version the user saw. An edit made in the sheet (`payload`, nil
// without one) is saved first as the next version, and that is the one approved: what runs is
// what the editor showed.
func (s *Store) ApproveReview(item ReviewItem, payload json.RawMessage, done func(ReviewItem, error)) {
	if payload == nil {
		s.changeReview(item, "approve", nil, done)
		return
	}
	s.changeReview(item, "edit", payload, func(edited ReviewItem, err error) {
		if err != nil {
			done(edited, err)
			return
		}
		s.changeReview(edited, "approve", nil, done)
	})
}

func (s *Store) RejectReview(item ReviewItem, done func(ReviewItem, error)) {
	s.changeReview(item, "reject", nil, done)
}

// changeReview names the version the sheet displayed, so one changed on another Device meanwhile
// is refused rather than decided blind. Replies come in order on the main thread; one that arrives
// after the account changed, or that names another item, is an error.
func (s *Store) changeReview(item ReviewItem, action string, payload json.RawMessage, done func(ReviewItem, error)) {
	params := map[string]any{"id": item.ID, "expected_version": item.Version}
	if action == "edit" {
		params["payload"] = json.RawMessage(bytes.Clone(payload))
	}
	if s.IsMock {
		s.post(func() {
			current := s.Review(item.ID)
			if current == nil {
				done(ReviewItem{}, ErrNotFound)
				return
			}
			if current.Version != item.Version {
				done(ReviewItem{}, errors.New("This changed on another Device. Review it again."))
				return
			}
			if !current.Pending() {
				done(ReviewItem{}, errors.New("This already ran or was decided."))
				return
			}
			next := current.Clone()
			switch action {
			case "edit":
				if err := json.Unmarshal(params["payload"].(json.RawMessage), &next.Payload); err != nil {
					done(ReviewItem{}, err)
					return
				}
				next.Version++
				next.Outcome = nil
			case "approve":
				// The demo's Runner runs it at once.
				next.State = "succeeded"
				next.Outcome = &ReviewOutcome{Summary: "Done"}
				if next.Payload.Kind == "draft" {
					result, _ := json.Marshal(map[string]string{"text": next.Payload.Text})
					next.Outcome = &ReviewOutcome{Summary: "Accepted", Result: result}
				}
			case "reject":
				next.State = "rejected"
				next.Outcome = &ReviewOutcome{Summary: "Rejected"}
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
