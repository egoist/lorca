package main

import (
	"strconv"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A coding agent's card, after the macOS app's AgentCellView: on the row of the `coding_agent`
// call that started it, from Auto-review's question to how it ended. The agent's name with Stop
// while it runs, what it was asked, how it stands and where it works, and while it works its last
// lines. When it asks: Allow once, Always allow, and Deny before it starts or runs a command; a
// pop-up of the choices its pane offers; or a field for an answer to type. A click on what it
// works on opens its transcript.

// agentTitle is "Chef wants to start Claude Code on Workbench", "Claude Code wants to run a
// command", "Claude Code asks", or the agent's name.
func agentTitle(agent *model.AgentRun, botName string) string {
	if q := agent.Question; q != nil {
		switch q.Kind {
		case "start":
			if agent.Device != "" {
				return L("%@ wants to start %@ on %@", botName, agent.Name(), agent.Device)
			}
			return L("%@ wants to start %@", botName, agent.Name())
		case "command":
			return L("%@ wants to run a command", agent.Name())
		default:
			return L("%@ asks", agent.Name())
		}
	}
	return agent.Name()
}

// agentDetail is the line under the task: how it stands and where it works, or how it ended.
func agentDetail(agent *model.AgentRun) string {
	if agent.Question != nil {
		return ""
	}
	parts := []string{agent.Status()}
	if agent.State == model.AgentFailed && agent.Outcome != "" {
		parts = []string{L("Failed: %@", agent.Outcome)}
	}
	if place := agent.Place(); place != "" && agent.State != model.AgentDenied {
		parts = append(parts, place)
	}
	if host := agent.HostName(); host != "" && agent.IsOpen() {
		parts = append(parts, L("in %@", host))
	}
	return strings.Join(parts, " · ")
}

// agentOutput is its last lines with something on them, while it works.
func agentOutput(agent *model.AgentRun) string {
	if agent.Question != nil || (agent.State != model.AgentStarting && agent.State != model.AgentWorking) {
		return ""
	}
	return outputText(agent.Output)
}

func agentNote(agent *model.AgentRun) string {
	q := agent.Question
	switch {
	case q == nil:
		return ""
	case q.IsPermission() && q.HasRule:
		return L("Always allow adds the rule “%@”.", q.Rule)
	case q.Kind == "text":
		return L("What you type goes to %@, not into the chat.", agent.Name())
	}
	return ""
}

func agentSpokenText(agent *model.AgentRun, botName string) string {
	parts := []string{agentTitle(agent, botName), agent.Task}
	for _, part := range []string{agentDetail(agent), agentOutput(agent)} {
		if part != "" {
			parts = append(parts, part)
		}
	}
	if q := agent.Question; q != nil {
		parts = append(parts, firstNonEmpty(q.Command, q.Text))
	}
	return strings.Join(parts, ": ")
}

// agentCardState is a card's own: the choice picked, the answer being typed, a request under way,
// and what went wrong, until the question changes.
type agentCardState struct {
	choice string
	answer string
	busy   bool
	err    string
	seen   *model.AgentQuestion
}

func (m *mainWindow) agentCard(c *ui.Context, chat *model.Chat, message *model.Message, cardAvatar *avatarContent, groupStart bool) {
	p := colors(c)
	agent := message.Body.Tool.Agent
	name := botName(message)
	chatID, messageID := chat.ID, message.ID
	holder := ui.Box(c.Key("agent:" + message.ID))
	st := ui.Local(holder, "card", func() agentCardState { return agentCardState{} })
	// Another question starts afresh, on the choice the pane's cursor is on.
	if st.seen != agent.Question {
		st.seen, st.err, st.answer, st.choice = agent.Question, "", "", "0"
	}
	run := func(work func(done func(error)), failure string) {
		if st.busy {
			return
		}
		st.busy, st.err = true, ""
		work(func(err error) {
			st.busy = false
			if err == nil {
				return
			}
			if agentNote(agent) != "" {
				st.err = model.ErrorText(err)
			} else {
				m.showAlert(alertOptions{Message: failure, Informative: model.ErrorText(err)}, nil)
			}
		})
	}
	holder.Children(func() {
		cardFrame(c, cardAvatar, groupStart, agentSpokenText(agent, name), func() {
			ui.Row(c).Size(18, 18).Center().TextColor(p.Accent).Children(func() { symbol(c, "chevron.left.forwardslash.chevron.right", 16, 2) })
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
				ui.Row(c).Gap(8).MinWidth(0).Children(func() {
					ui.Text(c, agentTitle(agent, name)).Grow(1).Shrink(1).MinWidth(0).FontSize(12.5).FontWeight(600).FixedLineHeight(17).SingleLine()
					if agent.IsRunning() && pushButton(c, L("Stop"), pushOptions{Small: true, Disabled: st.busy}).Clicked() {
						run(func(done func(error)) { store.StopAgent(chatID, messageID, done) }, L("Could not stop %@", agent.Name()))
					}
				})
				// What it works on and how it stands; a click opens its transcript.
				body := func() {
					if agent.Task != "" {
						ui.Text(c, agent.Task).Margin(3, 0, 0, 0).FontSize(12).LineHeight(1.35).MaxLines(2).Tooltip(agent.Task)
					}
					if detail := agentDetail(agent); detail != "" {
						ui.Text(c, detail).Margin(3, 0, 0, 0).FontSize(11).TextColor(p.Label2).SingleLine().Tooltip(agent.Outcome)
					}
					if text := agentOutput(agent); text != "" {
						commandOutput(c, text, 6).Margin(8, 0, 0, 0)
					}
				}
				if agent.Started() {
					b := ui.ButtonBase(c).Column().AlignItems(ui.Stretch).FillWidth().Cursor(ui.CursorPointer).Label(L("Show the transcript")).TextColor(p.Label)
					b.Children(body)
					if b.Clicked() {
						m.presentAgentTranscript(chatID, message, agent)
					}
				} else {
					ui.Column(c).FillWidth().Children(body)
				}
				q := agent.Question
				if q != nil && q.Kind == "command" {
					if commandBlock(c, "$ "+model.FirstLine(q.Command), 1) {
						m.presentCommandSheet(L("%@'s command", agent.Name()), q.Command)
					}
				}
				if q != nil && (q.Kind == "choices" || q.Kind == "text") && q.Text != "" {
					commandOutput(c, q.Text, 8).Margin(8, 0, 0, 0)
				}
				if q != nil && q.IsPermission() && q.Reason != "" {
					ui.Text(c, q.Reason).Margin(7, 0, 0, 0).FontSize(11.5).LineHeight(1.35).TextColor(p.Label2).MaxLines(4)
				}
				switch {
				case q != nil && q.IsPermission():
					ui.Row(c).Wrap().Gap(6).Margin(10, 0, 0, 0).Children(func() {
						for _, choice := range q.Decisions() {
							if pushButton(c.Key(choice.Decision), choice.Title, pushOptions{Small: true}).Clicked() {
								store.AnswerPermission(chatID, messageID, choice.Decision)
							}
						}
					})
				case q != nil && q.Kind == "choices":
					ui.Row(c).Gap(6).Margin(10, 0, 0, 0).Children(func() {
						options := make([]popUpOption, len(q.Choices))
						for i, choice := range q.Choices {
							options[i] = popUpOption{Value: strconv.Itoa(i), Label: choice}
						}
						ui.Box(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
							if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: st.choice, Style: popUpBordered, Label: L("Answer"), Disabled: st.busy}); changed {
								st.choice = picked
							}
						})
						if pushButton(c, L("Answer"), pushOptions{Small: true, Disabled: st.busy}).Clicked() {
							choice, _ := strconv.Atoi(st.choice)
							run(func(done func(error)) { store.AnswerAgentChoice(chatID, messageID, choice, done) }, L("Could not answer %@", agent.Name()))
						}
					})
				case q != nil && q.Kind == "text":
					ui.Row(c).Gap(6).Margin(10, 0, 0, 0).Children(func() {
						send := func() {
							text := st.answer
							run(func(done func(error)) {
								store.AnswerAgentText(chatID, messageID, text, func(err error) {
									if err == nil {
										st.answer = ""
									}
									done(err)
								})
							}, L("Could not answer %@", agent.Name()))
						}
						field := textField(c, &st.answer, fieldOptions{Placeholder: L("Type your answer"), Label: firstNonEmpty(q.Text, L("Type your answer")), Disabled: st.busy}).Grow(1).Shrink(1).MinWidth(0)
						if field.Submitted() {
							send()
						}
						if pushButton(c, L("Send"), pushOptions{Small: true, Disabled: st.busy}).Clicked() {
							send()
						}
					})
				}
				if note := firstNonEmpty(st.err, agentNote(agent)); note != "" {
					t := ui.Text(c, note).Margin(8, 0, 0, 0).FontSize(11).LineHeight(1.35).TextColor(p.Label2)
					if st.err != "" {
						t.TextColor(p.Red).Tooltip(st.err)
					}
				}
			})
		})
	})
}

// MARK: - The transcript

// agentTranscript follows a coding agent's transcript while its sheet is up, asking its Runner
// again every two seconds, five through the relay.
type agentTranscript struct {
	chatID, messageID string
	remote            bool
	text              string
	err               string
	loaded            bool
	closed            bool
	timer             *time.Timer
	scroll            ui.ScrollState
}

func (t *agentTranscript) load() {
	if t.closed {
		return
	}
	store.AgentTranscript(t.chatID, t.messageID, func(text string, err error) {
		if t.closed {
			return
		}
		if err != nil {
			// What was shown stays; the next try may reach the Runner.
			if !t.loaded {
				t.err = model.ErrorText(err)
			}
		} else {
			if text != t.text {
				t.scroll.Y = 1 << 30
			}
			t.text, t.loaded, t.err = text, true, ""
		}
		t.schedule()
	})
}

func (t *agentTranscript) schedule() {
	if t.closed || store.IsMock {
		return
	}
	every := 2 * time.Second
	if t.remote {
		every = 5 * time.Second
	}
	t.timer = time.AfterFunc(every, func() { post(t.load) })
}

func (t *agentTranscript) close() {
	t.closed = true
	if t.timer != nil {
		t.timer.Stop()
	}
}

// presentAgentTranscript shows what a coding agent was sent, said, and did, or what its pane
// shows; on the Runner, an agent in a terminal host offers to show its pane there.
func (w *appWindow) presentAgentTranscript(chatID string, message *model.Message, agent *model.AgentRun) {
	t := &agentTranscript{chatID: chatID, messageID: message.ID, remote: !store.RunsHere(message)}
	t.load()
	title := agent.Name()
	if place := agent.Place(); place != "" {
		title += " · " + place
	}
	showsPane := agent.HostName() != "" && agent.IsRunning() && store.RunsHere(message)
	w.present(func(c *ui.Context, s *sheet) {
		p := colors(c)
		options := sheetOptions{Title: title, Width: 640, Confirm: L("Done"), NoCancel: true}
		if showsPane {
			options.Leading = func() {
				if pushButton(c, L("Show in %@", agent.HostName()), pushOptions{}).Clicked() {
					store.ShowAgent(chatID, message.ID, func(err error) {
						if err != nil {
							w.showAlert(alertOptions{Message: L("Could not show it"), Informative: model.ErrorText(err)}, nil)
						}
					})
				}
			}
		}
		result := sheetFrame(c, options, func() {
			box := ui.Scroll(c).Height(420).FillWidth().Padding(9, 10).Radius(8).Background(p.Code).TextColor(p.Label)
			box.TrackScroll(&t.scroll)
			box.Children(func() {
				switch {
				case t.text != "":
					ui.Text(c, t.text).Font(monoFont).FontSize(11.5).LineHeight(1.45).Selectable()
				case t.err != "":
					ui.Text(c, t.err).FontSize(12).TextColor(p.Label2)
				case t.loaded:
					ui.Text(c, L("Nothing yet")).FontSize(12).TextColor(p.Label2)
				default:
					ui.Text(c, L("Loading…")).FontSize(12).TextColor(p.Label2)
				}
			})
		})
		if result.Confirmed || result.Cancelled {
			s.dismiss()
		}
	}, t.close)
}
