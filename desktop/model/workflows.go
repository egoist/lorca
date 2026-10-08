package model

import (
	"encoding/json"
	"fmt"
)

// WorkflowPack is the additive marketplace outcome contract. Older replies omit Packs.
// Runtime authority and durable setup state belong to the local CLI.
type WorkflowPack struct {
	ID          string                `json:"id"`
	Name        string                `json:"name"`
	Outcome     string                `json:"outcome"`
	Description string                `json:"description"`
	Questions   []WorkflowQuestion    `json:"questions"`
	Connections []WorkflowRequirement `json:"connections"`
}

type WorkflowQuestion struct {
	ID          string `json:"id"`
	Label       string `json:"label"`
	Placeholder string `json:"placeholder"`
}
type WorkflowRequirement struct {
	ServiceID string `json:"service_id"`
	Name      string `json:"name"`
}

type WorkflowSample struct {
	JobID      string   `json:"job_id"`
	ChatID     string   `json:"chat_id"`
	BotID      string   `json:"bot_id"`
	State      string   `json:"state"`
	MessageIDs []string `json:"message_ids"`
	Error      string   `json:"error"`
}

type WorkflowSetup struct {
	ID            string            `json:"id"`
	RunnerID      string            `json:"runner_id"`
	Pack          WorkflowPack      `json:"pack"`
	Answers       map[string]string `json:"answers"`
	BotIDs        map[string]string `json:"bot_ids"`
	ConnectionIDs map[string]string `json:"connection_ids"`
	Phase         string            `json:"phase"`
	Sample        *WorkflowSample   `json:"sample"`
}

type WorkflowAccount struct {
	ID          string      `json:"id"`
	Name        string      `json:"name"`
	ServiceID   string      `json:"service_id"`
	AccountName string      `json:"account_name"`
	State       PluginState `json:"state"`
	Detail      string      `json:"detail"`
}

func (a WorkflowAccount) Label() string {
	if a.AccountName != "" {
		return a.Name + " · " + a.AccountName
	}
	return a.Name
}

type WorkflowConnection struct {
	ServiceID  string            `json:"service_id"`
	Name       string            `json:"name"`
	SelectedID string            `json:"selected_id"`
	Choices    []WorkflowAccount `json:"choices"`
	Available  bool              `json:"available"`
	State      PluginState       `json:"state"`
	Detail     string            `json:"detail"`
}

type WorkflowBot struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}
type WorkflowSpecialist struct {
	ID         string        `json:"id"`
	Name       string        `json:"name"`
	SelectedID string        `json:"selected_id"`
	Choices    []WorkflowBot `json:"choices"`
}

type WorkflowRoutine struct {
	ID           string `json:"id"`
	Name         string `json:"name"`
	ScheduleText string `json:"schedule_text"`
	IsEnabled    bool   `json:"is_enabled"`
}

type WireWorkflowProgress struct {
	Setup          WorkflowSetup        `json:"setup"`
	Connections    []WorkflowConnection `json:"connections"`
	Specialists    []WorkflowSpecialist `json:"specialists"`
	Routines       []WorkflowRoutine    `json:"routines"`
	SampleMessages []WireMessage        `json:"sample_messages"`
	IsRunning      bool                 `json:"is_running"`
	CanSample      bool                 `json:"can_sample"`
	CanEnable      bool                 `json:"can_enable"`
	BlockedReason  string               `json:"blocked_reason"`
}

type WorkflowProgress struct {
	Setup          WorkflowSetup
	Connections    []WorkflowConnection
	Specialists    []WorkflowSpecialist
	Routines       []WorkflowRoutine
	SampleMessages []*Message
	IsRunning      bool
	CanSample      bool
	CanEnable      bool
	BlockedReason  string
}

func ToWorkflowProgress(w WireWorkflowProgress) WorkflowProgress {
	out := WorkflowProgress{Setup: w.Setup, Connections: w.Connections, Specialists: w.Specialists, Routines: w.Routines,
		IsRunning: w.IsRunning, CanSample: w.CanSample, CanEnable: w.CanEnable, BlockedReason: w.BlockedReason}
	for _, message := range w.SampleMessages {
		out.SampleMessages = append(out.SampleMessages, ToMessage(message))
	}
	return out
}

// Workflow snapshots parameters before leaving the main thread. Both successful and failed
// responses reach the caller through the same ordered post queue as roster and job events.
func (s *Store) Workflow(method string, params map[string]any, done func(WorkflowProgress, error)) {
	switch method {
	case "start", "get", "configure", "connection", "clear_connection", "sample", "review", "enable", "cancel":
	default:
		s.post(func() { done(WorkflowProgress{}, fmt.Errorf("unknown workflow method %q", method)) })
		return
	}
	snapshot, err := json.Marshal(params)
	if err != nil {
		s.post(func() { done(WorkflowProgress{}, err) })
		return
	}
	if s.IsMock && s.transport == nil {
		s.post(func() {
			wire, err := s.demoWorkflow(method, snapshot)
			done(ToWorkflowProgress(wire), err)
		})
		return
	}
	Async(s, func() (WorkflowProgress, error) {
		wire, err := call[WireWorkflowProgress](s, "workflows."+method, json.RawMessage(snapshot))
		return ToWorkflowProgress(wire), err
	}, done)
}
