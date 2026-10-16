package main

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func decodeMessage(t *testing.T, raw string) *model.Message {
	t.Helper()
	var wire model.WireMessage
	if err := json.Unmarshal([]byte(raw), &wire); err != nil {
		t.Fatal(err)
	}
	return model.ToMessage(wire)
}

func TestTheAgentRowsDecodeFromTheCLI(t *testing.T) {
	demoWindow(t)
	start := decodeMessage(t, `{"id":"m1","chat_id":"c1","author":{"kind":"bot","bot_id":"b1"},"state":{"kind":"complete"},"created_at":1,
	 "body":{"kind":"tool","name":"coding_agent","summary":"Started","detail":"Add dark mode\n\nTo Settings.","is_running":false,
	  "agent":{"id":"agent-1","kind":"codex","title":"Settings: dark mode","task":"Add dark mode","folder":"~/x/dark","branch":"dark",
	   "state":"asking",
	   "pull_request":"https://github.com/acme/shop/pull/7","changes":{"files":4,"added":12,"removed":3},
	   "device":"Workbench","started_at":1792069627.1,"updated_at":1792069700.4}}}`)
	agent := model.AgentOf(start)
	if agent == nil || !start.Body.Tool.IsShown() || !start.Body.Tool.IsAgentMessage() {
		t.Fatalf("no agent shown: %+v", start.Body.Tool)
	}
	if agent.Name() != "Codex" || agent.Label() != "Settings: dark mode" || agent.State != model.AgentAsking ||
		agent.PullNumber() != "#7" || agent.Changes == nil || agent.Changes.Added != 12 || agent.UpdatedAt.Unix() != 1792069700 {
		t.Errorf("decoded %+v", agent)
	}
	// A message to it names it; the chat's start row has its title and kind.
	send := decodeMessage(t, `{"id":"m2","chat_id":"c1","author":{"kind":"bot","bot_id":"b1"},"state":{"kind":"complete"},"created_at":2,
	 "body":{"kind":"tool","name":"coding_agent","summary":"Sent","detail":"Also the Chinese page.","is_running":false,"target_agent_id":"agent-1"}}`)
	chat := &model.Chat{ID: "c1", Messages: []*model.Message{start, send}}
	if !send.Body.Tool.IsShown() || model.AgentLabel(chat, send.Body.Tool) != "Settings: dark mode" || model.AgentRowOf(chat, "agent-1") != start {
		t.Errorf("the message names no agent: %+v", send.Body.Tool)
	}
	said := decodeMessage(t, `{"id":"m3","chat_id":"c1","author":{"kind":"bot","bot_id":"b1"},"state":{"kind":"complete"},"created_at":3,
	 "body":{"kind":"agent_message","agent_id":"agent-1","agent_kind":"codex","title":"Settings: dark mode","text":"Done.\nAll tests pass.","failed":true}}`)
	if said.Body.Kind != model.BodyAgentMessage || said.Body.AgentMessage.Text != "Done.\nAll tests pass." || !said.Body.AgentMessage.Failed {
		t.Errorf("agent message %+v", said.Body.AgentMessage)
	}
	// While the start runs, its detail is the call's arguments: the marker reads its task.
	start.Body.Tool.IsRunning = true
	if got := start.Body.Tool.AgentText(); got != "Add dark mode" {
		t.Errorf("running start reads %q", got)
	}
}

func TestTheAgentSaysHowItStands(t *testing.T) {
	demoWindow(t)
	agent := &model.AgentRun{ID: "agent-1", Kind: "claude", Title: "Fix it", Branch: "fix-it", State: model.AgentWorking}
	cases := []struct {
		state  model.AgentState
		pr     string
		detail string
	}{
		{model.AgentStarting, "", "Working · fix-it"},
		{model.AgentWorking, "", "Working · fix-it"},
		{model.AgentAsking, "", "Needs attention · fix-it"},
		{model.AgentIdle, "", "Finished · fix-it"},
		{model.AgentIdle, "https://github.com/acme/shop/pull/7", "Ready for review · #7"},
		{model.AgentFailed, "", "Run failed · fix-it"},
		{model.AgentStopped, "", "Stopped · fix-it"},
	}
	for _, each := range cases {
		agent.State, agent.PullRequest = each.state, each.pr
		if got := agentRowDetail(agent); got != each.detail {
			t.Errorf("%s: %q", each.state, got)
		}
	}
	agent.State = model.AgentIdle
	if agent.IsRunning() || !agent.IsOpen() {
		t.Error("a finished agent offers no Stop and takes a follow-up")
	}
	if (&model.AgentRun{State: model.AgentFailed}).Started() {
		t.Error("a start that failed has no page")
	}
}

// Details lists what works or asks first, then the latest change, and leaves out what ended days
// ago and starts that failed.
func TestTheChatsCodingAgents(t *testing.T) {
	now := time.Now()
	row := func(id string, state model.AgentState, updated time.Duration) *model.Message {
		return &model.Message{ID: id, Body: model.Body{Kind: model.BodyTool, Tool: &model.ToolInvocation{Name: "coding_agent",
			Agent: &model.AgentRun{ID: "agent-" + id, State: state, UpdatedAt: now.Add(-updated)}}}}
	}
	failed := row("never", model.AgentFailed, time.Minute)
	failed.Body.Tool.Agent.ID = ""
	chat := &model.Chat{Messages: []*model.Message{
		row("old", model.AgentIdle, 4*24*time.Hour),
		row("done", model.AgentIdle, time.Hour),
		row("working", model.AgentWorking, 3*time.Hour),
		row("stopped", model.AgentStopped, 10*time.Minute),
		row("asking", model.AgentAsking, 2*time.Minute),
		row("old-working", model.AgentWorking, 5*24*time.Hour),
		failed,
	}}
	var got []string
	for _, message := range model.CodingAgentsIn(chat, now) {
		got = append(got, message.ID)
	}
	want := []string{"asking", "working", "old-working", "stopped", "done"}
	if len(got) != len(want) {
		t.Fatalf("listed %q", got)
	}
	for i := range want {
		if got[i] != want[i] {
			t.Fatalf("listed %q, want %q", got, want)
		}
	}
}

func TestTheTranscriptFoldsReadsAndSearches(t *testing.T) {
	demoWindow(t)
	steps := agentSteps(model.MockAgentTranscript())
	var lines []string
	for _, step := range steps {
		switch {
		case step.explored != nil:
			lines = append(lines, exploredText(step.explored))
		case step.isTool():
			lines = append(lines, agentToolText(step.item.Tool))
		default:
			lines = append(lines, step.item.Kind)
		}
	}
	want := []string{"prompt", "status", "thought", "text", "Explored 3 files 2 searches", "plan", "Edited src/pages/download.astro",
		"Fetched https://lorca.app/docs/cli/install", "Not allowed: git push origin fix-docs-link", "Edited tests/links.test.ts",
		"Ran bun test links", "error", "text", "Used github: create_issue", "Running bun test links"}
	if len(lines) != len(want) {
		t.Fatalf("steps %q", lines)
	}
	for i := range want {
		if lines[i] != want[i] {
			t.Errorf("step %d: %q, want %q", i, lines[i], want[i])
		}
	}
	read := func(status string) model.AgentItem {
		return model.AgentItem{Kind: "tool", Tool: &model.AgentToolStep{Kind: "read", Title: "a.rs", Status: status}}
	}
	if got := exploredText([]model.AgentItem{read("done")}); got != "Explored 1 file" {
		t.Errorf("one file: %q", got)
	}
	if got := exploredText([]model.AgentItem{read("done"), read("running")}); got != "Exploring 2 files" {
		t.Errorf("running: %q", got)
	}
}

// TestRenderCodingAgents draws the developer's chat with its agents' markers, the card the asking
// agent's question ends the chat with, and the Coding section; answers the card; opens an agent's
// page and a fold in it.
func TestRenderCodingAgents(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	chat := store.Chat("chat-patch")
	tt := ui.NewTester(m.frame(m.view), 1280, 860)
	settle(tt)
	time.Sleep(300 * time.Millisecond)
	settle(tt)
	asking := model.AgentRowOf(chat, "agent-b85e4f10")
	if asking == nil {
		t.Fatal("the demo has no asking agent")
	}
	working := model.AgentOf(model.AgentRowOf(chat, "agent-3f9a2c1d"))
	for _, text := range []string{
		"Developer wants to run a command on Studio", "git push -u origin dark-mode-toggle", "Always allow",
		"Codex, the coding agent on “Settings: add a dark mode toggle”, asks to run this. Pushing sends the branch to GitHub, where others can see it.",
		agentMarkerText(L("Messaged"), "Settings: add a dark mode toggle", model.AgentOf(asking).Task),
		agentMarkerText(L("Messaged"), working.Title, "Fix the same link on the Chinese download page too, and cover it in the test."),
		"CODING", "Needs attention · dark-mode-toggle", "Working · fix-docs-link", "Ready for review · #7",
	} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "coding-chat")
	// The inspector's section on its own, with the transcript scrolled up to the finished agent.
	for i, row := range m.chat.rows {
		if row.kind == rowMessage && row.message.Body.Kind == model.BodyAgentMessage {
			m.chat.list.ScrollTo(i, ui.Center)
		}
	}
	settle(tt)
	time.Sleep(300 * time.Millisecond)
	settle(tt)
	if !tt.HasText(agentMarkerText(L("Message from"), "Docs: use mygo.json in the launcher examples", "Updated the four launcher examples to read mygo.json, in English and Chinese. The docs build passes.")) {
		t.Errorf("no agent message in %q", tt.Texts())
	}
	renderBoth(t, tt, "coding-chat-top")
	m.chat.list.ScrollToEnd()
	settle(tt)
	time.Sleep(300 * time.Millisecond)
	settle(tt)

	// The asking agent's page says it needs attention.
	m.presentAgentPage(chat.ID, asking.ID)
	settleTransitions(tt)
	if !tt.HasText("Codex · Needs attention · dark-mode-toggle · Studio") {
		t.Errorf("no asking page: %q", tt.Texts())
	}
	renderBoth(t, tt, "coding-page-asking")
	for m.hasSheet() {
		m.sheets[len(m.sheets)-1].dismiss()
	}
	settleTransitions(tt)

	if err := tt.Click(L("Allow once")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if card := chat.Messages[len(chat.Messages)-1]; card.Body.Kind != model.BodyPermission || card.Body.Request.Decision != model.DecisionAllowed || tt.HasText(L("Allow once")) {
		t.Errorf("the answer left %+v", card.Body.Request)
	}

	// The working agent's row in Details opens its page.
	if err := tt.Click(working.Title); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if !m.hasSheet() {
		t.Fatal("no agent page")
	}
	for _, text := range []string{"Claude Code · Working · fix-docs-link · Studio", "Prompt", "Show more", "Working in ~/Projects/site on main",
		"Thought for 3s", "Explored 3 files 2 searches", "Point the link at /docs/cli/install", "Not allowed: git push origin fix-docs-link",
		"The link test failed: the Chinese page still links to /docs/install.", "Running bun test links", "Stop", "fix-docs-link"} {
		if !tt.HasText(text) {
			t.Errorf("no %q on the page: %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "coding-page")
	// Its start: the brief, folded past six lines.
	if r, ok := tt.Find("Thought for 3s"); ok {
		for range 10 {
			tt.Scroll(r.X+r.W/2, r.Y+r.H/2, 0, -400)
		}
	}
	settle(tt)
	renderBoth(t, tt, "coding-page-top")
	if err := tt.Click("Explored 3 files 2 searches"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText("Read src/pages/download.astro") || !tt.HasText("Searched /docs/install") {
		t.Errorf("the fold did not open: %q", tt.Texts())
	}
	if err := tt.Click("Ran bun test links"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	renderBoth(t, tt, "coding-page-open")
	if err := tt.Click(L("Stop")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if _, shown := tt.Find(L("Stop")); model.AgentOf(model.AgentRowOf(chat, "agent-3f9a2c1d")).State != model.AgentStopped || shown {
		t.Errorf("Stop left %+v", model.AgentOf(model.AgentRowOf(chat, "agent-3f9a2c1d")))
	}
	if err := tt.Click(L("Done")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if m.hasSheet() {
		t.Error("Done left the page up")
	}

	// The finished agent's page has its pull request and what it changed.
	finished := model.AgentRowOf(chat, "agent-7d21e0b4")
	m.presentAgentPage(chat.ID, finished.ID)
	settleTransitions(tt)
	for _, text := range []string{"#7", "+12 −3 in 4 files", "docs-use-mygo-json", "Claude Code · Ready for review · docs-use-mygo-json · Studio"} {
		if !tt.HasText(text) {
			t.Errorf("no %q on the finished page: %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "coding-page-finished")
}

func TestAnAgentNotifiesWhenItAsksAndWhenItReports(t *testing.T) {
	demoWindow(t)
	chat := store.Chat("chat-patch")
	// Its card asks and notifies; its row, which may be far up the chat, does not.
	card := chat.Messages[len(chat.Messages)-1]
	notification := notificationFor(card)
	if notification == nil || notification.kind != notifyPermission || notification.body != L("Confirmation needed: %@", "$ git push -u origin dark-mode-toggle") {
		t.Fatalf("notification %+v", notification)
	}
	if !asks(card) {
		t.Error("the asking agent's card is not a question")
	}
	if row := model.AgentRowOf(chat, "agent-b85e4f10"); asks(row) || notificationFor(row) != nil {
		t.Error("the asking agent's row asks too")
	}
	// An agent's report to the user notifies as a reply does; why one ended is its bot's to tell.
	report := &model.Message{Author: card.Author, Body: model.Body{Kind: model.BodyAgentMessage,
		AgentMessage: &model.AgentMessage{AgentKind: "claude", Title: "Login", Text: "Fixed it.\nhttps://github.com/acme/shop/pull/7"}}}
	if notification := notificationFor(report); notification == nil || notification.kind != notifyReply || notification.body != L("Message from %@: %@", "Login", "Fixed it.\nhttps://github.com/acme/shop/pull/7") {
		t.Errorf("report notification %+v", notification)
	}
	report.Body.AgentMessage.Failed = true
	if notificationFor(report) != nil {
		t.Error("an agent that ended on its own notifies")
	}
}

func TestTheChatPreviewNamesTheAgent(t *testing.T) {
	demoWindow(t)
	developer := store.Chat("chat-patch")
	start := model.AgentRowOf(developer, "agent-b85e4f10")
	chat := &model.Chat{BotIDs: developer.BotIDs, Messages: []*model.Message{start}}
	want := L("Messaged %@: %@", "Settings: add a dark mode toggle", "Add a dark mode toggle to the Settings page, after the Appearance row. Follow the existing switch style, and push the branch when it builds.")
	if got := store.Preview(chat); got != want {
		t.Errorf("preview %q", got)
	}
	// A message to it reads its start's title.
	send := &model.Message{Author: start.Author, Body: model.Body{Kind: model.BodyTool, Tool: &model.ToolInvocation{Name: "coding_agent", Detail: "Also the docs.", TargetAgentID: "agent-b85e4f10"}}}
	chat.Messages = append(chat.Messages, send)
	if got := store.Preview(chat); got != L("Messaged %@: %@", "Settings: add a dark mode toggle", "Also the docs.") {
		t.Errorf("preview %q", got)
	}
	chat.Messages = append(chat.Messages, &model.Message{Author: start.Author, Body: model.Body{Kind: model.BodyAgentMessage,
		AgentMessage: &model.AgentMessage{AgentKind: "pi", Title: "Fix it", Text: "Fixed it."}}})
	if got := store.Preview(chat); got != L("Message from %@: %@", "Fix it", "Fixed it.") {
		t.Errorf("preview %q", got)
	}
}

func TestCodingAgentSettings(t *testing.T) {
	m, tt := settingsWindowTester(t, 900)
	m.showSettings(model.PaneCodingAgents)
	settle(tt)
	// The pane shows the picked Runner's agents, as that Runner keeps them.
	runner := store.Device(m.settingsDeviceID)
	if runner == nil || !runner.IsRunner() {
		t.Fatalf("no Runner picked: %q", m.settingsDeviceID)
	}
	for _, text := range []string{L("Coding Agents on %@", runner.Name), "Claude Code", "Codex", "OpenCode", "pi", "fx", "Installed · 2.3.1 (Claude Code)", "Not installed"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	agents := func() []model.InstalledCodingAgent { return m.settings.coding.list }
	renderBoth(t, tt, "coding-settings")
	// Codex's row dragged onto Claude Code's goes first; the line shows where while it drags.
	from, ok := tt.Find("Codex")
	to, ok2 := tt.Find("Claude Code")
	if !ok || !ok2 {
		t.Fatalf("no rows in %q", tt.Texts())
	}
	tt.Press(from.X+from.W/2, from.Y+from.H/2)
	for step := float32(1); step <= 6; step++ {
		tt.Move(from.X+from.W/2, from.Y+from.H/2+(to.Y-from.Y)*step/6)
		tt.Frame()
	}
	renderBoth(t, tt, "coding-settings-dragging")
	tt.Release(to.X+to.W/2, to.Y+to.H/2)
	settle(tt)
	if agents()[0].Kind != "codex" || agents()[1].Kind != "claude" {
		t.Errorf("order %+v", agents())
	}
	// The Runner keeps it: asked again, it answers the new order.
	var kept []model.InstalledCodingAgent
	store.CodingAgents(runner.ID, func(list []model.InstalledCodingAgent, _ error) { kept = list })
	settle(tt)
	if len(kept) == 0 || kept[0].Kind != "codex" {
		t.Errorf("the Runner kept %+v", kept)
	}
	if moved := movedCodingAgent(agents(), "claude", 4); moved[4].Kind != "claude" || moved[0].Kind != "codex" {
		t.Errorf("moved to the end: %+v", moved)
	}
	if agents()[2].Kind != "opencode" || agents()[2].Enabled {
		t.Fatalf("order %+v", agents())
	}
	if err := tt.Click(L("Turn %@ on", "OpenCode")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !agents()[2].Enabled {
		t.Errorf("the switch left OpenCode off: %+v", agents())
	}
	// The search finds an agent by its maker, and the pane by its name.
	var found []settingsEntry
	for _, match := range panesMatching("anthropic", nil) {
		if match.pane == model.PaneCodingAgents {
			found = match.entries
		}
	}
	if len(found) != 1 || found[0].title != "Claude Code" {
		t.Errorf("search %+v", found)
	}
	if matches := panesMatching("coding agents", nil); len(matches) == 0 || matches[0].pane != model.PaneCodingAgents {
		t.Errorf("search %+v", matches)
	}
}

// A start that failed before the agent had an id has no page: its marker shows why in a popover.
func TestAFailedStartShowsWhy(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	chat := store.Chat("chat-patch")
	row := model.AgentRowOf(chat, "agent-b85e4f10")
	store.Update(row.ID, chat.ID, func(message *model.Message) {
		tool := *message.Body.Tool
		agent := *tool.Agent
		agent.ID, agent.State, agent.Outcome = "", model.AgentFailed, "None of the coding agents the user turned on (Codex) is installed on this Runner."
		tool.Agent = &agent
		message.Body.Tool = &tool
	})
	tt := ui.NewTester(m.frame(m.view), 1280, 860)
	settle(tt)
	time.Sleep(300 * time.Millisecond)
	settle(tt)
	marker := agentMarkerText(L("Messaged"), "Settings: add a dark mode toggle", "Add a dark mode toggle to the Settings page, after the Appearance row. Follow the existing switch style, and push the branch when it builds.")
	if err := tt.Click(marker); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.hasSheet() || !tt.HasText("None of the coding agents the user turned on (Codex) is installed on this Runner.") {
		t.Errorf("the failed start shows no reason: %q", tt.Texts())
	}
	if len(model.CodingAgentsIn(chat, time.Now())) != 2 {
		t.Error("Details lists a start that failed")
	}
}
