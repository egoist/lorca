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

// codingAgents is the account's coding agents in its order of preference: a row each with its
// mark, whether this Device has it, arrows that move it up or down, and its switch. Every change
// sends the whole list, shows at once, and goes back with an alert when the CLI refuses it.
func (s settingsPane) codingAgents(c *ui.Context) {
	p := colors(c)
	if s.page != nil {
		s.page.coding.refresh()
	}
	choices := store.CodingAgents
	save := func(next []model.CodingAgentChoice) {
		store.SetCodingAgents(next, func(err error) {
			if err != nil {
				s.w.showAlert(alertOptions{Message: L("Could not save the coding agents"), Informative: model.ErrorText(err)}, nil)
			}
		})
	}
	s.frame(c, string(model.PaneCodingAgents), func() {
		s.section(c, codingAgentsEntry().row, nil, func(k *card) {
			for i, choice := range choices {
				name := model.CodingAgentName(choice.Kind)
				var installed model.InstalledCodingAgent
				known := false
				if s.page != nil {
					installed, known = s.page.coding.installed[choice.Kind]
				}
				detail := codingAgentDetail(installed, known)
				ui.Box(c.Key(choice.Kind)).Children(func() {
					row := k.row(rowBox(c).MinHeight(48).Label(name))
					row.Children(func() {
						ui.Row(c).Size(22, 22).Center().TextColor(p.Label).Children(func() { agentLogo(c, choice.Kind, 20) })
						ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
							ui.Text(c, name).FontSize(12.5).FontWeight(500).SingleLine()
							if detail != "" {
								ui.Text(c, detail).FontSize(textCaption).TextColor(p.Label2).SingleLine().Tooltip(detail)
							}
						})
						ui.Row(c).Gap(2).Shrink(0).Children(func() {
							up := hoverButton(c.Key("up"), hoverButtonOptions{Symbol: "arrow.up", Size: 13, Tooltip: L("Move %@ Up", name), Disabled: i == 0})
							down := hoverButton(c.Key("down"), hoverButtonOptions{Symbol: "arrow.down", Size: 13, Tooltip: L("Move %@ Down", name), Disabled: i == len(choices)-1})
							switch {
							case up.Clicked() && i > 0:
								next := slices.Clone(choices)
								next[i-1], next[i] = next[i], next[i-1]
								save(next)
							case down.Clicked() && i < len(choices)-1:
								next := slices.Clone(choices)
								next[i], next[i+1] = next[i+1], next[i]
								save(next)
							}
						})
						on := choice.Enabled
						index := i
						toggle := L("Turn %@ off", name)
						if !on {
							toggle = L("Turn %@ on", name)
						}
						toggleSwitch(c, &on, true).Label(toggle).Tooltip(toggle).OnChange(func() {
							next := slices.Clone(store.CodingAgents)
							if index < len(next) {
								next[index].Enabled = on
								save(next)
							}
						})
					})
					s.mark(c, row, name)
				})
			}
		})
		s.footnote(c, L("Bots hand coding work to these agents on their Runner. A bot uses the first one that's on and installed there, unless you asked for another."))
	})
}
