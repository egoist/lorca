package model

import (
	"crypto/rand"
	"encoding/json"
	"fmt"
	"slices"
	"strings"
	"time"
)

// DurableTask is the CLI's canonical account-encrypted task projection. The UI keeps no
// independent ownership, execution, permission, budget, or persistence ledger.
type DurableTask struct {
	ID                 string         `json:"id"`
	Revision           uint64         `json:"revision"`
	AuthorityRunnerID  string         `json:"authority_runner_id"`
	OwnerBotID         string         `json:"owner_bot_id"`
	RunnerID           string         `json:"runner_id"`
	Goal               string         `json:"goal"`
	AcceptanceCriteria []string       `json:"acceptance_criteria"`
	Dependencies       []string       `json:"dependencies"`
	NextAction         string         `json:"next_action"`
	ChatIDs            []string       `json:"chat_ids"`
	Links              []TaskLink     `json:"links"`
	State              TaskState      `json:"state"`
	Reason             *string        `json:"reason"`
	Result             *string        `json:"result"`
	Evidence           []TaskEvidence `json:"evidence"`
	ActiveRun          *TaskRun       `json:"active_run"`
	CreatedAt          float64        `json:"created_at"`
	UpdatedAt          float64        `json:"updated_at"`
}

type TaskState string

const (
	TaskQueued         TaskState = "queued"
	TaskWorking        TaskState = "working"
	TaskBlocked        TaskState = "blocked"
	TaskAwaitingReview TaskState = "awaiting_review"
	TaskCompleted      TaskState = "completed"
	TaskCancelled      TaskState = "cancelled"
)

// Title is the state in the app's words.
// Title is the state in the app's words.
func (state TaskState) Title() string {
	switch state {
	case TaskQueued:
		return L("Not started")
	case TaskWorking:
		return L("Working")
	case TaskBlocked:
		return L("Blocked")
	case TaskAwaitingReview:
		return L("Ready for review")
	case TaskCompleted:
		return L("Completed")
	case TaskCancelled:
		return L("Cancelled")
	default:
		return string(state)
	}
}

// Symbol is the state's SF Symbol name.
func (state TaskState) Symbol() string {
	switch state {
	case TaskWorking:
		return "arrow.triangle.2.circlepath"
	case TaskBlocked:
		return "exclamationmark.circle.fill"
	case TaskAwaitingReview:
		return "eye"
	case TaskCompleted:
		return "checkmark.circle"
	case TaskCancelled:
		return "xmark.circle"
	default:
		return "circle"
	}
}

func (state TaskState) Finished() bool { return state == TaskCompleted || state == TaskCancelled }

// order puts what waits on the user first, then open work, then finished tasks.
func (state TaskState) order() int {
	switch state {
	case TaskBlocked:
		return 0
	case TaskAwaitingReview:
		return 1
	case TaskWorking:
		return 2
	case TaskQueued:
		return 3
	default:
		return 4
	}
}

// CancelledByUser is why a task the user cancelled in the app stopped, for its bot to read. The
// app shows the state alone for it.
const CancelledByUser = "Cancelled by the user."

// CanStart is whether a run can start: queued or blocked, and not running.
func (task DurableTask) CanStart() bool {
	return (task.State == TaskQueued || task.State == TaskBlocked) && task.ActiveRun == nil
}

type TaskLink struct {
	Label string `json:"label"`
	URL   string `json:"url"`
}
type TaskRun struct {
	ID        string  `json:"id"`
	BotID     string  `json:"bot_id"`
	RunnerID  string  `json:"runner_id"`
	ChatID    string  `json:"chat_id"`
	StartedAt float64 `json:"started_at"`
}
type TaskEvidence struct {
	Kind         string  `json:"kind"`
	Label        string  `json:"label"`
	ChatID       *string `json:"chat_id,omitempty"`
	MessageID    *string `json:"message_id,omitempty"`
	AttachmentID *string `json:"attachment_id,omitempty"`
	URL          *string `json:"url,omitempty"`
	OutputID     *string `json:"output_id,omitempty"`
	Version      *uint64 `json:"version,omitempty"`
	ReviewID     *string `json:"review_id,omitempty"`
}

func (task DurableTask) ReasonText() string { return str(task.Reason) }
func (task DurableTask) ResultText() string { return str(task.Result) }

// Clone detaches nested draft state from subsequent snapshot/event replacements.
func (task DurableTask) Clone() DurableTask {
	data, _ := json.Marshal(task)
	var out DurableTask
	_ = json.Unmarshal(data, &out)
	return out
}

// TaskID creates an opaque id/request key before dispatch, retained across ambiguous replies.
func TaskID(prefix string) string {
	var value [16]byte
	if _, err := rand.Read(value[:]); err != nil {
		panic(err)
	}
	value[6] = (value[6] & 0x0f) | 0x40
	value[8] = (value[8] & 0x3f) | 0x80
	return fmt.Sprintf("%s%08x-%04x-%04x-%04x-%012x", prefix, value[0:4], value[4:6], value[6:8], value[8:10], value[10:16])
}

func (s *Store) DurableTask(id string) *DurableTask {
	for _, task := range s.DurableTasks {
		if task.ID == id {
			return task
		}
	}
	return nil
}
func (s *Store) TasksIn(chatID string) []*DurableTask {
	var tasks []*DurableTask
	for _, task := range s.DurableTasks {
		if slices.Contains(task.ChatIDs, chatID) {
			tasks = append(tasks, task)
		}
	}
	// What waits on the user first, then open work, each newest first.
	slices.SortStableFunc(tasks, func(a, b *DurableTask) int {
		if a.State.order() != b.State.order() {
			return a.State.order() - b.State.order()
		}
		if a.UpdatedAt > b.UpdatedAt {
			return -1
		}
		if a.UpdatedAt < b.UpdatedAt {
			return 1
		}
		return strings.Compare(a.ID, b.ID)
	})
	return tasks
}

// AcceptDurableTask accepts only newer CLI revisions, including replies overtaken by events.
func (s *Store) AcceptDurableTask(task DurableTask) bool {
	copy := task.Clone()
	for i, old := range s.DurableTasks {
		if old.ID == task.ID {
			if old.Revision >= task.Revision {
				return false
			}
			s.DurableTasks[i] = &copy
			s.emit(Event{Kind: EventDurableTasksChanged})
			return true
		}
	}
	s.DurableTasks = append(s.DurableTasks, &copy)
	s.emit(Event{Kind: EventDurableTasksChanged})
	return true
}

// TaskRequest freezes parameters on the UI thread; worker replies rejoin the ordered post
// queue before touching the store or a sheet. The CLI enforces all task authority/CAS rules.
func (s *Store) TaskRequest(method string, params map[string]any, done func(DurableTask, error)) {
	identity := s.IdentityID
	finish := func(task DurableTask, err error) {
		if identity != s.IdentityID {
			task, err = DurableTask{}, &RequestError{L("The account changed. Open the task again.")}
		}
		if err == nil {
			s.AcceptDurableTask(task)
			task = s.DurableTask(task.ID).Clone()
		}
		if done != nil {
			done(task, err)
		}
	}
	payload, err := json.Marshal(params)
	if err != nil {
		s.post(func() {
			if done != nil {
				done(DurableTask{}, err)
			}
		})
		return
	}
	if s.IsMock {
		s.post(func() {
			if identity != s.IdentityID {
				finish(DurableTask{}, nil)
				return
			}
			task, err := s.mockTaskRequest(method, payload)
			finish(task, err)
		})
		return
	}
	Async(s, func() (DurableTask, error) { return call[DurableTask](s, method, json.RawMessage(payload)) }, finish)
}

// Mock mode edits only the existing demo projection. It runs no bot, provider, or relay.
func (s *Store) mockTaskRequest(method string, payload []byte) (DurableTask, error) {
	var p map[string]json.RawMessage
	if err := json.Unmarshal(payload, &p); err != nil {
		return DurableTask{}, err
	}
	var id string
	_ = json.Unmarshal(p["id"], &id)
	old := s.DurableTask(id)
	if method == "tasks.get" {
		if old == nil {
			return DurableTask{}, &RequestError{"Unknown task"}
		}
		return old.Clone(), nil
	}
	var next DurableTask
	if method == "tasks.create" {
		if old != nil {
			return old.Clone(), nil
		}
		if err := json.Unmarshal(payload, &next); err != nil {
			return next, err
		}
		bot := s.Bot(next.OwnerBotID)
		if bot == nil {
			return next, &RequestError{"Unknown owning bot"}
		}
		next.AuthorityRunnerID, next.RunnerID = bot.RunnerID, bot.RunnerID
		next.State, next.CreatedAt = TaskQueued, float64(time.Now().Unix())
	} else {
		if old == nil {
			return next, &RequestError{"Unknown task"}
		}
		var expected uint64
		_ = json.Unmarshal(p["expected_revision"], &expected)
		if expected != old.Revision {
			return next, &RequestError{fmt.Sprintf("Task revision conflict: expected %d, current %d. Read the task again.", expected, old.Revision)}
		}
		next = old.Clone()
		data, _ := json.Marshal(next)
		var merged map[string]json.RawMessage
		_ = json.Unmarshal(data, &merged)
		for _, field := range []string{"owner_bot_id", "runner_id", "goal", "acceptance_criteria", "dependencies", "next_action", "chat_ids", "links", "state", "reason", "result", "evidence"} {
			if value, ok := p[field]; ok {
				merged[field] = value
			}
		}
		data, _ = json.Marshal(merged)
		_ = json.Unmarshal(data, &next)
		// A new state without a reason clears the old one, as the CLI does.
		if _, ok := p["reason"]; !ok && next.State != old.State {
			next.Reason = nil
		}
		if next.OwnerBotID != old.OwnerBotID {
			if bot := s.Bot(next.OwnerBotID); bot != nil {
				next.RunnerID = bot.RunnerID
			}
		}
		if method == "tasks.run" {
			if !next.CanStart() {
				return next, &RequestError{"Task already has a run or awaits review"}
			}
			var chatID string
			_ = json.Unmarshal(p["chat_id"], &chatID)
			next.State, next.Reason = TaskWorking, nil
			next.ActiveRun = &TaskRun{ID: TaskID("task-run-"), BotID: next.OwnerBotID, RunnerID: next.RunnerID, ChatID: chatID, StartedAt: float64(time.Now().Unix())}
		}
	}
	if strings.TrimSpace(next.Goal) == "" || len(next.AcceptanceCriteria) == 0 || strings.TrimSpace(next.NextAction) == "" {
		return next, &RequestError{"Goal, next action, and acceptance criteria are required."}
	}
	if (next.State == TaskBlocked || next.State == TaskCancelled) && strings.TrimSpace(next.ReasonText()) == "" {
		return next, &RequestError{"Blocked and cancelled tasks require a reason."}
	}
	if next.State == TaskCompleted && (strings.TrimSpace(next.ResultText()) == "" || len(next.Evidence) == 0) {
		return next, &RequestError{"Completion requires a result and supporting evidence."}
	}
	if old != nil {
		next.Revision = old.Revision + 1
	} else {
		next.Revision = 1
	}
	next.UpdatedAt = float64(time.Now().Unix())
	return next, nil
}
