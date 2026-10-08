package model

import (
	"encoding/json"
	"fmt"
	"maps"
	"slices"
)

// This is presentation-only demo state, like the existing mock marketplace and reply engine.
// A real Store always sends workflows.* to the CLI; the demo holds no keys or connections.
func demoWorkflowPacks() []WorkflowPack {
	return []WorkflowPack{
		{ID: "meeting-preparation", Name: "Meeting preparation", Outcome: "Arrive at your next meeting with a briefing and agenda.", Description: "Review a meeting brief before enabling a routine.",
			Questions:   []WorkflowQuestion{{ID: "meeting-scope", Label: "Which meetings should the briefing cover?", Placeholder: "Upcoming meetings today, with external attendees"}},
			Connections: []WorkflowRequirement{{ServiceID: "google-calendar", Name: "Google Calendar"}, {ServiceID: "google-drive", Name: "Google Drive"}}},
		{ID: "inbox-triage", Name: "Inbox triage", Outcome: "See the messages that need you and a draft of the next action.", Description: "Review a small inbox triage sample before enabling a routine.",
			Questions:   []WorkflowQuestion{{ID: "inbox-scope", Label: "Which messages should triage include?", Placeholder: "Unread messages from the last day"}, {ID: "priorities", Label: "What needs your attention first?", Placeholder: "Customer replies, deadlines and blocked teammates"}},
			Connections: []WorkflowRequirement{{ServiceID: "gmail", Name: "Gmail"}}},
		{ID: "repository-monitoring", Name: "Repository monitoring", Outcome: "Keep up with the issues, pull requests and releases that need you.", Description: "Review a repository summary before enabling a routine.",
			Questions:   []WorkflowQuestion{{ID: "repositories", Label: "Which repositories should be monitored?", Placeholder: "owner/repository, owner/another-repository"}},
			Connections: []WorkflowRequirement{{ServiceID: "github", Name: "GitHub"}}},
	}
}

func (s *Store) demoWorkflow(method string, data json.RawMessage) (WireWorkflowProgress, error) {
	var params struct {
		ID        string            `json:"id"`
		PackID    string            `json:"pack_id"`
		RunnerID  string            `json:"runner_id"`
		Answers   map[string]string `json:"answers"`
		BotIDs    map[string]string `json:"bot_ids"`
		ServiceID string            `json:"service_id"`
		PluginID  string            `json:"plugin_id"`
		JobID     string            `json:"job_id"`
	}
	if err := json.Unmarshal(data, &params); err != nil {
		return WireWorkflowProgress{}, err
	}
	if s.mockWorkflows == nil {
		s.mockWorkflows = map[string]WireWorkflowProgress{}
	}
	id := params.ID
	if method == "start" {
		id = "demo-workflow-" + params.PackID + "-" + params.RunnerID
		if s.Device(params.RunnerID) == nil {
			return WireWorkflowProgress{}, fmt.Errorf("Choose a Runner for this workflow.")
		}
		if _, ok := s.mockWorkflows[id]; !ok {
			packs := demoWorkflowPacks()
			i := slices.IndexFunc(packs, func(p WorkflowPack) bool { return p.ID == params.PackID })
			if i < 0 {
				return WireWorkflowProgress{}, fmt.Errorf("This workflow is no longer in the marketplace.")
			}
			p := packs[i]
			var bots []WorkflowBot
			for _, bot := range s.Bots {
				if bot.RunnerID == params.RunnerID {
					bots = append(bots, WorkflowBot{ID: bot.ID, Name: bot.Name})
				}
			}
			progress := WireWorkflowProgress{Setup: WorkflowSetup{ID: id, RunnerID: params.RunnerID, Pack: p, Answers: map[string]string{}, BotIDs: map[string]string{}, ConnectionIDs: map[string]string{}, Phase: "questions"},
				Specialists: []WorkflowSpecialist{{ID: "specialist", Name: p.Name, Choices: bots}}}
			for _, requirement := range p.Connections {
				connection := WorkflowConnection{ServiceID: requirement.ServiceID, Name: requirement.Name, Available: true, State: "missing", Detail: "Choose or add an account on this Runner."}
				if requirement.ServiceID == "github" {
					connection.Choices = []WorkflowAccount{{ID: "github", Name: "GitHub", State: PluginReady, Detail: "Connected (demo)"}}
				} else {
					for _, label := range []string{"Work", "Personal"} {
						connection.Choices = append(connection.Choices, WorkflowAccount{ID: requirement.ServiceID + "-demo-" + label, Name: requirement.Name, ServiceID: requirement.ServiceID, AccountName: label + " (demo)", State: PluginReady, Detail: "Connected (demo)"})
					}
				}
				progress.Connections = append(progress.Connections, connection)
			}
			s.mockWorkflows[id] = progress
		}
	}
	progress, ok := s.mockWorkflows[id]
	if !ok {
		return WireWorkflowProgress{}, fmt.Errorf("Unknown workflow setup.")
	}
	switch method {
	case "start":
		if progress.Setup.Phase == "cancelled" {
			progress.Setup.Phase = "questions"
			if len(progress.Setup.BotIDs) > 0 {
				progress.Setup.Phase = "connections"
			}
		}
	case "get":
	case "configure":
		for _, q := range progress.Setup.Pack.Questions {
			if params.Answers[q.ID] == "" {
				return WireWorkflowProgress{}, fmt.Errorf("Answer %s.", q.Label)
			}
		}
		progress.Setup.Answers = maps.Clone(params.Answers)
		botID := params.BotIDs["specialist"]
		if botID == "" && len(progress.Specialists[0].Choices) > 0 {
			botID = progress.Specialists[0].Choices[0].ID
		}
		progress.Setup.BotIDs = map[string]string{"specialist": botID}
		progress.Specialists[0].SelectedID = botID
		progress.Routines = []WorkflowRoutine{{ID: "demo-routine-" + id, Name: progress.Setup.Pack.Name, ScheduleText: "Weekdays at 9:00 AM", IsEnabled: false}}
		progress.Setup.Sample = nil
		progress.Setup.Phase = "connections"
	case "connection", "clear_connection":
		i := slices.IndexFunc(progress.Connections, func(c WorkflowConnection) bool { return c.ServiceID == params.ServiceID })
		if i < 0 {
			return WireWorkflowProgress{}, fmt.Errorf("This workflow does not require that integration.")
		}
		connection := &progress.Connections[i]
		if method == "clear_connection" {
			connection.SelectedID = ""
			connection.State = "missing"
			delete(progress.Setup.ConnectionIDs, params.ServiceID)
		} else {
			choice := slices.IndexFunc(connection.Choices, func(a WorkflowAccount) bool { return a.ID == params.PluginID })
			if choice < 0 {
				return WireWorkflowProgress{}, fmt.Errorf("Choose an existing account before adding another.")
			}
			connection.SelectedID = params.PluginID
			connection.State = connection.Choices[choice].State
			connection.Detail = connection.Choices[choice].Detail
			progress.Setup.ConnectionIDs[params.ServiceID] = params.PluginID
		}
		progress.Setup.Sample = nil
		progress.Setup.Phase = "connections"
	case "sample":
		if !demoWorkflowReady(progress) {
			return WireWorkflowProgress{}, fmt.Errorf("Choose ready accounts before running a sample.")
		}
		botID := progress.Setup.BotIDs["specialist"]
		chatID := ""
		for _, chat := range s.Chats {
			if chat.Kind == ChatDM && slices.Contains(chat.BotIDs, botID) {
				chatID = chat.ID
				break
			}
		}
		progress.Setup.Sample = &WorkflowSample{JobID: "demo-sample-" + id, ChatID: chatID, BotID: botID, State: "ready", MessageIDs: []string{"demo-sample-message"}}
		progress.SampleMessages = []WireMessage{{ID: "demo-sample-message", Author: WireAuthor{Kind: "bot", BotID: &botID}, Body: WireBody{Kind: "text", Text: stringPointer("Demo sample: two items need your attention. Review the draft before enabling its schedule.")}, CreatedAt: 1}}
		progress.SampleMessages[0].State.Kind = "complete"
		progress.Setup.Phase = "sample"
		for i := range progress.Routines {
			progress.Routines[i].IsEnabled = false
		}
	case "review":
		if progress.Setup.Sample == nil || progress.Setup.Sample.JobID != params.JobID || progress.Setup.Sample.State != "ready" {
			return WireWorkflowProgress{}, fmt.Errorf("Review the completed result of the current sample first.")
		}
		progress.Setup.Sample.State = "reviewed"
		progress.Setup.Phase = "reviewed"
	case "enable":
		if progress.Setup.Sample == nil || progress.Setup.Sample.State != "reviewed" {
			return WireWorkflowProgress{}, fmt.Errorf("Run and review a sample before enabling schedules.")
		}
		for i := range progress.Routines {
			progress.Routines[i].IsEnabled = true
		}
		progress.Setup.Phase = "enabled"
	case "cancel":
		progress.Setup.Phase = "cancelled"
		progress.Setup.Sample = nil
		progress.SampleMessages = nil
		for i := range progress.Routines {
			progress.Routines[i].IsEnabled = false
		}
	}
	progress.CanSample = demoWorkflowReady(progress) && progress.Setup.Phase != "cancelled"
	progress.CanEnable = progress.CanSample && progress.Setup.Sample != nil && progress.Setup.Sample.State == "reviewed"
	progress.BlockedReason = ""
	if !demoWorkflowReady(progress) {
		progress.BlockedReason = "Choose and connect the required accounts before running a sample."
	}
	s.mockWorkflows[id] = progress
	// A callback receives an independent reply, like JSON decoded from a real CLI request.
	clone, _ := json.Marshal(progress)
	var out WireWorkflowProgress
	_ = json.Unmarshal(clone, &out)
	return out, nil
}

func demoWorkflowReady(p WireWorkflowProgress) bool {
	if len(p.Setup.BotIDs) == 0 {
		return false
	}
	for _, c := range p.Connections {
		if c.SelectedID == "" || c.State != PluginReady {
			return false
		}
	}
	return true
}

func stringPointer(s string) *string { return &s }
