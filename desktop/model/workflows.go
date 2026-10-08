package model

import (
	"encoding/json"
	"fmt"
)

// WorkflowPack is a guided workflow from the marketplace: an outcome, the questions it asks, and
// the accounts it needs. A CLI whose index has none answers without packs.
type WorkflowPack struct {
	ID          string                `json:"id"`
	Name        string                `json:"name"`
	Outcome     string                `json:"outcome"`
	Description string                `json:"description"`
	SymbolName  string                `json:"symbol_name"`
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
	JobID  string `json:"job_id"`
	ChatID string `json:"chat_id"`
	BotID  string `json:"bot_id"`
	// State is running, ready, failed, or reviewed.
	State string `json:"state"`
}

type WorkflowSetup struct {
	ID            string            `json:"id"`
	RunnerID      string            `json:"runner_id"`
	Pack          WorkflowPack      `json:"pack"`
	Answers       map[string]string `json:"answers"`
	BotIDs        map[string]string `json:"bot_ids"`
	ConnectionIDs map[string]string `json:"connection_ids"`
	// Phase is questions, connections, sample, reviewed, enabled, or cancelled.
	Phase  string          `json:"phase"`
	Sample *WorkflowSample `json:"sample"`
}

// WorkflowAccount is one of the Runner's installed plugins for a service: a named account, or the
// plugin.
type WorkflowAccount struct {
	ID          string      `json:"id"`
	Name        string      `json:"name"`
	AccountName string      `json:"account_name"`
	State       PluginState `json:"state"`
	Detail      string      `json:"detail"`
}

type WorkflowConnection struct {
	ServiceID  string            `json:"service_id"`
	Name       string            `json:"name"`
	SelectedID string            `json:"selected_id"`
	Choices    []WorkflowAccount `json:"choices"`
	// Available is the marketplace having the service, so it can be added.
	Available bool `json:"available"`
}

// Account is the account in use: the one chosen, else the Runner's only one, which setup takes.
func (c WorkflowConnection) Account() *WorkflowAccount {
	for i := range c.Choices {
		if c.SelectedID != "" && c.Choices[i].ID == c.SelectedID || c.SelectedID == "" && len(c.Choices) == 1 {
			return &c.Choices[i]
		}
	}
	return nil
}

type WorkflowBot struct {
	ID   string `json:"id"`
	Name string `json:"name"`
}

type WorkflowSpecialist struct {
	ID   string `json:"id"`
	Name string `json:"name"`
	// SelectedID is the bot setup uses, or empty when it will add a new one.
	SelectedID string        `json:"selected_id"`
	Choices    []WorkflowBot `json:"choices"`
}

type WorkflowRoutine struct {
	ID           string `json:"id"`
	Name         string `json:"name"`
	ScheduleText string `json:"schedule_text"`
	IsEnabled    bool   `json:"is_enabled"`
}

type WorkflowSampleMessage struct {
	ID   string `json:"id"`
	Body struct {
		Text string `json:"text"`
	} `json:"body"`
}

// WorkflowProgress is a pack's setup on one Runner, as every workflows.* request answers it. The
// CLI keeps it; the page only shows it.
type WorkflowProgress struct {
	Setup          WorkflowSetup           `json:"setup"`
	Connections    []WorkflowConnection    `json:"connections"`
	Specialists    []WorkflowSpecialist    `json:"specialists"`
	Routines       []WorkflowRoutine       `json:"routines"`
	SampleMessages []WorkflowSampleMessage `json:"sample_messages"`
	IsRunning      bool                    `json:"is_running"`
}

// Workflow sends workflows.<method>, its parameters copied before the request leaves the main
// thread; the answer, or the error, comes back in order with the store's other posts.
func (s *Store) Workflow(method string, params map[string]any, done func(WorkflowProgress, error)) {
	switch method {
	case "start", "get", "configure", "connection", "sample", "review", "enable", "cancel":
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
			progress, err := s.demoWorkflow(method, snapshot)
			done(progress, err)
		})
		return
	}
	Async(s, func() (WorkflowProgress, error) {
		return call[WorkflowProgress](s, "workflows."+method, json.RawMessage(snapshot))
	}, done)
}
