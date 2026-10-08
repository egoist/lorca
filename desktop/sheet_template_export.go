package main

import (
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

type templateExportChoice struct {
	id, title string
	selected  bool
}
type templateExportGroup struct {
	key, title string
	choices    []templateExportChoice
}

type templateExportState struct {
	botID, name            string
	contents               model.TemplateContents
	groups                 []templateExportGroup
	profile                bool
	preview                *model.TemplatePreview
	reviewed               bool
	loading, busy, writing bool
	generation             int
	status                 string
	failed                 bool
}

func (w *appWindow) presentTemplateExport(botID string) {
	bot := store.Bot(botID)
	if bot == nil {
		return
	}
	st := &templateExportState{botID: botID, name: bot.Name, loading: true}
	s := w.present(func(c *ui.Context, s *sheet) { st.view(c, w, s) }, nil)
	st.load(s)
}

func (st *templateExportState) load(s *sheet) {
	st.generation++
	generation := st.generation
	st.loading, st.failed = true, false
	store.TemplateContents(st.botID, func(contents model.TemplateContents, err error) {
		if s.window == nil || generation != st.generation {
			return
		}
		st.loading = false
		if err != nil {
			st.status, st.failed = model.ErrorText(err), true
			return
		}
		st.contents = contents
		st.groups = []templateExportGroup{{key: "skill_ids", title: "Reusable skills"}, {key: "memory_ids", title: "Selected memories"}, {key: "routine_ids", title: "Routines"}, {key: "requirement_ids", title: "Integration requirements"}}
		for _, item := range contents.Skills {
			st.groups[0].choices = append(st.groups[0].choices, templateExportChoice{id: item.ID, title: item.Content.Name})
		}
		for _, item := range contents.Memories {
			st.groups[1].choices = append(st.groups[1].choices, templateExportChoice{id: item.ID, title: item.Content})
		}
		for _, item := range contents.Routines {
			st.groups[2].choices = append(st.groups[2].choices, templateExportChoice{id: item.ID, title: item.Content.Name})
		}
		for _, item := range contents.Requirements {
			st.groups[3].choices = append(st.groups[3].choices, templateExportChoice{id: item.ServiceID, title: item.ServiceID})
		}
		st.profile = false
		st.invalidateReview()
		st.status = strings.Join(contents.Notes, "\n")
	})
}

func (st *templateExportState) selection() model.TemplateSelection {
	selection := model.TemplateSelection{Profile: st.profile}
	for _, group := range st.groups {
		for _, choice := range group.choices {
			if !choice.selected {
				continue
			}
			switch group.key {
			case "skill_ids":
				selection.SkillIDs = append(selection.SkillIDs, choice.id)
			case "memory_ids":
				selection.MemoryIDs = append(selection.MemoryIDs, choice.id)
			case "routine_ids":
				selection.RoutineIDs = append(selection.RoutineIDs, choice.id)
			case "requirement_ids":
				selection.RequirementIDs = append(selection.RequirementIDs, choice.id)
			}
		}
	}
	return selection
}

func (st *templateExportState) invalidateReview() {
	st.generation++
	st.preview, st.reviewed, st.failed = nil, false, false
	st.status = strings.Join(st.contents.Notes, "\n")
}

func (st *templateExportState) requestPreview(s *sheet) {
	st.generation++
	generation := st.generation
	st.busy, st.reviewed, st.failed = true, false, false
	st.status = L("Loading reusable content…")
	store.PreviewTemplateExport(st.botID, st.selection(), func(preview model.TemplatePreview, err error) {
		if s.window == nil || generation != st.generation {
			return
		}
		st.busy = false
		if err != nil {
			st.status, st.failed = model.ErrorText(err), true
			return
		}
		if preview.Template == nil || preview.Digest == "" || preview.Visibility != "private_file" {
			st.status, st.failed = L("Couldn't read the template response."), true
			return
		}
		st.preview = &preview
		st.status = L("Review every selected instruction, memory, resource, and script. Credential-like text is redacted; personal content can remain.")
	})
}

func (st *templateExportState) chooseFile(w *appWindow, s *sheet) {
	if st.preview == nil || !st.reviewed || st.busy {
		return
	}
	st.busy = true
	chooseTemplateDestination(w.win, st.name, func(path string, err error) {
		if s.window == nil {
			return
		}
		if err != nil {
			st.busy, st.failed, st.status = false, true, model.ErrorText(err)
			return
		}
		if path == "" {
			st.busy = false
			return
		}
		path, exists, confirm := templateDestination(path)
		if confirm {
			w.showAlert(alertOptions{Message: L("Replace the existing private template?"), Informative: path, Buttons: []alertButton{{Title: L("Replace")}, {Title: L("Cancel")}}}, func(index int) {
				if s.window == nil {
					return
				}
				if index == 0 {
					st.write(s, path, true)
				} else {
					st.busy = false
				}
			})
			return
		}
		st.write(s, path, exists)
	})
}

func (st *templateExportState) write(s *sheet, path string, overwrite bool) {
	if st.preview == nil || !st.reviewed {
		st.busy = false
		return
	}
	st.writing = true
	options := model.TemplateExportOptions{BotID: st.botID, Selection: st.selection(), Path: path, ExpectedDigest: st.preview.Digest, Reviewed: true, Overwrite: overwrite}
	store.ExportTemplate(options, func(_ model.TemplateExportResult, err error) {
		if s.window == nil {
			return
		}
		st.busy, st.writing = false, false
		if err != nil {
			st.invalidateReview()
			st.status, st.failed = model.ErrorText(err), true
			return
		}
		s.dismiss()
	})
}

func (st *templateExportState) view(c *ui.Context, w *appWindow, s *sheet) {
	p := colors(c)
	confirm := L("Preview Contents")
	disabled := st.loading || st.busy || st.selection().Empty()
	if st.preview != nil {
		confirm, disabled = L("Save Private File…"), st.busy || !st.reviewed
	}
	result := sheetFrame(c, sheetOptions{Title: L("Export Bot Template"), Subtitle: L("Choose reusable content from %@. Saving creates a private file; share it only with people you choose.", st.name), Width: 640, Confirm: confirm, ConfirmDisabled: disabled, Leading: func() {
		if st.failed && st.preview == nil && pushButton(c, L("Reload Contents"), pushOptions{Disabled: st.busy || st.loading}).Clicked() {
			st.load(s)
		}
	}}, func() {
		if st.loading {
			ui.Text(c, L("Loading reusable content…")).TextColor(p.Label2)
		} else {
			ui.Scroll(c.Key("template-choices")).Height(190).Children(func() {
				ui.Column(c).Gap(6).Children(func() {
					ui.Checkbox(c.Key("profile"), &st.profile, L("Profile and instructions: %@", st.contents.Profile.Name)).Disabled(st.busy).OnChange(st.invalidateReview)
					for i := range st.groups {
						group := &st.groups[i]
						ui.Text(c, templateGroupTitle(group.key)).FontSize(12).FontWeight(600)
						if len(group.choices) == 0 {
							ui.Text(c, L("None available")).FontSize(textCaption).TextColor(p.Label2)
						}
						for j := range group.choices {
							choice := &group.choices[j]
							ui.Checkbox(c.Key(group.key+":"+choice.id), &choice.selected, strings.ReplaceAll(choice.title, "\n", " ")).Tooltip(choice.title).Disabled(st.busy).OnChange(st.invalidateReview)
						}
					}
				})
			})
		}
		text := L("Select content, then preview the complete private file before saving.")
		if st.preview != nil {
			text = st.preview.Text()
		}
		templatePreviewView(c.Key("template-preview"), text, 205)
		if st.status != "" {
			tint := p.Label2
			if st.failed {
				tint = p.Red
			}
			ui.Text(c, st.status).FontSize(textCaption).LineHeight(1.4).TextColor(tint)
		}
		ui.Checkbox(c.Key("template-review"), &st.reviewed, L("I reviewed the selected content for personal information.")).Disabled(st.preview == nil || st.busy)
	})
	if result.Cancelled && !st.writing {
		s.dismiss()
	}
	if result.Confirmed && !st.busy && !st.loading {
		if st.preview == nil {
			st.requestPreview(s)
		} else if st.reviewed {
			st.chooseFile(w, s)
		}
	}
}

func templateGroupTitle(key string) string {
	switch key {
	case "skill_ids":
		return L("Reusable skills")
	case "memory_ids":
		return L("Selected memories")
	case "routine_ids":
		return L("Routines")
	default:
		return L("Integration requirements")
	}
}

func templatePreviewView(c *ui.Context, text string, height float32) {
	p := colors(c)
	ui.Scroll(c).Height(height).Padding(8).Background(p.Field).Border(1, p.FieldBorder).Radius(6).Label(L("Template contents preview")).Children(func() {
		ui.Text(c, text).FontSize(12).LineHeight(1.4).Selectable()
	})
}
