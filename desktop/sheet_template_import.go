package main

import (
	"fmt"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Adding a bot from a template, a link someone shared or a file, after the macOS app's
// TemplateImportViewController: its name, the Runner it runs on, the provider, and for each plugin
// it uses one of that Runner's own connections. The new bot shares nothing with the one it came
// from, and its routines start paused.

type templateImportState struct {
	// from is the From field: a pasted link, or the file Choose File… picked. link and path are
	// the template it opened.
	from, link, path string
	// nameEdited is a name the user typed, which a template opened later leaves alone.
	nameEdited     bool
	name, runnerID string
	provider       model.ProviderKind
	// mappings is the connection each plugin uses, by plugin: the CLI's pick until the user makes one.
	mappings map[string]string
	preview  *model.TemplatePreview
	// named is whether the name field has taken the template's name.
	named      bool
	status     string
	failed     bool
	importing  bool
	generation int
	// previewed is the Runner's plugins as last previewed, so a change to them previews again.
	previewed string
	onCreate  func(string)
}

// presentTemplateImport is New Bot from Template, opening a shared bot's link (the one that opened
// the app) or a file when it is given one.
func (w *appWindow) presentTemplateImport(link, path string, onCreate func(string)) {
	st := &templateImportState{provider: store.PreferredProvider(), mappings: map[string]string{}, onCreate: onCreate}
	for i, runner := range store.Runners() {
		if i == 0 || runner.IsThisDevice {
			st.runnerID = runner.ID
		}
		if runner.IsThisDevice {
			break
		}
	}
	s := w.present(func(c *ui.Context, s *sheet) { st.view(c, w, s) }, nil)
	switch {
	case link != "":
		st.from = link
		st.open(s, link, "")
	case path != "":
		st.from = path
		st.open(s, "", path)
	}
}

// isTemplateLink is whether the From field holds a whole link: its page and the key after #.
func isTemplateLink(text string) bool {
	return strings.Contains(text, "/t/") && strings.Contains(text, "#")
}

// open reads another template: what it sets up shows again from the start.
func (st *templateImportState) open(s *sheet, link, path string) {
	if link == st.link && path == st.path {
		return
	}
	st.link, st.path = link, path
	st.mappings = map[string]string{}
	st.preview, st.named = nil, false
	st.status, st.failed = L("Loading…"), false
	st.refresh(s)
}

func (st *templateImportState) runner() *model.Device { return store.Device(st.runnerID) }

// runnerPlugins is the picked Runner's plugins and their states, to notice a change.
func (st *templateImportState) runnerPlugins() string {
	runner := st.runner()
	if runner == nil {
		return ""
	}
	var parts []string
	for _, plugin := range runner.Plugins {
		parts = append(parts, fmt.Sprintf("%s=%v", plugin.ID, plugin.State))
	}
	return strings.Join(parts, ",")
}

func (st *templateImportState) refresh(s *sheet) {
	if st.link == "" && st.path == "" {
		return
	}
	st.generation++
	generation := st.generation
	st.previewed = st.runnerPlugins()
	options := model.TemplateImportOptions{Path: st.path, Link: st.link, RunnerID: st.runnerID, Mappings: st.mappings}
	store.PreviewTemplateImport(options, func(preview model.TemplatePreview, err error) {
		if s.window == nil || generation != st.generation {
			return
		}
		if err != nil {
			st.preview, st.status, st.failed = nil, model.ErrorText(err), true
			return
		}
		st.preview, st.status, st.failed = &preview, "", false
		if preview.Template == nil {
			// A file or link that holds no template only says why.
			st.status, st.failed = strings.Join(preview.Issues, "\n"), true
			return
		}
		if !st.named && !st.nameEdited && preview.Template.Profile != nil {
			st.name = preview.Template.Profile.Name
		}
		st.named = true
		for _, plugin := range preview.Requirements {
			if plugin.Selected != "" {
				st.mappings[plugin.ServiceID] = plugin.Selected
			}
		}
	})
}

// note is the line under the contents: what keeps the import from going ahead, else that routines
// start paused.
func (st *templateImportState) note() (string, bool) {
	if st.status != "" {
		return st.status, st.failed
	}
	preview := st.preview
	runner := st.runner()
	switch {
	case preview == nil:
		return "", false
	case len(store.Runners()) == 0 || runner == nil:
		return L("Pair a Runner first: a bot runs on a computer."), true
	case len(preview.Issues) > 0:
		return strings.Join(preview.Issues, "\n"), true
	}
	for _, plugin := range preview.Requirements {
		switch {
		case plugin.Ready():
			continue
		case len(plugin.Candidates) == 0:
			return L("Add %@ to %@ from the Marketplace first.", plugin.Name, runner.Name), true
		case plugin.Selected == "":
			return L("Choose the %@ connection this bot uses.", plugin.Name), true
		default:
			return L("Finish setting up %@ on %@ first.", plugin.Name, runner.Name), true
		}
	}
	if preview.Template != nil && len(preview.Template.Routines) > 0 {
		return L("Routines start paused."), false
	}
	return "", false
}

func (st *templateImportState) canCreate() bool {
	return !st.importing && st.preview != nil && st.preview.CanImport && strings.TrimSpace(st.name) != "" && st.runner() != nil
}

func (st *templateImportState) create(s *sheet) {
	if !st.canCreate() {
		return
	}
	options := model.TemplateImportOptions{Path: st.path, Link: st.link, RunnerID: st.runnerID, Name: strings.TrimSpace(st.name), Provider: st.provider,
		Mappings: st.mappings, ExpectedDigest: st.preview.Digest, Reviewed: true}
	st.importing = true
	store.ImportTemplate(options, func(chatID string, err error) {
		if s.window == nil {
			return
		}
		st.importing = false
		if err != nil {
			st.status, st.failed = model.ErrorText(err), true
			return
		}
		s.dismiss()
		if st.onCreate != nil {
			st.onCreate(chatID)
		}
	})
}

// sections are the file's contents, read-only.
func (st *templateImportState) sections() []templateSection {
	doc := st.preview.Template
	if doc == nil {
		return nil
	}
	profile := templateSection{title: L("Profile")}
	if doc.Profile != nil {
		profile.items = []templateItem{templateProfile(*doc.Profile, nil)}
	}
	skills := templateSection{title: L("Skills")}
	for i, skill := range doc.Skills {
		skills.items = append(skills.items, templateItem{id: fmt.Sprint("skill-", i), title: skill.Name, detail: skill.Description, lines: 1})
	}
	memories := templateSection{title: L("Memories")}
	for i, memory := range doc.Memories {
		memories.items = append(memories.items, templateMemory(fmt.Sprint("memory-", i), memory, nil))
	}
	routines := templateSection{title: L("Routines")}
	for i, routine := range doc.Routines {
		routines.items = append(routines.items, templateRoutine(fmt.Sprint("routine-", i), routine, routine.ScheduleText, nil))
	}
	return []templateSection{profile, skills, routines, memories}
}

func (st *templateImportState) view(c *ui.Context, w *appWindow, s *sheet) {
	p := colors(c)
	const width = 480
	// The pop-ups fill the row after the label, as wide as the sheet's content allows.
	fill := float32(width - 40 - formLabelWidth - 10)
	// A plugin added or signed in on the Runner meanwhile changes what the import needs.
	if !st.importing && st.preview != nil && st.runnerPlugins() != st.previewed {
		st.refresh(s)
	}
	text, warning := st.note()
	result := sheetFrame(c, sheetOptions{
		Title:           L("New Bot from Template"),
		Width:           width,
		Confirm:         L("Create Bot"),
		ConfirmDisabled: !st.canCreate(),
	}, func() {
		formRow(c, L("From"), false, func() {
			ui.Row(c).Grow(1).MinWidth(0).Gap(8).Children(func() {
				field := textField(c.Key("template-from"), &st.from, fieldOptions{Placeholder: L("Paste a link to a shared bot"), Label: L("From"), Disabled: st.importing}).Grow(1).MinWidth(0)
				field.OnChange(func() {
					if text := strings.TrimSpace(st.from); isTemplateLink(text) {
						st.open(s, text, "")
					}
				})
				if pushButton(c, L("Choose File…"), pushOptions{Disabled: st.importing}).Clicked() {
					chooseTemplateSource(w.win, func(path string, err error) {
						if s.window == nil || path == "" {
							return
						}
						if err != nil {
							st.status, st.failed = model.ErrorText(err), true
							return
						}
						st.from = path
						st.open(s, "", path)
					})
				}
			})
		})
		if st.preview == nil || st.preview.Template == nil {
			if text != "" {
				tint := p.Label3
				if st.failed {
					tint = p.Red
				}
				ui.Text(c, text).FontSize(11.5).LineHeight(1.4).TextColor(tint)
			}
			return
		}
		formRow(c, L("Name"), false, func() {
			textField(c.Key("template-name"), &st.name, fieldOptions{Placeholder: L("Name"), Label: L("Name"), Disabled: st.importing}).Grow(1).MinWidth(0).OnChange(func() { st.nameEdited = true })
		})
		formRow(c, L("Runner"), false, func() {
			var options []popUpOption
			for _, device := range store.Runners() {
				label := device.Name
				if device.IsThisDevice {
					label = L("%@ (this computer)", device.Name)
				}
				options = append(options, popUpOption{Value: device.ID, Label: label})
			}
			if picked, changed, _ := popUpButton(c.Key("template-runner"), popUp{Options: options, Value: st.runnerID, Width: fill, Label: L("Runner"), Disabled: st.importing || len(options) == 0}); changed {
				st.runnerID = picked
				st.mappings = map[string]string{}
				st.refresh(s)
			}
		})
		formRow(c, L("Provider"), false, func() {
			kinds := store.ProviderKinds()
			options := make([]popUpOption, 0, len(kinds))
			for _, kind := range kinds {
				options = append(options, popUpOption{Value: kind, Label: model.ProviderName(kind, store.Providers) + " (" + model.ProviderSubtitle(kind) + ")"})
			}
			if picked, changed, _ := popUpButton(c.Key("template-provider"), popUp{Options: options, Value: st.provider, Width: fill, Label: L("Provider"), Disabled: st.importing}); changed {
				st.provider = picked
			}
		})
		if st.preview != nil {
			// A line per plugin: a pop-up of the Runner's connections for it, or that it has none.
			for _, plugin := range st.preview.Requirements {
				formRow(c.Key("plugin:"+plugin.ServiceID), plugin.Name, false, func() {
					if len(plugin.Candidates) == 0 {
						name := ""
						if runner := st.runner(); runner != nil {
							name = runner.Name
						}
						ui.Text(c, L("Not on %@", name)).Padding(4, 0).FontSize(13).TextColor(p.Orange).SingleLine()
						return
					}
					var options []popUpOption
					if plugin.Selected == "" {
						options = append(options, popUpOption{Value: "", Label: L("Choose…")})
					}
					for _, candidate := range plugin.Candidates {
						label := candidate.Name
						if candidate.State != "ready" {
							label = L("%@ (needs setup)", candidate.Name)
						}
						options = append(options, popUpOption{Value: candidate.ID, Label: label})
					}
					if picked, changed, _ := popUpButton(c.Key("connection:"+plugin.ServiceID), popUp{Options: options, Value: plugin.Selected, Width: fill, Label: plugin.Name, Disabled: st.importing}); changed && picked != "" {
						st.mappings[plugin.ServiceID] = picked
						st.refresh(s)
					}
				})
			}
			ui.Column(c).Margin(4, 0, 0, 0).Children(func() {
				templateList(c.Key("template-contents"), 260, st.sections(), false, nil)
			})
		}
		if text != "" {
			tint := p.Label3
			if st.failed {
				tint = p.Red
			} else if warning {
				tint = p.Orange
			}
			ui.Text(c, text).FontSize(11.5).LineHeight(1.4).TextColor(tint)
		}
	})
	switch {
	case result.Cancelled && !st.importing:
		s.dismiss()
	case result.Confirmed:
		st.create(s)
	}
}
