package main

import (
	"slices"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// settingsCodingState is the picked Runner's coding agents as it last answered, and which Runner
// that was. The list is asked when the pane shows a Runner, and each change takes its answer.
type settingsCodingState struct {
	deviceID string
	list     []model.InstalledCodingAgent
	loaded   bool
	failure  string
	// loads counts the asks and changes, so an answer one after it overtook is dropped.
	loads int
	// dragging is the order the rows show while one is dragged, saved once it is dropped.
	dragging []model.InstalledCodingAgent
	// passed is the row the dragged one last made room past. The row under the pointer is found
	// as the pointer moves, so until it moves again it is still that row, now in another place.
	passed string

	// sandboxes is the Runner's cloud sandboxes as it last answered, asked with the agents.
	sandboxes        model.CloudSandboxes
	sandboxesLoaded  bool
	sandboxesFailure string
	sandboxLoads     int
}

func (s *settingsCodingState) load(runnerID string) {
	s.loads++
	load := s.loads
	store.CodingAgents(runnerID, func(list []model.InstalledCodingAgent, err error) {
		if load != s.loads {
			return
		}
		if err != nil {
			s.failure = model.ErrorText(err)
			return
		}
		s.list, s.loaded, s.failure = list, true, ""
	})
	s.sandboxLoads++
	sandboxLoad := s.sandboxLoads
	store.CloudSandboxes(runnerID, func(sandboxes model.CloudSandboxes, err error) {
		if sandboxLoad != s.sandboxLoads {
			return
		}
		if err != nil {
			s.sandboxesFailure = model.ErrorText(err)
			return
		}
		s.sandboxes, s.sandboxesLoaded, s.sandboxesFailure = sandboxes, true, ""
	})
}

// took is the sandboxes as the Runner answered a change, unless the pane moved to another Runner.
func (s *settingsCodingState) took(runnerID string, sandboxes model.CloudSandboxes) {
	if s.deviceID != runnerID {
		return
	}
	s.sandboxLoads++
	s.sandboxes, s.sandboxesLoaded, s.sandboxesFailure = sandboxes, true, ""
}

// cloudSandboxes is the picked Runner's cloud sandboxes: where its agents run when a bot names no
// place, and a row each for Vercel Sandbox and Cloudflare Sandbox that opens its sheet.
func (s settingsPane) cloudSandboxes(c *ui.Context, k *card, device *model.Device, state *settingsCodingState) {
	p := colors(c)
	switch {
	case !state.sandboxesLoaded && state.sandboxesFailure != "":
		noteRow(c, k, state.sandboxesFailure, nil)
		return
	case !state.sandboxesLoaded:
		keyValueRow(c, k, L("Loading…"), "", false, nil)
		return
	}
	runnerID := device.ID
	options := []popUpOption{{Value: "", Label: device.Name}}
	for _, sandbox := range state.sandboxes.Sandboxes {
		if sandbox.SetUp {
			options = append(options, popUpOption{Value: sandbox.Kind, Label: sandbox.Name})
		}
	}
	where := L("Run agents on")
	if picked, changed := popUpRow(c, k, where, popUp{Options: options, Value: state.sandboxes.Default, Label: where, Disabled: len(options) == 1}, true); changed {
		previous := state.sandboxes
		state.sandboxes.Default = picked
		store.SetDefaultCloudSandbox(runnerID, picked, func(sandboxes model.CloudSandboxes, err error) {
			if err != nil {
				if state.deviceID == runnerID {
					state.sandboxes = previous
				}
				s.w.showAlert(alertOptions{Message: L("Could not change where agents run"), Informative: model.ErrorText(err)}, nil)
				return
			}
			state.took(runnerID, sandboxes)
		})
	}
	for _, kind := range model.CloudSandboxKinds {
		sandbox := state.sandboxes.Sandbox(kind)
		action := L("Set Up…")
		if sandbox.SetUp {
			action = L("Edit…")
		}
		row := k.row(rowBox(c).MinHeight(44).Label(sandbox.Name))
		row.Children(func() {
			ui.Row(c).Size(22, 22).Center().TextColor(p.Label).Children(func() { symbol(c, "cloud", 16, 2) })
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
				ui.Text(c, sandbox.Name).FontSize(12.5).FontWeight(500).SingleLine()
				detail := sandbox.Detail()
				ui.Text(c, detail).FontSize(textCaption).TextColor(p.Label2).SingleLine().Tooltip(detail)
			})
			if linkButton(c, action, false).Clicked() {
				s.w.presentCloudSandbox(device, sandbox, func(sandboxes model.CloudSandboxes) { state.took(runnerID, sandboxes) })
			}
		})
		s.mark(c, row, sandbox.Name)
	}
}

// codingAgentDetail is the line under an agent's name: whether its Runner has it, and which
// version.
func codingAgentDetail(agent model.InstalledCodingAgent) string {
	switch {
	case !agent.Installed:
		return L("Not installed")
	case agent.Version != "":
		return L("Installed · %@", agent.Version)
	}
	return L("Installed")
}

// codingAgentDrag is an agent's row being dragged to another place in the list.
type codingAgentDrag struct{ kind string }

// movedCodingAgent is the list with the agent `kind` moved to place `to`.
func movedCodingAgent(choices []model.InstalledCodingAgent, kind string, to int) []model.InstalledCodingAgent {
	from := slices.IndexFunc(choices, func(choice model.InstalledCodingAgent) bool { return choice.Kind == kind })
	if from < 0 || to < 0 || to >= len(choices) || from == to {
		return nil
	}
	next := slices.Clone(choices)
	moving := next[from]
	next = slices.Delete(next, from, from+1)
	return slices.Insert(next, to, moving)
}

// codingAgents is the picked Runner's coding agents in its order of preference: a row each with
// its mark, whether the Runner has it, and its switch. A row drags to another place, the rows
// making room for it as it goes, and a grip shows on the row under the pointer. Every change
// shows at once, sends the whole list to the Runner, takes its answer, and goes back with an
// alert when it refuses.
func (s settingsPane) codingAgents(c *ui.Context, m *mainWindow) {
	p := colors(c)
	device := store.Device(m.settingsDeviceID)
	title := codingAgentsEntry().row
	if device != nil {
		title = L("Coding Agents on %@", device.Name)
	}
	state := &settingsCodingState{}
	if s.page != nil {
		state = &s.page.coding
	}
	if device != nil && device.IsRunner() && state.deviceID != device.ID {
		*state = settingsCodingState{deviceID: device.ID}
		state.load(device.ID)
	}
	choices := state.list
	if state.dragging != nil {
		choices = state.dragging
	}
	save := func(next []model.InstalledCodingAgent) {
		previous, runnerID := state.list, state.deviceID
		state.list = next
		state.loads++
		load := state.loads
		store.SetCodingAgents(runnerID, next, func(list []model.InstalledCodingAgent, err error) {
			if load != state.loads || state.deviceID != runnerID {
				return
			}
			if err != nil {
				state.list = previous
				s.w.showAlert(alertOptions{Message: L("Could not save the coding agents"), Informative: model.ErrorText(err)}, nil)
				return
			}
			state.list = list
		})
	}
	// A drag that ended with no row taking it (Escape, or released off the list) puts the rows back.
	dragged, dropped := false, false
	s.frame(c, string(model.PaneCodingAgents)+"|"+m.settingsDeviceID, func() {
		s.section(c, title, nil, func(k *card) {
			settingsRunnerRows(c, k, device, func() {
				switch {
				case !state.loaded && state.failure != "":
					noteRow(c, k, state.failure, nil)
					return
				case !state.loaded:
					keyValueRow(c, k, L("Loading…"), "", false, nil)
					return
				}
				for i, agent := range choices {
					name := model.CodingAgentName(agent.Kind)
					detail := codingAgentDetail(agent)
					ui.Box(c.Key(agent.Kind)).Children(func() {
						row := k.row(rowBox(c).MinHeight(48).Label(name)).Drag(codingAgentDrag{kind: agent.Kind}).Cursor(ui.CursorGrab)
						// The copy that follows the pointer is drawn from the row; the row itself is the
						// empty place it would drop into.
						row.Background(p.BotBubble)
						if row.Dragging() {
							dragged = true
							row.Opacity(0)
						}
						if over, ok := ui.DragOver[codingAgentDrag](row); ok {
							switch {
							case over.kind == agent.Kind:
								state.passed = ""
							case agent.Kind != state.passed:
								if next := movedCodingAgent(choices, over.kind, i); next != nil {
									state.dragging, state.passed = next, agent.Kind
								}
							}
						}
						if _, ok := ui.Drop[codingAgentDrag](row); ok {
							dropped = true
							if next := state.dragging; next != nil && !slices.Equal(next, state.list) {
								save(next)
							}
							state.dragging, state.passed = nil, ""
						}
						hovered := row.Hovered() && state.dragging == nil
						row.Children(func() {
							ui.Row(c).Size(22, 22).Center().TextColor(p.Label).Children(func() { agentLogo(c, agent.Kind, 20) })
							ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
								ui.Text(c, name).FontSize(12.5).FontWeight(500).SingleLine()
								ui.Text(c, detail).FontSize(textCaption).TextColor(p.Label2).SingleLine().Tooltip(detail)
							})
							grip := symbol(c, "line.3.horizontal", 13, 2).TextColor(p.Label3).Tooltip(L("Drag to reorder"))
							if !hovered {
								grip.Opacity(0)
							}
							on := agent.Enabled
							kind := agent.Kind
							toggle := L("Turn %@ off", name)
							if !on {
								toggle = L("Turn %@ on", name)
							}
							toggleSwitch(c, &on, true).Label(toggle).Tooltip(toggle).OnChange(func() {
								next := slices.Clone(state.list)
								if i := slices.IndexFunc(next, func(agent model.InstalledCodingAgent) bool { return agent.Kind == kind }); i >= 0 {
									next[i].Enabled = on
									save(next)
								}
							})
						})
						s.mark(c, row, name)
					})
				}
			})
		})
		s.footnote(c, L("Bots on this Runner hand coding work to these agents. A bot uses the first one that's on and installed, unless you asked for another. Drag an agent to change the order."))
		if device != nil && device.IsRunner() {
			s.section(c, L("Cloud Sandboxes"), nil, func(k *card) {
				s.cloudSandboxes(c, k, device, state)
			})
			s.footnote(c, L("An agent in a cloud sandbox works on a clone of the folder's repository, in your own Vercel or Cloudflare account, and pushes from there instead of running on this Runner. Bots run agents where you pick here, unless you ask for another place."))
		}
	})
	if !dragged && !dropped {
		state.dragging, state.passed = nil, ""
	}
}
