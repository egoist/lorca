package main

import (
	"encoding/json"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func TestTheAgentCardDecodesFromTheCLI(t *testing.T) {
	var wire model.WireMessage
	raw := `{"id":"m1","chat_id":"c1","author":{"kind":"bot","bot_id":"b1"},"state":{"kind":"complete"},"created_at":1,
	 "body":{"kind":"tool","name":"coding_agent","summary":"Started","detail":"","is_running":false,
	  "agent":{"id":"agent-1","kind":"codex","host":"herdr","task":"Add dark mode","folder":"~/x/dark","branch":"dark",
	   "state":"asking","question":{"kind":"command","command":"git push","reason":"It publishes.","rule":"push branches"}}}}`
	if err := json.Unmarshal([]byte(raw), &wire); err != nil {
		t.Fatal(err)
	}
	message := model.ToMessage(wire)
	agent := model.AgentOf(message)
	if agent == nil || !message.Body.Tool.IsShown() {
		t.Fatalf("no card shown: %+v", message.Body.Tool)
	}
	if agent.Name() != "Codex" || agent.HostName() != "Herdr" || agent.Question == nil || !agent.Question.IsPermission() || len(agent.Question.Decisions()) != 3 {
		t.Errorf("decoded %+v %+v", agent, agent.Question)
	}
	if got := agentTitle(agent, "Developer"); got != "Codex wants to run a command" {
		t.Errorf("title %q", got)
	}
}

func TestTheAgentCardSaysHowItStands(t *testing.T) {
	demoWindow(t)
	agent := &model.AgentRun{Kind: "claude", Task: "Fix it", Branch: "fix-it", State: model.AgentWorking, Device: "Workbench"}
	if got := agentDetail(agent); got != "Working · fix-it" {
		t.Errorf("working %q", got)
	}
	agent.State, agent.Outcome = model.AgentFailed, "Claude Code exited with code 1"
	if got := agentDetail(agent); got != "Failed: Claude Code exited with code 1 · fix-it" {
		t.Errorf("failed %q", got)
	}
	agent.State = model.AgentIdle
	if agent.IsRunning() {
		t.Error("a finished agent offers no Stop")
	}
	agent.State, agent.Question = model.AgentAsking, &model.AgentQuestion{Kind: "start"}
	if got := agentTitle(agent, "Developer"); got != "Developer wants to start Claude Code on Workbench" {
		t.Errorf("start %q", got)
	}
	if agent.IsRunning() || agent.Started() {
		t.Error("nothing to stop or read before it starts")
	}
}

// TestRenderAgentCard draws the developer's chat with its agent's card in each state, and the
// transcript it opens.
func TestRenderAgentCard(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	chat := store.Chat("chat-patch")
	var rowID string
	var base model.AgentRun
	for _, message := range chat.Messages {
		if agent := model.AgentOf(message); agent != nil {
			rowID, base = message.ID, *agent
		}
	}
	if rowID == "" {
		t.Fatal("the demo has no agent card")
	}
	set := func(change func(agent *model.AgentRun)) {
		store.Update(rowID, "chat-patch", func(message *model.Message) {
			tool := *message.Body.Tool
			agent := base
			change(&agent)
			tool.Agent = &agent
			message.Body.Tool = &tool
		})
	}
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settle(tt)
	states := []struct {
		name   string
		change func(agent *model.AgentRun)
	}{
		{"working", func(agent *model.AgentRun) {}},
		{"herdr", func(agent *model.AgentRun) { agent.Host = "herdr" }},
		{"start", func(agent *model.AgentRun) {
			agent.State = model.AgentAsking
			agent.Question = &model.AgentQuestion{Kind: "start", Reason: "Claude Code edits files and runs commands on Workbench.", Rule: "start coding agents in the site repository", HasRule: true}
		}},
		{"command", func(agent *model.AgentRun) {
			agent.State = model.AgentAsking
			agent.Question = &model.AgentQuestion{Kind: "command", Command: "git push origin fix-docs-link", Reason: "Pushing sends the branch to GitHub, where others can see it.", Rule: "push branches to GitHub", HasRule: true}
		}},
		{"choices", func(agent *model.AgentRun) {
			agent.State = model.AgentAsking
			agent.Question = &model.AgentQuestion{Kind: "choices", Text: "Bash command\n  rm -rf node_modules && bun install\nDo you want to proceed?", Choices: []string{"Yes", "Yes, and don't ask again for rm commands", "No, and tell Claude what to do differently"}}
		}},
		{"done", func(agent *model.AgentRun) { agent.State = model.AgentIdle }},
		{"failed", func(agent *model.AgentRun) {
			agent.State, agent.Outcome = model.AgentFailed, "Claude Code exited with code 1"
		}},
	}
	for _, state := range states {
		set(state.change)
		settle(tt)
		renderBoth(t, tt, "agent-card-"+state.name)
	}
	set(func(agent *model.AgentRun) {})
	settle(tt)
	if !tt.HasText(base.Task) {
		t.Fatalf("no task in %q", tt.Texts())
	}
	if err := tt.Click(base.Task); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	renderBoth(t, tt, "agent-transcript")
	if !tt.HasText(model.MockAgentTranscript) {
		t.Errorf("the transcript is not shown: %q", tt.Texts())
	}
}
