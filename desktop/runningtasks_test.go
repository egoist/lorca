package main

import (
	"slices"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// runningTasksFixture is two of Patch's commands running in its chat, past the task delay: an
// install the bot's call waits on, and a dev server in the background.
func runningTasksFixture(m *mainWindow) *ui.Tester {
	m.selectChat("chat-patch")
	command := func(id, description, command string, foreground bool) {
		run := &model.CommandRun{SessionID: "s-" + id, Command: command, State: model.CommandRunning, Output: "ready", Device: "Studio", Background: !foreground}
		store.Append(&model.Message{
			ID:        id,
			Author:    model.BotAuthor("bot-patch"),
			Body:      model.Body{Kind: model.BodyTool, Tool: &model.ToolInvocation{Name: "bash", Summary: "$ " + command, Description: description, IsRunning: foreground, Run: run}},
			State:     model.MessageState{Kind: model.StateStreaming},
			CreatedAt: time.Now(),
		}, "chat-patch")
	}
	command("msg-install", "Install dependencies", "npm install\nnpm test", true)
	command("msg-server", "Start the dev server", "npm run dev", false)
	time.Sleep(model.TaskDelay + 100*time.Millisecond)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	tt.SetScale(2)
	settle(tt)
	return tt
}

// openRunningTasks clicks the header's button and returns the flyout it opened.
func openRunningTasks(t *testing.T, tt *ui.Tester, flyouts *[]*testFlyout, count int) *testFlyout {
	t.Helper()
	opened := len(*flyouts)
	if err := tt.Click(L("Running tasks (%d)", count)); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if len(*flyouts) != opened+1 {
		t.Fatalf("%d flyouts open", len(*flyouts)-opened)
	}
	f := (*flyouts)[len(*flyouts)-1]
	settleFlyout(f)
	return f
}

func TestRunningTasksFlyout(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	tt := runningTasksFixture(m)
	f := openRunningTasks(t, tt, flyouts, 2)
	ft := f.tt
	if !m.tasks.open || m.tasks.flyout != f {
		t.Fatal("the flyout is not the state's")
	}
	for _, text := range []string{"Install dependencies", "Start the dev server", "$ npm install", "$ npm run dev", L("Run in Background"), L("Stop")} {
		if !ft.HasText(text) {
			t.Errorf("no %q in %q", text, ft.Texts())
		}
	}
	renderBoth(t, ft, "running-tasks-flyout")

	// Run in Background sends the install there: the bot's call returns, the command runs on.
	if err := ft.Click(L("Run in Background")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	settleFlyout(f)
	install := store.Chat("chat-patch").Messages[slices.IndexFunc(store.Chat("chat-patch").Messages, func(message *model.Message) bool { return message.ID == "msg-install" })]
	if model.RunsInForeground(install) || ft.HasText(L("Run in Background")) {
		t.Fatalf("the install stayed in the foreground: %q", ft.Texts())
	}

	// A command stopped from the list stays in it, saying so, while the button counts the rest.
	if err := ft.Click(L("Stop")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	settleFlyout(f)
	if !ft.HasText("Install dependencies") || !ft.HasText(L("Stopped")) {
		t.Fatalf("the stopped command left the list: %q", ft.Texts())
	}
	if _, ok := tt.Find(L("Running tasks (%d)", 1)); !ok {
		t.Fatal("the button does not count the one left")
	}
	if err := ft.Click(L("Stop")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	settleFlyout(f)
	if !ft.HasText("Install dependencies") || !ft.HasText("Start the dev server") || f.isShut {
		t.Fatalf("ended commands left the open list: %q", ft.Texts())
	}

	// The button, still shown while the flyout is open, closes it; with nothing running it goes.
	if err := tt.Click(L("Running tasks (%d)", 0)); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.tasks.open || m.tasks.flyout != nil || !f.isShut {
		t.Fatalf("open %v, shut %v", m.tasks.open, f.isShut)
	}
	if _, ok := tt.Find(L("Running tasks (%d)", 0)); ok {
		t.Fatal("the button stayed with nothing running")
	}
}

// The flyout opens at its list's height, measured before it shows, and follows the list after.
func TestRunningTasksFlyoutHeight(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	tt := runningTasksFixture(m)
	if err := tt.Click(L("Running tasks (%d)", 2)); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	f := (*flyouts)[0]
	opened := f.height
	settleFlyout(f)
	if opened < 150 || f.height != opened {
		t.Fatalf("opened at %d, fitted to %d", opened, f.height)
	}
	// A row's error grows it.
	m.tasks.errors["msg-server"] = "The Runner is offline."
	settleFlyout(f)
	if !f.tt.HasText("The Runner is offline.") || f.height <= opened {
		t.Fatalf("height %d after an error, %d before: %q", f.height, opened, f.tt.Texts())
	}
}

// Closing the flyout from outside (Escape, a click elsewhere) resets it: opened again, it lists only
// what runs then.
func TestRunningTasksFlyoutClosedOutside(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	tt := runningTasksFixture(m)
	f := openRunningTasks(t, tt, flyouts, 2)
	if err := f.tt.Click(L("Stop")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	f.Close()
	settle(tt)
	if m.tasks.open || m.tasks.flyout != nil {
		t.Fatal("closing the flyout left it open")
	}
	f = openRunningTasks(t, tt, flyouts, 1)
	if f.tt.HasText("Install dependencies") || !f.tt.HasText("Start the dev server") {
		t.Fatalf("reopened: %q", f.tt.Texts())
	}

	// Another chat closes it.
	m.selectChat("chat-relay")
	settle(tt)
	if !f.isShut || m.tasks.open {
		t.Fatal("the flyout stayed open over another chat")
	}
}

// A click on a command closes the flyout and shows all of it in a sheet.
func TestRunningTasksFlyoutShowsTheCommand(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	tt := runningTasksFixture(m)
	f := openRunningTasks(t, tt, flyouts, 2)
	if err := f.tt.Click("$ npm install"); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if !f.isShut || m.tasks.open || !m.hasSheet() || !tt.HasText("npm test") {
		t.Fatalf("shut %v, sheet %v: %q", f.isShut, m.hasSheet(), tt.Texts())
	}
}

// Once the list has nothing to show, its rows gone, the flyout closes.
func TestRunningTasksFlyoutClosesEmpty(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	tt := runningTasksFixture(m)
	f := openRunningTasks(t, tt, flyouts, 2)
	chat := store.Chat("chat-patch")
	chat.Messages = slices.DeleteFunc(chat.Messages, func(message *model.Message) bool {
		return message.ID == "msg-install" || message.ID == "msg-server"
	})
	settleFlyout(f)
	settle(tt)
	if !f.isShut || m.tasks.open {
		t.Fatal("the empty flyout stayed open")
	}
}

// One header flyout shows at a time: Attention closes Running tasks, and the other way round.
func TestRunningTasksAndAttentionFlyouts(t *testing.T) {
	m := demoWindow(t)
	flyouts := stubFlyouts(t)
	tt := runningTasksFixture(m)
	tasks := openRunningTasks(t, tt, flyouts, 2)
	m.run("attention")
	settle(tt)
	attention := (*flyouts)[len(*flyouts)-1]
	if !tasks.isShut || m.tasks.open || !m.attention.open || attention == tasks || attention.isShut {
		t.Fatalf("tasks shut %v, attention open %v", tasks.isShut, m.attention.open)
	}
	tasks = openRunningTasks(t, tt, flyouts, 2)
	if !attention.isShut || m.attention.open || !m.tasks.open || tasks.isShut {
		t.Fatalf("attention shut %v, tasks open %v", attention.isShut, m.tasks.open)
	}
}
