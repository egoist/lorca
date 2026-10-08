package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

type botAccessSheet struct {
	botID, name             string
	initial                 *model.BotPermissions
	draft                   *model.AccessDraft
	loading, saving, closed bool
	loads                   int
	problem                 string
}

// The editor owns values across frames. Replies contain data only, arrive on the store's
// ordered main-thread queue, and are ignored after dismissal or a newer catalog request.
func (w *appWindow) presentBotAccess(botID string) *botAccessSheet {
	bot := store.Bot(botID)
	if bot == nil {
		return nil
	}
	st := &botAccessSheet{botID: botID, name: bot.Name, initial: bot.Permissions.Clone(), draft: model.NewAccessDraft(bot.Permissions)}
	if runner := store.Device(bot.RunnerID); runner != nil {
		fallback := model.BotPermissionCatalog{}
		for _, plugin := range runner.Plugins {
			fallback.Connections = append(fallback.Connections, model.PermissionConnection{ID: plugin.ID, Name: plugin.Name})
		}
		st.draft.MergeCatalog(fallback)
	}
	w.present(st.view, func() { st.closed = true })
	st.load()
	return st
}

func (st *botAccessSheet) load() {
	st.loads++
	load := st.loads
	st.loading, st.problem = true, ""
	store.BotPermissionCatalog(st.botID, func(catalog model.BotPermissionCatalog, err error) {
		if st.closed || load != st.loads {
			return
		}
		st.loading = false
		if err != nil {
			st.problem = model.ErrorText(err)
			return
		}
		st.draft.MergeCatalog(catalog)
	})
}

func accessCheck(c *ui.Context, key, title, label string, checked *bool, disabled bool) {
	p := colors(c)
	row := ui.CheckboxBase(c.Key(key), checked).Height(28).Gap(8).Label(label).Disabled(disabled).OnChange(func() {})
	_ = row.Changed() // apply bound input before drawing this frame's checkmark
	row.Children(func() {
		box := ui.Box(c).Size(15, 15).Radius(4).Shrink(0).Center()
		if *checked {
			box.Background(p.Accent).TextColor(p.AccentText).Children(func() { symbol(c, "checkmark", 11, 2.5) })
		} else {
			box.Background(p.Field).Border(1, p.Label3)
		}
		ui.Text(c, title).FontSize(12.5).TextColor(p.Label).Shrink(1).MinWidth(0)
	})
}

func accessCapability(value string) string {
	switch value {
	case "read":
		return L("Read")
	case "draft":
		return L("Draft")
	default:
		return L("Write")
	}
}

func (st *botAccessSheet) view(c *ui.Context, s *sheet) {
	p := colors(c)
	d := st.draft
	result := sheetFrame(c, sheetOptions{Title: L("Access"), Subtitle: st.name, Width: 580, Confirm: L("Save"), ConfirmDisabled: st.saving}, func() {
		ui.Text(c, L("These limits apply before Auto-review. Only you can change them. Read, draft, and write are separate grants.")).FontSize(textCaption).TextColor(p.Label2).LineHeight(1.4)
		if st.loading {
			ui.Text(c, L("Loading available tools…")).FontSize(textCaption).TextColor(p.Label2)
		}
		if st.problem != "" {
			ui.Text(c, st.problem).FontSize(textCaption).TextColor(p.Red).LineHeight(1.4)
			if pushButton(c.Key("reload-catalog"), L("Retry"), pushOptions{Small: true, Disabled: st.loading || st.saving}).Clicked() {
				st.load()
			}
		}
		accessCheck(c, "all-connections", L("All connections"), L("All connections"), &d.AllConnections, st.saving)
		for _, instance := range d.Instances() {
			ui.Column(c.Key("connection:" + instance.ID)).Gap(3).Children(func() {
				ui.Text(c, instance.Name).FontSize(12.5).FontWeight(600)
				ui.Text(c, instance.ID).FontSize(textCaption).TextColor(p.Label3).Selectable()
				ui.Row(c).Gap(12).Children(func() {
					accessCheck(c, "read", L("Read"), L("Allow %@ for %@", L("Read"), instance.Name), &instance.Read, d.AllConnections || st.saving)
					accessCheck(c, "draft", L("Draft"), L("Allow %@ for %@", L("Draft"), instance.Name), &instance.Draft, d.AllConnections || st.saving)
					accessCheck(c, "write", L("Write"), L("Allow %@ for %@", L("Write"), instance.Name), &instance.Write, d.AllConnections || st.saving)
				})
				accessCheck(c, "all-tools", L("All connection tools"), L("All tools for %@", instance.Name), &instance.AllTools, d.AllConnections || st.saving)
				for _, tool := range instance.OfferedTools() {
					accessCheck(c, "tool:"+tool.Name, tool.Name+" · "+accessCapability(tool.Capability), L("Allow %@ on %@", tool.Name, instance.Name), &tool.Selected, d.AllConnections || instance.AllTools || st.saving)
				}
			})
		}
		accessCheck(c, "all-local", L("All local tools"), L("All local tools"), &d.AllTools, st.saving)
		ui.Column(c.Key("local-tools")).Gap(1).Children(func() {
			for _, tool := range d.LocalTools() {
				accessCheck(c, tool.Name, tool.Name, L("Allow local tool %@", tool.Name), &tool.Selected, d.AllTools || st.saving)
			}
		})
		ui.Row(c.Key("filesystem")).Gap(12).AlignItems(ui.Center).Children(func() {
			ui.Text(c, L("Filesystem")).FontSize(12).TextColor(p.Label2)
			if picked, changed, _ := popUpButton(c, popUp{Value: d.Filesystem, Label: L("Filesystem access"), Disabled: st.saving,
				Options: []popUpOption{{Value: "none", Label: L("None")}, {Value: "read", Label: L("Read")}, {Value: "write", Label: L("Read and write")}}, Width: 200}); changed {
				d.Filesystem = picked
			}
		})
		accessCheck(c, "shell", L("Allow shell commands"), L("Allow shell commands"), &d.Shell, st.saving)
		ui.Text(c, L("Shell commands and filesystem tools use the Runner's user account. Shell access can reach credentials and bypass connection limits. A working directory provides no isolation; use an isolated process or a dedicated Runner for stronger separation.")).FontSize(textCaption).TextColor(p.Label2).LineHeight(1.4)
		ui.Text(c, L("Unlisted connections and tools are denied when an allowlist is selected. Saved tool labels are informational; the CLI checks live capabilities before a call.")).FontSize(textCaption).TextColor(p.Label2).LineHeight(1.4)
	})
	switch {
	case result.Cancelled:
		s.dismiss()
	case result.Confirmed:
		bot := store.Bot(st.botID)
		if bot == nil || !model.EqualBotPermissions(bot.Permissions, st.initial) {
			st.problem = L("Access changed while this sheet was open. Reopen it to review the current settings.")
			return
		}
		st.saving = true
		store.SetBotPermissions(st.botID, st.draft.Policy(), func(err error) {
			if st.closed {
				return
			}
			st.saving = false
			if err != nil {
				st.problem = model.ErrorText(err)
				return
			}
			s.dismiss()
		})
	}
}
