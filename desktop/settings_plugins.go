package main

import (
	"slices"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// plugins is what the picked Runner has installed, with each plugin's state, and the
// marketplace; then the MCP servers of its mcp.json.
func (s settingsPane) plugins(c *ui.Context, m *mainWindow) {
	p := colors(c)
	device := store.Device(m.settingsDeviceID)
	title := L("Plugins")
	if device != nil {
		title = L("Plugins on %@", device.Name)
	}
	s.frame(c, string(model.PanePlugins)+"|"+m.settingsDeviceID, func() {
		s.section(c, title, nil, func(k *card) {
			settingsRunnerRows(c, k, device, func() {
				// Its mcp.json's servers are listed in their own section.
				shown := 0
				for _, plugin := range device.Plugins {
					if plugin.IsMcpServer() {
						continue
					}
					shown++
					ui.Column(c.Key(plugin.ID)).Children(func() {
						rowElement, row := pluginRow(c, k, plugin, true, "")
						s.mark(c, rowElement, plugin.Name)
						if row.Clicked {
							s.w.presentPlugin(plugin.ID, device)
						}
					})
				}
				if shown == 0 {
					keyValueRow(c, k, L("No plugins installed"), "", false, &p.Label2)
				}
				marketplaceElement, marketplace := actionRow(c, k, L("Marketplace"), actionRowOptions{Tint: &p.Label2, Action: L("Add from Plugins…")})
				s.mark(c, marketplaceElement, L("Marketplace"))
				if marketplace.Action {
					m.presentMarketplace(device.ID)
				}
			})
		})
		s.footnote(c, L("Plugins are installed on a Runner, and the bots assigned to it use them. An action that changes something goes through Auto-review first."))
		if device != nil && device.IsRunner() {
			s.mcpServers(c, device)
			s.footnote(c, L("Servers you add yourself live in mcp.json on %@, in the format Claude Desktop and Cursor use. Edit them here or with the lorca mcp command; after editing the file itself, click Reload.", device.Name))
		}
	})
}

// settingsRunnerRows fills a Runner's section: its rows on a Runner, else the one row a Device
// that is not one shows, or before the CLI answers.
func settingsRunnerRows(c *ui.Context, k *card, device *model.Device, rows func()) {
	switch {
	case device == nil:
		keyValueRow(c, k, L("Waiting for the CLI"), "", false, nil)
	case !device.IsRunner():
		text := model.UnknownDeviceNote()
		if device.OS != model.OSUnknown {
			text = L("%@ Devices hold your keys and chats but never run a bot. Pick a Runner: a Device running macOS, Linux, or Windows.", device.OS.DisplayName())
		}
		noteRow(c, k, text, nil)
	default:
		rows()
	}
}

// MARK: - MCP servers

// settingsMcpState is the picked Runner's mcp.json as it last answered, and which Runner that was.
type settingsMcpState struct {
	// key is what the list was last asked for: the Runner, and how its servers stood.
	key      string
	deviceID string
	file     *model.McpFile
	failure  string
	// loads counts the asks, so an answer that an ask after it overtook is dropped.
	loads int
	// changed is the CLI having said something changed since the pane looked; reloadAt is when
	// this computer's file is asked again for it.
	changed   bool
	reloadAt  time.Time
	listening *model.Store
}

func (s *settingsMcpState) load(runnerID string) {
	s.loads++
	load := s.loads
	store.McpServers(runnerID, func(file model.McpFile, err error) {
		if load != s.loads {
			return
		}
		if err != nil {
			s.failure = model.ErrorText(err)
			return
		}
		s.file, s.failure = &file, ""
	})
}

// replace puts a server's new standing in the list.
func (s *settingsMcpState) replace(server model.McpServer) {
	if s.file == nil {
		return
	}
	file := *s.file
	file.Servers = slices.Clone(file.Servers)
	for i := range file.Servers {
		if file.Servers[i].Name == server.Name {
			file.Servers[i] = server
		}
	}
	s.file = &file
}

// mcpKey is the Runner and how its mcp.json's servers stand in its roster.
func mcpKey(device *model.Device) string {
	var b strings.Builder
	b.WriteString(device.ID)
	for _, plugin := range device.Plugins {
		if plugin.IsMcpServer() {
			b.WriteString("|" + plugin.ID + "\x00" + string(plugin.State) + "\x00" + plugin.Detail)
		}
	}
	return b.String()
}

// mcpServers is a Runner's mcp.json: each server with how it stands and a switch, a row to add
// one, and the file itself, with Reload for an edit made outside Lorca (and on this computer,
// Open). The list follows the servers' states in the Runner's roster.
func (s settingsPane) mcpServers(c *ui.Context, device *model.Device) {
	p := colors(c)
	mcp := &s.page.mcp
	now := c.Now()
	if mcp.listening != store {
		// This computer's file is asked again whenever the CLI says something changed, as a
		// reload from a terminal (`lorca mcp reload`) does: a file that no longer reads moves no
		// server's state.
		mcp.listening = store
		listened := store
		store.Subscribe(func(event model.Event) {
			if event.Kind == model.EventRosterChanged && listened == store {
				mcp.changed = true
			}
		})
	}
	// Again when another Device is picked, or a server of this one changes how it stands.
	if key := mcpKey(device); key != mcp.key {
		if mcp.deviceID != device.ID {
			mcp.file, mcp.failure = nil, ""
		}
		mcp.key, mcp.deviceID = key, device.ID
		mcp.load(device.ID)
	}
	if mcp.changed {
		mcp.changed = false
		if device.IsThisDevice {
			mcp.reloadAt = now.Add(300 * time.Millisecond)
		}
	}
	if !mcp.reloadAt.IsZero() {
		if wait := mcp.reloadAt.Sub(now); wait > 0 {
			c.After(wait)
		} else {
			mcp.reloadAt = time.Time{}
			mcp.load(device.ID)
		}
	}

	runnerID := device.ID
	s.section(c, L("MCP Servers on %@", device.Name), nil, func(k *card) {
		if mcp.file == nil {
			label, tint := L("Loading…"), &p.Label2
			if mcp.failure != "" {
				label, tint = mcp.failure, &p.Red
			}
			keyValueRow(c, k, label, "", false, tint)
			return
		}
		file := *mcp.file
		if file.Error != "" {
			noteRow(c, k, L("%@ Lorca keeps the servers it read before.", file.Error), &p.Red)
		}
		for _, server := range file.Servers {
			ui.Column(c.Key(server.Name)).Children(func() {
				rowElement, clicked := settingsMcpRow(c, k, server,
					func(on bool) { s.setMcpEnabled(runnerID, server, on) })
				s.mark(c, rowElement, server.Name)
				if clicked {
					s.w.presentMcpServer(device, server.Name)
				}
			})
		}
		if len(file.Servers) == 0 && file.Error == "" {
			noteRow(c, k, L("No MCP servers yet. Add one by the command that starts it or its URL, or paste the JSON from its README."), nil)
		}
		addElement, add := actionRow(c, k, L("Custom"), actionRowOptions{Tint: &p.Label2, Action: L("Add Server…")})
		s.mark(c, addElement, L("Custom"))
		if add.Action {
			s.w.presentMcpServer(device, "")
		}
		// Open once the file is there, which it is once it holds a server.
		second := ""
		if device.IsThisDevice && (len(file.Servers) > 0 || file.Error != "") {
			second = L("Open")
		}
		rowElement, row := actionRow(c, k, "mcp.json", actionRowOptions{Value: file.Path, Tint: &p.Label2, Mono: true, Tooltip: file.Path, Action: L("Reload"), Second: second})
		s.mark(c, rowElement, "mcp.json")
		if row.Second {
			path := file.Path
			go func() {
				if mygo.Shell.OpenPath(path) != nil {
					mygo.Shell.ShowItemInFolder(path)
				}
			}()
		}
		if row.Action {
			s.reloadMcp(runnerID)
		}
	})
}

// reloadMcp has the Runner read its mcp.json again, after an edit made outside Lorca.
func (s settingsPane) reloadMcp(runnerID string) {
	mcp, w := &s.page.mcp, s.w
	mcp.loads++
	store.ReloadMcpServers(runnerID, func(file model.McpFile, err error) {
		if err != nil {
			w.showAlert(alertOptions{Message: L("Couldn't reload mcp.json"), Informative: model.ErrorText(err)}, nil)
			return
		}
		if mcp.deviceID == runnerID {
			mcp.file, mcp.failure = &file, ""
		}
	})
}

// setMcpEnabled turns a server on or off: the switch moves at once and goes back if the Runner
// cannot.
func (s settingsPane) setMcpEnabled(runnerID string, server model.McpServer, on bool) {
	mcp, w := &s.page.mcp, s.w
	moved := server
	moved.Enabled = on
	mcp.replace(moved)
	store.SetMcpServerEnabled(runnerID, server.Name, on, func(next model.McpServer, err error) {
		if mcp.deviceID != runnerID {
			return
		}
		if err != nil {
			mcp.replace(server)
			message := L("Couldn't turn %@ off", server.Name)
			if on {
				message = L("Couldn't turn %@ on", server.Name)
			}
			w.showAlert(alertOptions{Message: message, Informative: model.ErrorText(err)}, nil)
			return
		}
		mcp.replace(next)
	})
}

// settingsMcpRow is one server: its symbol in the color of how it stands, its name, its state and
// address, and a switch, unless it cannot run. It reports clicks outside the switch; change runs
// after the view is built with the switch's new value.
func settingsMcpRow(c *ui.Context, k *card, server model.McpServer, change func(bool)) (ui.Element, bool) {
	p := colors(c)
	r := k.row(rowBox(c).MinHeight(44).Label(server.Name).Cursor(ui.CursorPointer))
	if description := server.Entry.Description(); description != "" {
		r.Tooltip(description)
	}
	if r.Hovered() {
		r.Background(p.RowHover)
	}
	clicked := r.Clicked()
	r.Children(func() {
		state, tone := model.McpStateText(server)
		stateColor := p.tone(tone)
		icon := p.Label3
		if server.Enabled && server.Problem == "" {
			icon = stateColor
		}
		ui.Row(c).Width(18).Justify(ui.Center).TextColor(icon).Children(func() { symbol(c, server.Entry.Symbol(), 16, 1.7) })
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			title := ui.Text(c, server.Name).FontSize(12.5).FontWeight(500).SingleLine()
			if !server.Enabled {
				title.TextColor(p.Label2)
			}
			ui.RichText(c,
				ui.Span{Text: state, Color: stateColor},
				ui.Span{Text: " · " + model.McpAddress(server.Entry), Font: monoFont, Size: textCaption, Color: p.Label3},
			).FontSize(textCaption).SingleLine()
		})
		if server.Problem == "" {
			on := server.Enabled
			tooltip := L("Turn %@ on", server.Name)
			if on {
				tooltip = L("Turn %@ off", server.Name)
			}
			toggle := toggleSwitch(c, &on, true).Tooltip(tooltip).Label(server.Name).
				OnChange(func() { change(on) })
			if toggle.Changed() {
				clicked = false
			}
		}
	})
	return r, clicked
}
