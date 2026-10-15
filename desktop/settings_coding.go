package main

import (
	"slices"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// settingsCodingState is what this Device's CLI said of the coding agents it has: installed or
// not, and the version. Asked again whenever the pane comes back.
type settingsCodingState struct {
	asked     bool
	installed map[string]model.InstalledCodingAgent
	// dragging is the order the rows show while one is dragged, saved once it is dropped.
	dragging []model.CodingAgentChoice
	// passed is the row the dragged one last made room past. The row under the pointer is found
	// as the pointer moves, so until it moves again it is still that row, now in another place.
	passed string
}

func (s *settingsCodingState) refresh() {
	if s.asked {
		return
	}
	s.asked = true
	store.InstalledCodingAgents(func(agents []model.InstalledCodingAgent, err error) {
		if err != nil {
			return
		}
		s.installed = map[string]model.InstalledCodingAgent{}
		for _, agent := range agents {
			s.installed[agent.Kind] = agent
		}
	})
}

// codingAgentDetail is the line under an agent's name: whether this Device has it, and which
// version; nothing until the CLI says.
func codingAgentDetail(agent model.InstalledCodingAgent, known bool) string {
	switch {
	case !known:
		return ""
	case !agent.Installed:
		return L("Not installed on this Device")
	case agent.Version != "":
		return L("Installed · %@", agent.Version)
	}
	return L("Installed")
}

// codingAgentDrag is an agent's row being dragged to another place in the list.
type codingAgentDrag struct{ kind string }

// movedCodingAgent is the list with the agent `kind` moved to place `to`.
func movedCodingAgent(choices []model.CodingAgentChoice, kind string, to int) []model.CodingAgentChoice {
	from := slices.IndexFunc(choices, func(choice model.CodingAgentChoice) bool { return choice.Kind == kind })
	if from < 0 || to < 0 || to >= len(choices) || from == to {
		return nil
	}
	next := slices.Clone(choices)
	moving := next[from]
	next = slices.Delete(next, from, from+1)
	return slices.Insert(next, to, moving)
}

// codingAgents is the account's coding agents in its order of preference: a row each with its
// mark, whether this Device has it, and its switch. A row drags to another place, the rows making
// room for it as it goes, and a grip shows on the row under the pointer. Every change sends the
// whole list, shows at once, and goes back with an alert when the CLI refuses it.
func (s settingsPane) codingAgents(c *ui.Context) {
	p := colors(c)
	state := &settingsCodingState{}
	if s.page != nil {
		state = &s.page.coding
		state.refresh()
	}
	choices := store.CodingAgents
	if state.dragging != nil {
		choices = state.dragging
	}
	save := func(next []model.CodingAgentChoice) {
		store.SetCodingAgents(next, func(err error) {
			if err != nil {
				s.w.showAlert(alertOptions{Message: L("Could not save the coding agents"), Informative: model.ErrorText(err)}, nil)
			}
		})
	}
	// A drag that ended with no row taking it (Escape, or released off the list) puts the rows back.
	dragged, dropped := false, false
	s.frame(c, string(model.PaneCodingAgents), func() {
		s.section(c, codingAgentsEntry().row, nil, func(k *card) {
			for i, choice := range choices {
				name := model.CodingAgentName(choice.Kind)
				installed, known := state.installed[choice.Kind]
				detail := codingAgentDetail(installed, known)
				ui.Box(c.Key(choice.Kind)).Children(func() {
					row := k.row(rowBox(c).MinHeight(48).Label(name)).Drag(codingAgentDrag{kind: choice.Kind}).Cursor(ui.CursorGrab)
					// The copy that follows the pointer is drawn from the row; the row itself is the
					// empty place it would drop into.
					row.Background(p.BotBubble)
					if row.Dragging() {
						dragged = true
						row.Opacity(0)
					}
					if over, ok := ui.DragOver[codingAgentDrag](row); ok {
						switch {
						case over.kind == choice.Kind:
							state.passed = ""
						case choice.Kind != state.passed:
							if next := movedCodingAgent(choices, over.kind, i); next != nil {
								state.dragging, state.passed = next, choice.Kind
							}
						}
					}
					if _, ok := ui.Drop[codingAgentDrag](row); ok {
						dropped = true
						if next := state.dragging; next != nil && !slices.Equal(next, store.CodingAgents) {
							save(next)
						}
						state.dragging, state.passed = nil, ""
					}
					hovered := row.Hovered() && state.dragging == nil
					row.Children(func() {
						ui.Row(c).Size(22, 22).Center().TextColor(p.Label).Children(func() { agentLogo(c, choice.Kind, 20) })
						ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
							ui.Text(c, name).FontSize(12.5).FontWeight(500).SingleLine()
							if detail != "" {
								ui.Text(c, detail).FontSize(textCaption).TextColor(p.Label2).SingleLine().Tooltip(detail)
							}
						})
						grip := symbol(c, "line.3.horizontal", 13, 2).TextColor(p.Label3).Tooltip(L("Drag to reorder"))
						if !hovered {
							grip.Opacity(0)
						}
						on := choice.Enabled
						kind := choice.Kind
						toggle := L("Turn %@ off", name)
						if !on {
							toggle = L("Turn %@ on", name)
						}
						toggleSwitch(c, &on, true).Label(toggle).Tooltip(toggle).OnChange(func() {
							next := slices.Clone(store.CodingAgents)
							if i := slices.IndexFunc(next, func(choice model.CodingAgentChoice) bool { return choice.Kind == kind }); i >= 0 {
								next[i].Enabled = on
								save(next)
							}
						})
					})
					s.mark(c, row, name)
				})
			}
		})
		s.footnote(c, L("Bots hand coding work to these agents on their Runner. A bot uses the first one that's on and installed there, unless you asked for another. Drag an agent to change the order."))
	})
	if !dragged && !dropped {
		state.dragging, state.passed = nil, ""
	}
}
