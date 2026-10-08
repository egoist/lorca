package main

import (
	"fmt"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A chat's running tasks, after the macOS app's RunningTasksViewController: the commands its bots
// have running in their terminals, each with what it does in the bot's words, who runs it and for
// how long, the command (a click shows all of it), its last lines, and Stop, with Run in Background
// while the bot's call still waits on it. One that ends while the list is open stays, saying how it
// ended, until the list closes.

type runningTask struct {
	id           string
	title        string
	botName      string
	showsBotName bool
	command      string
	output       string
	state        model.CommandState
	// foreground is the bot's call still waiting on it: Run in Background sends it there.
	foreground bool
	startedAt  time.Time
}

func (t runningTask) isLive() bool {
	return t.state == model.CommandRunning || t.state == model.CommandWaiting
}

// status is "Scout · Running · 1:05", or how it ended: "Finished".
func (t runningTask) status(now time.Time) string {
	var parts []string
	if t.showsBotName {
		parts = append(parts, t.botName)
	}
	switch t.state {
	case model.CommandRunning:
		parts = append(parts, L("Running"), model.Elapsed(t.startedAt, now))
	case model.CommandWaiting:
		parts = append(parts, L("Waiting for input"), model.Elapsed(t.startedAt, now))
	case model.CommandExited:
		parts = append(parts, L("Finished"))
	case model.CommandFailed:
		parts = append(parts, L("Failed"))
	default:
		parts = append(parts, L("Stopped"))
	}
	return strings.Join(parts, " · ")
}

// runningTasksState is the popover's own while it is open: the commands it has listed, so one that
// ends stays, and what went wrong with Stop or Run in Background, by command.
type runningTasksState struct {
	open    bool
	chatID  string
	listed  map[string]bool
	errors  map[string]string
	busy    map[string]bool
	hadAny  bool
	wasOpen bool
}

// runningTasks are the chat's tasks for the popover: the commands running for long enough, and
// those listed since it opened that have ended.
func (s *runningTasksState) tasks(chatID string) []runningTask {
	chat := store.Chat(chatID)
	if chat == nil {
		return nil
	}
	running := map[string]bool{}
	for _, message := range store.RunningCommands(chatID) {
		running[message.ID] = true
	}
	var list []runningTask
	for _, message := range chat.Messages {
		run := model.CommandRunOf(message)
		if run == nil || !(running[message.ID] || s.listed[message.ID] && run.HasEnded()) {
			continue
		}
		s.listed[message.ID] = true
		title := message.Body.Tool.Description
		if title == "" {
			title = model.FirstLine(run.Command)
		}
		list = append(list, runningTask{
			id:           message.ID,
			title:        title,
			botName:      botName(message),
			showsBotName: chat.IsGroup(),
			command:      run.Command,
			output:       run.Output,
			state:        run.State,
			foreground:   model.RunsInForeground(message),
			startedAt:    message.CreatedAt,
		})
	}
	return list
}

// runningTasksButton is the chat's running commands in the header: shown while it has any, or
// while their popover is open, with how many as a badge once there is more than one.
func (m *mainWindow) runningTasksButton(c *ui.Context, chatID string) {
	p := colors(c)
	s := &m.tasks
	if s.chatID != chatID {
		// The popover belongs to the chat on screen.
		*s = runningTasksState{chatID: chatID}
	}
	count := 0
	if chatID != "" {
		count = len(store.RunningCommands(chatID))
	}
	if count == 0 && !s.open {
		return
	}
	holder := ui.Box(c)
	var button ui.Element
	holder.Children(func() {
		button = hoverButton(c, hoverButtonOptions{Symbol: "terminal", Tooltip: L("Running tasks"), Label: L("Running tasks (%d)", count), Active: s.open})
		if count > 1 {
			ui.Row(c).Absolute().Top(-2).Right(-4).MinWidth(15).Height(15).Padding(0, 4).Radius(8).Justify(ui.Center).Background(p.Red).PassThrough().Children(func() {
				ui.Text(c, fmt.Sprint(count)).FontSize(10).FontWeight(600).TextColor(p.White).FontFeatures("tnum")
			})
		}
	})
	if button.Clicked() && (count > 0 || s.open) {
		s.open = !s.open
	}
	if s.open && !s.wasOpen {
		s.listed, s.errors, s.busy, s.hadAny = map[string]bool{}, map[string]string{}, map[string]bool{}, false
	}
	s.wasOpen = s.open
	ui.PopoverBase(c, button, &s.open, func(panel ui.Element) {
		popoverPanel(c, panel).Width(420)
		m.runningTasksList(c, chatID)
	})
}

func (m *mainWindow) runningTasksList(c *ui.Context, chatID string) {
	p := colors(c)
	s := &m.tasks
	tasks := s.tasks(chatID)
	// Nothing left to show: the commands' rows are gone.
	if len(tasks) > 0 {
		s.hadAny = true
	} else if s.hadAny {
		s.open = false
		return
	}
	now := c.Now()
	c.After(time.Second)
	// Stop, or Run in Background: a failure says why under the row while the command runs.
	perform := func(task runningTask, action func(chatID, messageID string, done func(error))) {
		if s.busy[task.id] {
			return
		}
		s.busy[task.id] = true
		id := task.id
		action(chatID, id, func(err error) {
			s.busy[id] = false
			if err != nil {
				s.errors[id] = model.ErrorText(err)
			} else {
				delete(s.errors, id)
			}
		})
	}
	ui.Scroll(c).MaxHeight(480).Padding(4, 6).Children(func() {
		for i, task := range tasks {
			row := ui.Row(c.Key(task.id)).AlignItems(ui.Start).Gap(10).Padding(10, 8)
			if i > 0 {
				line := p.Separator
				row.DrawOver(func(painter *ui.Painter, r ui.Rect) {
					painter.Fill(ui.Rect{X: r.X + 8, Y: r.Y, W: r.W - 16, H: 1}, line, 0)
				})
			}
			row.Children(func() {
				ui.Row(c).Padding(1, 0, 0, 0).TextColor(p.Accent).Children(func() { symbol(c, "terminal", 17, 1.9) })
				ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
					ui.Row(c).Gap(8).Children(func() {
						ui.Text(c, task.title).Grow(1).Shrink(1).MinWidth(0).FontSize(12.5).FontWeight(600).SingleLine().Tooltip(task.title)
						if task.isLive() && task.foreground {
							if pushButton(c, L("Run in Background"), pushOptions{Small: true, Disabled: s.busy[task.id]}).Clicked() {
								perform(task, store.SendCommandToBackground)
							}
						}
						if task.isLive() {
							if pushButton(c, L("Stop"), pushOptions{Small: true, Disabled: s.busy[task.id]}).Clicked() {
								perform(task, store.StopCommand)
							}
						}
					})
					ui.Text(c, task.status(now)).FontSize(11).TextColor(p.Label2)
					if commandBlock(c, "$ "+model.FirstLine(task.command), 1) {
						s.open = false
						m.presentCommandSheet(L("%@'s command", task.botName), task.command)
					}
					if task.output != "" {
						commandOutput(c, task.output, 10)
					}
					// A failed Stop or Run in Background says so while the command runs; once it
					// ends, its state says the rest.
					if text := s.errors[task.id]; task.isLive() && text != "" {
						ui.Text(c, text).Margin(6, 0, 0, 0).FontSize(11).TextColor(p.Red)
					}
				})
			})
		}
	})
}
