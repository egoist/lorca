package model

import (
	"encoding/json"
	"fmt"
	"maps"
	"slices"
	"time"
)

// The demo's stand-in for the CLI's setups, as the macOS app's MockWorkflows: the same answers,
// from the demo roster, with a sample that finishes a moment after it starts.

func demoWorkflowPacks() []WorkflowPack {
	return []WorkflowPack{
		{ID: "meeting-preparation", Name: "Meeting preparation", SymbolName: "calendar", Outcome: "Arrive at your next meeting with a briefing and agenda.",
			Description: "Choose a calendar and document account, then review one meeting brief before enabling a weekday routine.",
			Questions:   []WorkflowQuestion{{ID: "meeting-scope", Label: "Meetings", Placeholder: "Today’s meetings with people outside the team"}},
			Connections: []WorkflowRequirement{{ServiceID: "google-calendar", Name: "Google Calendar"}, {ServiceID: "google-drive", Name: "Google Drive"}}},
		{ID: "inbox-triage", Name: "Inbox triage", SymbolName: "envelope", Outcome: "See the messages that need you and a draft of the next action.",
			Description: "Choose an inbox, define what matters, and review a small triage sample before enabling a weekday routine.",
			Questions: []WorkflowQuestion{{ID: "inbox-scope", Label: "Messages", Placeholder: "Unread messages from the last day"},
				{ID: "priorities", Label: "Priorities", Placeholder: "Customer replies, deadlines, blocked teammates"}},
			Connections: []WorkflowRequirement{{ServiceID: "gmail", Name: "Gmail"}}},
		{ID: "repository-monitoring", Name: "Repository monitoring", SymbolName: "arrow.triangle.branch", Outcome: "Keep up with the issues, pull requests and releases that need you.",
			Description: "Select repositories and a GitHub connection, then review a summary before enabling a weekday routine.",
			Questions:   []WorkflowQuestion{{ID: "repositories", Label: "Repositories", Placeholder: "owner/repo, owner/another-repo"}},
			Connections: []WorkflowRequirement{{ServiceID: "github", Name: "GitHub"}}},
	}
}

var demoWorkflowSpecialists = map[string]string{"meeting-preparation": "Meeting Preparer", "inbox-triage": "Inbox Triager", "repository-monitoring": "Repository Monitor"}
var demoWorkflowRoutines = map[string]string{"meeting-preparation": "Prepare upcoming meetings", "inbox-triage": "Triage the selected inbox", "repository-monitoring": "Monitor selected repositories"}

const demoWorkflowSample = `**example/workflow-demo** has two pull requests waiting on you and one new issue.

- **#128 Fix token refresh on wake**: approved by one reviewer, needs yours.
- **#131 Pin the relay image**: CI is green; small change.
- **#133 Crash when pairing offline**: no steps to reproduce yet.

Start with #128: it blocks the release. Then ask the reporter of #133 for a crash log.`

// DemoWorkflowSampleTime is how long the demo's sample runs.
var DemoWorkflowSampleTime = 2500 * time.Millisecond

// demoWorkflowAccounts are the named accounts the demo's Runners have, by service: two inboxes to
// pick from, a calendar that waits for its sign-in, and no Drive yet.
func demoWorkflowAccounts() map[string][]WorkflowAccount {
	return map[string][]WorkflowAccount{
		"gmail": {{ID: "gmail-work", Name: "Gmail · Work", AccountName: "Work", State: PluginReady, Detail: "Connected"},
			{ID: "gmail-personal", Name: "Gmail · Personal", AccountName: "Personal", State: PluginReady, Detail: "Connected"}},
		"google-calendar": {{ID: "google-calendar-work", Name: "Google Calendar · Work", AccountName: "Work", State: PluginNeedsAuth, Detail: "Sign in"}},
	}
}

type demoSetup struct {
	id, runnerID, packID, botID, phase, sample string
	answers, connections                       map[string]string
	routineOn                                  bool
}

func (s *Store) demoWorkflow(method string, data json.RawMessage) (WorkflowProgress, error) {
	var params struct {
		ID        string            `json:"id"`
		PackID    string            `json:"pack_id"`
		RunnerID  string            `json:"runner_id"`
		Answers   map[string]string `json:"answers"`
		BotIDs    map[string]string `json:"bot_ids"`
		ServiceID string            `json:"service_id"`
		PluginID  string            `json:"plugin_id"`
	}
	if err := json.Unmarshal(data, &params); err != nil {
		return WorkflowProgress{}, err
	}
	if s.mockWorkflows == nil {
		s.mockWorkflows = map[string]*demoSetup{}
	}
	if method == "start" {
		if !slices.ContainsFunc(demoWorkflowPacks(), func(p WorkflowPack) bool { return p.ID == params.PackID }) {
			return WorkflowProgress{}, fmt.Errorf("This workflow is no longer in the marketplace.")
		}
		id := "workflow-" + params.PackID + "-" + params.RunnerID
		setup := s.mockWorkflows[id]
		if setup == nil {
			setup = &demoSetup{id: id, runnerID: params.RunnerID, packID: params.PackID, answers: map[string]string{}, connections: map[string]string{}}
			s.mockWorkflows[id] = setup
		}
		if setup.phase == "" || setup.phase == "cancelled" {
			setup.phase = "questions"
		}
		return s.demoWorkflowProgress(setup), nil
	}
	setup := s.mockWorkflows[params.ID]
	if setup == nil {
		return WorkflowProgress{}, fmt.Errorf("Unknown workflow setup.")
	}
	switch method {
	case "configure":
		setup.answers = maps.Clone(params.Answers)
		for _, id := range params.BotIDs {
			setup.botID = id
		}
		if setup.botID == "" {
			setup.botID = s.CreateBot(NewBot{Name: demoWorkflowSpecialists[setup.packID], SymbolName: "eye.fill", Accent: "purple", RunnerID: setup.runnerID, Provider: s.PreferredProvider()})
		}
		setup.phase = "connections"
	case "connection":
		setup.connections[params.ServiceID] = params.PluginID
		if params.PluginID == "" {
			// Adds a named account for the workflow, waiting for its sign-in.
			if s.mockWorkflowAccounts == nil {
				s.mockWorkflowAccounts = demoWorkflowAccounts()
			}
			pack := demoWorkflowPacks()[slices.IndexFunc(demoWorkflowPacks(), func(p WorkflowPack) bool { return p.ID == setup.packID })]
			name := params.ServiceID
			for _, requirement := range pack.Connections {
				if requirement.ServiceID == params.ServiceID {
					name = requirement.Name
				}
			}
			id := params.ServiceID + "-added"
			s.mockWorkflowAccounts[params.ServiceID] = append(s.mockWorkflowAccounts[params.ServiceID],
				WorkflowAccount{ID: id, Name: name + " · " + pack.Name, AccountName: pack.Name, State: PluginNeedsAuth, Detail: "Sign in"})
			setup.connections[params.ServiceID] = id
		}
	case "sample":
		setup.sample, setup.phase = "running", "sample"
		id := setup.id
		time.AfterFunc(DemoWorkflowSampleTime, func() {
			s.post(func() {
				if setup := s.mockWorkflows[id]; setup != nil && setup.sample == "running" {
					setup.sample = "ready"
					s.emit(Event{Kind: EventRosterChanged})
				}
			})
		})
	case "review":
		setup.sample, setup.phase = "reviewed", "reviewed"
	case "enable":
		setup.routineOn, setup.phase = true, "enabled"
	case "cancel":
		setup.sample, setup.routineOn, setup.phase = "", false, "cancelled"
	}
	return s.demoWorkflowProgress(setup), nil
}

func (s *Store) demoWorkflowProgress(setup *demoSetup) WorkflowProgress {
	packs := demoWorkflowPacks()
	pack := packs[slices.IndexFunc(packs, func(p WorkflowPack) bool { return p.ID == setup.packID })]
	progress := WorkflowProgress{
		Setup: WorkflowSetup{ID: setup.id, RunnerID: setup.runnerID, Pack: pack, Answers: maps.Clone(setup.answers),
			BotIDs: map[string]string{}, ConnectionIDs: maps.Clone(setup.connections), Phase: setup.phase},
		IsRunning: setup.sample == "running",
	}
	specialist := WorkflowSpecialist{ID: "specialist", Name: demoWorkflowSpecialists[setup.packID], SelectedID: setup.botID}
	for _, bot := range s.Bots {
		if bot.RunnerID == setup.runnerID {
			specialist.Choices = append(specialist.Choices, WorkflowBot{ID: bot.ID, Name: bot.Name})
		}
	}
	progress.Specialists = []WorkflowSpecialist{specialist}
	if setup.botID != "" {
		progress.Setup.BotIDs["specialist"] = setup.botID
		progress.Routines = []WorkflowRoutine{{ID: "demo-routine", Name: demoWorkflowRoutines[setup.packID], ScheduleText: "Weekdays at 9:00 AM", IsEnabled: setup.routineOn}}
	}
	if setup.sample != "" {
		progress.Setup.Sample = &WorkflowSample{JobID: "demo-job", ChatID: s.DM(setup.botID), BotID: setup.botID, State: setup.sample}
	}
	if setup.sample == "ready" || setup.sample == "reviewed" {
		message := WorkflowSampleMessage{ID: "demo-sample"}
		message.Body.Text = demoWorkflowSample
		progress.SampleMessages = []WorkflowSampleMessage{message}
	}
	var plugins []InstalledPlugin
	if runner := s.Device(setup.runnerID); runner != nil {
		plugins = runner.Plugins
	}
	if s.mockWorkflowAccounts == nil {
		s.mockWorkflowAccounts = demoWorkflowAccounts()
	}
	for _, requirement := range pack.Connections {
		connection := WorkflowConnection{ServiceID: requirement.ServiceID, Name: requirement.Name, SelectedID: setup.connections[requirement.ServiceID], Available: true}
		for _, plugin := range plugins {
			if plugin.MarketplaceID() == requirement.ServiceID && !plugin.IsMcpServer() {
				connection.Choices = append(connection.Choices, WorkflowAccount{ID: plugin.ID, Name: plugin.Name, AccountName: plugin.AccountName, State: plugin.State, Detail: plugin.Detail})
			}
		}
		connection.Choices = append(connection.Choices, s.mockWorkflowAccounts[requirement.ServiceID]...)
		// What configure takes, as the CLI does: a service's only account.
		if connection.SelectedID == "" && setup.botID != "" && len(connection.Choices) == 1 {
			connection.SelectedID = connection.Choices[0].ID
			progress.Setup.ConnectionIDs[requirement.ServiceID] = connection.SelectedID
		}
		progress.Connections = append(progress.Connections, connection)
	}
	return progress
}
