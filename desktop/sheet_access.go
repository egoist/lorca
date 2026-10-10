package main

import (
	"slices"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// botAccessSheet is a bot's Access, after the Mac's BotAccessViewController: how far it may use
// each plugin on its Runner, down to single tools, and whether it reads or changes files and runs
// shell commands there. Only the user changes it; a chat's access request opens this sheet too.
// The choices live here, across frames, until Save.
type botAccessSheet struct {
	botID, botName string
	runner         *model.Device
	saved          *model.BotPermissions
	// plugins are the Runner's own list until it says what tools each one has.
	plugins []model.AccessPlugin
	levels  map[string]model.AccessLevel
	// chosen holds the tools chosen for a plugin; none here is all of them, one added later
	// included.
	chosen     map[string][]string
	expanded   map[string]bool
	filesystem model.AccessLevel
	shell      bool
	drafts     bool
	// problem is why the tools could not be listed.
	problem string
	closed  bool
}

func (w *appWindow) presentBotAccess(botID string) *botAccessSheet {
	bot := store.Bot(botID)
	if bot == nil {
		return nil
	}
	saved := bot.Permissions
	if saved == nil {
		saved = model.FullAccess()
	}
	st := &botAccessSheet{
		botID: botID, botName: bot.Name, runner: store.Device(bot.RunnerID), saved: saved.Clone(),
		levels: map[string]model.AccessLevel{}, chosen: map[string][]string{}, expanded: map[string]bool{},
		filesystem: saved.Filesystem, shell: saved.Shell, drafts: saved.Drafts,
	}
	// The Runner's own list first, so the sheet opens complete while it is asked for tools.
	var listed []model.AccessPlugin
	if st.runner != nil {
		for _, plugin := range st.runner.Plugins {
			listed = append(listed, model.AccessPlugin{ID: plugin.ID, Name: plugin.Name})
		}
	}
	st.show(listed)
	w.present(st.view, func() { st.closed = true })
	store.BotAccessCatalog(botID, func(catalog model.AccessCatalog, err error) {
		if st.closed {
			return
		}
		if err != nil {
			st.problem = L("Couldn't get the tools from %@.", st.runnerName())
			return
		}
		st.show(catalog.Connections)
	})
	return st
}

func (st *botAccessSheet) runnerName() string {
	if st.runner != nil {
		return st.runner.Name
	}
	return L("its Runner")
}

// show lists `listed`, keeping what the user already chose for a plugin listed before.
func (st *botAccessSheet) show(listed []model.AccessPlugin) {
	st.plugins = listed
	for _, plugin := range listed {
		if _, ok := st.levels[plugin.ID]; ok {
			continue
		}
		st.levels[plugin.ID] = st.saved.Level(plugin.ID)
		if st.saved.Connections != nil {
			if tools := (*st.saved.Connections)[plugin.ID].Tools; tools != nil {
				st.chosen[plugin.ID] = slices.Clone(*tools)
			}
		}
	}
}

// toolsSummary is "All tools", or how many of them the bot may use. Nothing before the plugin
// has connected once, or when it is off.
func (st *botAccessSheet) toolsSummary(plugin model.AccessPlugin) string {
	if len(plugin.Tools) == 0 || st.levels[plugin.ID] == model.AccessNone {
		return ""
	}
	chosen, ok := st.chosen[plugin.ID]
	if !ok {
		return L("All tools")
	}
	count := 0
	for _, tool := range plugin.Tools {
		if slices.Contains(chosen, tool.Name) {
			count++
		}
	}
	return L("%d of %d tools", count, len(plugin.Tools))
}

// toggleTool chooses or leaves out one tool. Every tool chosen is all of them.
func (st *botAccessSheet) toggleTool(plugin model.AccessPlugin, name string, on bool) {
	chosen, ok := st.chosen[plugin.ID]
	if !ok {
		for _, tool := range plugin.Tools {
			chosen = append(chosen, tool.Name)
		}
	}
	chosen = slices.DeleteFunc(slices.Clone(chosen), func(each string) bool { return each == name })
	if on {
		chosen = append(chosen, name)
	}
	for _, tool := range plugin.Tools {
		if !slices.Contains(chosen, tool.Name) {
			slices.Sort(chosen)
			st.chosen[plugin.ID] = chosen
			return
		}
	}
	delete(st.chosen, plugin.ID)
}

// policy is what Save writes. Every plugin fully open is the Runner's every plugin, one installed
// later included.
func (st *botAccessSheet) policy() *model.BotPermissions {
	policy := &model.BotPermissions{Filesystem: st.filesystem, Shell: st.shell, Drafts: st.drafts}
	open := true
	for _, plugin := range st.plugins {
		if _, picked := st.chosen[plugin.ID]; st.levels[plugin.ID] != model.AccessWrite || picked {
			open = false
		}
	}
	if !open {
		connections := map[string]model.ConnectionPermissions{}
		for _, plugin := range st.plugins {
			level := st.levels[plugin.ID]
			if level == model.AccessNone {
				continue
			}
			grant := model.ConnectionPermissions{Capabilities: level.Capabilities()}
			if chosen, ok := st.chosen[plugin.ID]; ok {
				tools := slices.Clone(chosen)
				grant.Tools = &tools
			}
			connections[plugin.ID] = grant
		}
		policy.Connections = &connections
	}
	return policy
}

func (st *botAccessSheet) view(c *ui.Context, s *sheet) {
	p := colors(c)
	result := sheetFrame(c, sheetOptions{Title: L("Access for %@", st.botName), Width: 460, Confirm: L("Save")}, func() {
		if len(st.plugins) > 0 {
			ui.Column(c).Gap(6).Children(func() {
				section(c, L("Plugins"), sectionCaption, nil, func(k *card) {
					for _, plugin := range st.plugins {
						ui.Column(c.Key("plugin:" + plugin.ID)).Children(func() { st.pluginRow(c, k, plugin) })
					}
				})
				if st.problem != "" {
					ui.Text(c, st.problem).FontSize(textCaption).TextColor(p.Label2).LineHeight(1.4)
				}
			})
		}
		ui.Column(c).Gap(6).Margin(4, 0, 0, 0).Children(func() {
			section(c, L("Messages"), sectionCaption, nil, func(k *card) {
				accessoryRow(c, k, L("Draft first"), "", func() {
					settingsSwitch(c, &st.drafts, L("Draft first"), false)
				})
			})
			ui.Text(c, L("Emails and Slack messages wait in the chat for you to send. Off, %@ sends them itself where the service can.", st.botName)).FontSize(textCaption).TextColor(p.Label2).LineHeight(1.4)
		})
		ui.Column(c).Gap(6).Margin(4, 0, 0, 0).Children(func() {
			// Files and shell commands are the Runner's, so its card goes by the Runner's name.
			title := L("Runner")
			if st.runner != nil {
				title = st.runner.Name
			}
			section(c, title, sectionCaption, nil, func(k *card) {
				accessoryRow(c, k, L("Files"), "", func() {
					var options []popUpOption
					for _, level := range []model.AccessLevel{model.AccessWrite, model.AccessRead, model.AccessNone} {
						options = append(options, popUpOption{Value: string(level), Label: level.Title()})
					}
					if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: string(st.filesystem), Style: popUpSettings, Label: L("Files")}); changed {
						st.filesystem = model.AccessLevel(picked)
					}
				})
				accessoryRow(c, k, L("Shell commands"), "", func() {
					settingsSwitch(c, &st.shell, L("Shell commands"), false)
				})
			})
			ui.Text(c, L("Shell commands run as you on %@ and can reach anything you can there.", st.runnerName())).FontSize(textCaption).TextColor(p.Label2).LineHeight(1.4)
		})
	})
	switch {
	case result.Cancelled:
		s.dismiss()
	case result.Confirmed:
		if policy := st.policy(); !model.EqualBotPermissions(policy, st.saved) {
			store.SetBotPermissions(st.botID, policy)
		}
		s.dismiss()
	}
}

// pluginRow is a plugin, after the Mac's PluginAccessRow: its mark and name, how many of its tools
// the bot may use, and how far. A click on the row shows the tools under it.
func (st *botAccessSheet) pluginRow(c *ui.Context, k *card, plugin model.AccessPlugin) {
	p := colors(c)
	level := st.levels[plugin.ID]
	canExpand := len(plugin.Tools) > 0 && level != model.AccessNone
	if !canExpand {
		delete(st.expanded, plugin.ID)
	}
	expanded := st.expanded[plugin.ID]
	r := k.row(rowBox(c).MinHeight(44).Padding(0, 12, 0, 6).Gap(4).Label(plugin.Name))
	r.Children(func() {
		// Everything but the pop-up shows or hides the tools.
		head := ui.Row(c).Grow(1).Shrink(1).MinWidth(0).Gap(4).AlignSelf(ui.Stretch).AlignItems(ui.Center)
		if canExpand {
			head.Cursor(ui.CursorPointer)
			if head.Clicked() {
				st.expanded[plugin.ID] = !expanded
			}
		}
		head.Children(func() {
			disclosure := ui.Row(c).Size(16, 16).Shrink(0).Center().TextColor(p.Label2)
			if canExpand {
				name := "chevron.right"
				if expanded {
					name = "chevron.down"
				}
				disclosure.Label(L("%@ tools", plugin.Name)).Children(func() { symbol(c, name, 12, 2) })
			}
			ui.Row(c).Width(18).Shrink(0).Justify(ui.Center).TextColor(p.Label2).Children(func() {
				// A named account (Gmail · Work) has its service's mark.
				symbolName, markID := "puzzlepiece.extension", plugin.ID
				if st.runner != nil {
					for _, installed := range st.runner.Plugins {
						if installed.ID == plugin.ID {
							symbolName, markID = installed.Symbol(), installed.MarketplaceID()
						}
					}
				}
				pluginTile(c, markID, symbolName, 18)
			})
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Margin(0, 0, 0, 6).Children(func() {
				ui.Text(c, plugin.Name).FontSize(12.5).FontWeight(500).SingleLine()
				if summary := st.toolsSummary(plugin); summary != "" {
					ui.Text(c, summary).FontSize(textCaption).TextColor(p.Label2).SingleLine()
				}
			})
		})
		if canExpand && head.Hovered() {
			r.Background(p.RowHover)
		}
		// Read and draft only for a plugin with a tool that drafts.
		drafts := level == model.AccessDraft || slices.ContainsFunc(plugin.Tools, func(tool model.AccessTool) bool { return tool.Capability == "draft" })
		var options []popUpOption
		for _, each := range model.AccessLevels {
			if each != model.AccessDraft || drafts {
				options = append(options, popUpOption{Value: string(each), Label: each.Title()})
			}
		}
		if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: string(level), Style: popUpSettings, Label: L("Access to %@", plugin.Name)}); changed {
			st.levels[plugin.ID] = model.AccessLevel(picked)
		}
	})
	if !expanded {
		return
	}
	// The tools, each with a checkbox for whether the bot may use it and what it does. A tool
	// beyond the plugin's level stays off until the level reaches it. Long lists scroll.
	k.row(ui.Scroll(c.Key("tools")).MaxHeight(168).Padding(5, 14, 5, 54)).Children(func() {
		for _, tool := range plugin.Tools {
			capability := tool.Capability
			if capability == "" {
				capability = "write"
			}
			reached := level.Allows(capability)
			_, picked := st.chosen[plugin.ID]
			on := reached && (!picked || slices.Contains(st.chosen[plugin.ID], tool.Name))
			ui.Row(c.Key(tool.Name)).Height(24).Gap(8).Children(func() {
				box := ui.CheckboxBase(c, &on).Grow(1).Shrink(1).MinWidth(0).Gap(7).Label(tool.Shown()).Disabled(!reached).
					OnChange(func() { st.toggleTool(plugin, tool.Name, on) })
				if tool.Description != "" {
					box.Tooltip(tool.Description)
				}
				box.Children(func() {
					mark := ui.Box(c).Size(14, 14).Radius(3.5).Shrink(0).Center()
					if on {
						mark.Background(p.Accent).TextColor(p.AccentText).Children(func() { symbol(c, "checkmark", 10, 3) })
					} else {
						mark.Background(p.Field).Border(1, p.Label3)
					}
					ui.Text(c, tool.Shown()).FontSize(12).TextColor(p.Label).SingleLine()
				})
				does := map[string]string{"read": L("Reads"), "draft": L("Drafts")}[capability]
				if does == "" {
					does = L("Changes")
				}
				ui.Text(c, does).Shrink(0).FontSize(11).TextColor(p.Label3).SingleLine()
			})
		}
	})
}
