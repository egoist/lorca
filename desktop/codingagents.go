package main

import (
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The account's coding agents, every bot's in every chat on every Runner, in a flyout under the
// header's Coding agents button, as Attention's: each agent's status mark, its title, the bot,
// chat, and Runner it belongs to, and how it stands with its pull request, branch, and folder,
// open ones first. A row opens the agent's page, from its card when its chat is not loaded here.
// The CLI lists them from its own store (`coding.list`); the store asks again as a start row's
// card changes.

// codingAgentsWidth is the flyout's.
const codingAgentsWidth = 420

type codingAgentsState struct {
	// open is the flyout showing, or asked for before the button had a place to anchor it.
	open   bool
	flyout flyoutWindow
	// height is the flyout's, fitted to its content.
	height int
}

// listedAgentOwner is "Developer · Launch · Studio": the bot, the chat, and the Runner. A direct
// chat goes by the bot's name, so its title is not said twice.
func listedAgentOwner(listed model.ListedAgent) string {
	chat := listed.ChatTitle
	if chat == listed.BotName {
		chat = ""
	}
	var parts []string
	for _, part := range []string{listed.BotName, chat, listed.Agent.Device} {
		if part != "" {
			parts = append(parts, part)
		}
	}
	return strings.Join(parts, " · ")
}

// listedAgentDetail is "Ready for review · #7 · docs-use-mygo-json · ~/Projects/site".
func listedAgentDetail(agent *model.AgentRun) string {
	parts := []string{agent.Status()}
	for _, part := range []string{agent.PullNumber(), agent.Branch, agent.Folder} {
		if part != "" {
			parts = append(parts, part)
		}
	}
	return strings.Join(parts, " · ")
}

// codingAgentsButton is the account's coding agents in the header: shown while one is live, or
// while the flyout is open. It carries no badge, which would read as unread messages.
func (m *mainWindow) codingAgentsButton(c *ui.Context) {
	s := &m.coding
	count := store.LiveAgents(model.Now())
	if count == 0 && !s.open {
		return
	}
	holder := ui.Box(c.Key("coding-agents"))
	var button ui.Element
	holder.Children(func() {
		button = hoverButton(c, hoverButtonOptions{Symbol: "chevron.left.forwardslash.chevron.right", Tooltip: L("Coding agents"), Label: L("Coding agents (%d)", count), Active: s.open})
	})
	if button.Clicked() {
		if s.open {
			m.closeCodingAgents()
		} else {
			s.open = true
		}
	}
	// The button is placed a frame after it first shows: the flyout opens under it then.
	if s.open && s.flyout == nil {
		if anchor := button.Bounds(); anchor.W > 0 {
			m.openCodingAgents(anchor)
		} else {
			c.Invalidate()
		}
	}
}

func (m *mainWindow) openCodingAgents(anchor ui.Rect) {
	// One header flyout shows at a time.
	if m.attention.open {
		m.closeAttention()
	}
	if m.tasks.open {
		m.closeRunningTasks()
	}
	s := &m.coding
	s.height = flyoutHeight(codingAgentsWidth, m.codingAgentsPanel)
	var flyout flyoutWindow
	flyout = openFlyout(m.win, anchor, codingAgentsWidth, s.height, m.codingAgentsFlyout, func() {
		if s.flyout == flyout {
			s.flyout, s.open = nil, false
			m.invalidate()
		}
	})
	s.flyout = flyout
}

func (m *mainWindow) closeCodingAgents() {
	s := &m.coding
	flyout := s.flyout
	s.flyout, s.open = nil, false
	if flyout != nil {
		flyout.Close()
	}
	m.invalidate()
}

// codingAgentsFlyout is the flyout's content: the panel, which the flyout follows as the list
// changes while it shows.
func (m *mainWindow) codingAgentsFlyout(c *ui.Context) {
	fitFlyout(m.coding.flyout, &m.coding.height, codingAgentsWidth, m.codingAgentsPanel(c))
}

func (m *mainWindow) codingAgentsPanel(c *ui.Context) ui.Element {
	return flyoutPanel(c, codingAgentsWidth).Children(func() { m.codingAgentsList(c) })
}

func (m *mainWindow) codingAgentsList(c *ui.Context) {
	p := colors(c)
	agents := store.AccountAgents()
	c.After(2 * time.Second)
	if len(agents) == 0 {
		ui.Text(c, L("No coding agents")).Padding(16).FontSize(12.5).TextColor(p.Label2)
		return
	}
	ui.Scroll(c).MaxHeight(480).Padding(4, 6).Children(func() {
		for i, listed := range agents {
			agent := listed.Agent
			row := ui.Row(c.Key(listed.ChatID+"/"+listed.MessageID)).AlignItems(ui.Start).Gap(10).Padding(10, 8).Radius(6).
				Label(agent.Label()).Tooltip(agent.PullRequest).Role(ui.RoleButton).Cursor(ui.CursorPointer)
			if row.Hovered() {
				row.Background(p.Hover)
			}
			if i > 0 {
				line := p.Separator
				row.DrawOver(func(painter *ui.Painter, r ui.Rect) {
					painter.Fill(ui.Rect{X: r.X + 8, Y: r.Y, W: r.W - 16, H: 1}, line, 0)
				})
			}
			row.Children(func() {
				ui.Row(c).Width(18).Padding(1, 0, 0, 0).Justify(ui.Center).Children(func() { agentStatusMark(c, agent) })
				ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
					ui.Text(c, agent.Label()).FontSize(12.5).FontWeight(600).MaxLines(2)
					ui.Text(c, listedAgentOwner(listed)).FontSize(11).TextColor(p.Label2).SingleLine()
					ui.Text(c, listedAgentDetail(agent)).FontSize(11).TextColor(p.Label3).SingleLine()
				})
			})
			if row.Clicked() {
				m.closeCodingAgents()
				m.presentListedAgent(listed)
			}
		}
	})
}
