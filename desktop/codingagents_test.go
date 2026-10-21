package main

import (
	"strings"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// codingAgentsFixture is the demo's three agents of Developer's in its chat, and one of
// Researcher's in a group, at work on the same repository.
func codingAgentsFixture(m *mainWindow) *ui.Tester {
	store.Append(&model.Message{
		ID:     "msg-scout-agent",
		Author: model.BotAuthor("bot-scout"),
		Body: model.Body{Kind: model.BodyTool, Tool: &model.ToolInvocation{Name: "coding_agent", Summary: "Started Codex", Agent: &model.AgentRun{
			ID: "agent-5c0ut000", Kind: "codex", Title: "Research: link checker", Task: "Find a link checker", Folder: "~/Projects/site", Branch: "link-checker",
			State: model.AgentWorking, Device: "Studio", DeviceID: "dev-studio", StartedAt: time.Now(), UpdatedAt: time.Now(),
		}}},
		CreatedAt: time.Now(),
	}, "chat-relay")
	m.selectChat("chat-relay")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	tt.SetScale(2)
	settle(tt)
	return tt
}

func TestCodingAgentsFlyout(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	tt := codingAgentsFixture(m)
	// The button's badge counts the account's agents at work now, whichever chat is on screen: four
	// are live, and the one done with a pull request waits for a follow-up.
	if !tt.HasText("3") {
		t.Fatalf("no badge: %q", tt.Texts())
	}
	if err := tt.Click(L("Coding agents (%d)", 3)); err != nil {
		t.Fatalf("%v: %q", err, tt.Texts())
	}
	settle(tt)
	if len(*flyouts) != 1 || !m.coding.open {
		t.Fatalf("%d flyouts, open %v", len(*flyouts), m.coding.open)
	}
	f := (*flyouts)[0]
	settleFlyout(f)
	ft := f.tt
	for _, text := range []string{
		"Research: link checker", "Researcher · " + store.Title(store.Chat("chat-relay")) + " · Studio",
		"Fix the broken docs link on the download page", "Developer · Studio",
		"Docs: use mygo.json in the launcher examples", "#7",
	} {
		if !ft.HasText(text) && !containsText(ft.Texts(), text) {
			t.Errorf("no %q in %q", text, ft.Texts())
		}
	}
	renderBoth(t, ft, "coding-agents-flyout")

	// A row closes the flyout and opens the agent's page.
	if err := ft.Click("Research: link checker"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.coding.open || !f.isShut || !m.hasSheet() {
		t.Fatalf("open %v, shut %v, sheet %v", m.coding.open, f.isShut, m.hasSheet())
	}
}

// With no live agent on the account, the button is gone.
func TestCodingAgentsButtonHidesWithNoLiveAgent(t *testing.T) {
	m := demoWindow(t)
	stubFlyouts(t)
	for _, chat := range store.Chats {
		for _, message := range chat.Messages {
			if agent := model.AgentOf(message); agent != nil {
				store.Update(message.ID, chat.ID, func(message *model.Message) {
					tool := *message.Body.Tool
					ended := *tool.Agent
					ended.State = model.AgentStopped
					tool.Agent = &ended
					message.Body.Tool = &tool
				})
			}
		}
	}
	m.selectChat("chat-patch")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	tt.SetScale(2)
	settle(tt)
	for _, count := range []int{0, 1, 2, 3} {
		if _, ok := tt.Find(L("Coding agents (%d)", count)); ok {
			t.Fatalf("the button shows with %d live", count)
		}
	}
}

// An agent whose chat this app has not loaded opens from the list's card, on its Runner.
func TestListedAgentOpensWithoutItsChat(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	tt.SetScale(2)
	settle(tt)
	m.presentListedAgent(model.ListedAgent{
		ChatID: "chat-elsewhere", ChatTitle: "Elsewhere", MessageID: "msg-elsewhere", BotID: "bot-ember", BotName: "DevOps",
		Agent: &model.AgentRun{ID: "agent-e15e0000", Kind: "claude", Title: "Deploy the relay", Folder: "~/infra", State: model.AgentWorking, Device: "Closet", DeviceID: "dev-closet"},
	})
	settle(tt)
	if !m.hasSheet() || !tt.HasText("Deploy the relay") || !containsText(tt.Texts(), "Closet") {
		t.Fatalf("sheet %v: %q", m.hasSheet(), tt.Texts())
	}
}

func containsText(texts []string, part string) bool {
	for _, text := range texts {
		if strings.Contains(text, part) {
			return true
		}
	}
	return false
}
