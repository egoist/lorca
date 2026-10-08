package main

import (
	"path/filepath"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

type templateImportState struct {
	path, name, runnerID string
	provider             model.ProviderKind
	loadedName           bool
	mappings             map[string]string
	preview              *model.TemplatePreview
	reviewed             bool
	busy, importing      bool
	generation           int
	status               string
	failed               bool
	onCreate             func(string)
}

func (w *appWindow) presentTemplateImport(onCreate func(string)) {
	chooseTemplateSource(w.win, func(path string, err error) {
		if w.win != nil && w.win.IsDestroyed() {
			return
		}
		if err != nil {
			w.showNote(L("Import Bot Template"), model.ErrorText(err))
			return
		}
		if path != "" {
			w.presentTemplateImportPath(path, onCreate)
		}
	})
}

func (w *appWindow) presentTemplateImportPath(path string, onCreate func(string)) {
	st := &templateImportState{path: path, provider: store.PreferredProvider(), mappings: map[string]string{}, onCreate: onCreate}
	runners := store.Runners()
	if len(runners) > 0 {
		st.runnerID = runners[0].ID
	}
	for _, runner := range runners {
		if runner.IsThisDevice {
			st.runnerID = runner.ID
			break
		}
	}
	s := w.present(func(c *ui.Context, s *sheet) { st.view(c, s) }, nil)
	st.refresh(s)
}

func (st *templateImportState) options() model.TemplateImportOptions {
	options := model.TemplateImportOptions{Path: st.path, RunnerID: st.runnerID, Provider: st.provider, Mappings: st.mappings}
	if st.loadedName {
		name := st.name
		options.Name = &name
	}
	return options
}

func (st *templateImportState) refresh(s *sheet) {
	if st.importing {
		return
	}
	st.generation++
	generation := st.generation
	st.reviewed, st.busy, st.failed, st.preview = false, true, false, nil
	st.status = L("Validating the private file and recipient connections…")
	store.PreviewTemplateImport(st.options(), func(preview model.TemplatePreview, err error) {
		if s.window == nil || generation != st.generation {
			return
		}
		st.busy = false
		if err != nil {
			st.status, st.failed = model.ErrorText(err), true
			return
		}
		st.preview = &preview
		if !st.loadedName {
			st.name, st.loadedName = preview.Name(), true
		}
		st.status = strings.Join(preview.Issues, "\n")
		if st.status == "" {
			st.status = L("Routines stay paused. Review instructions and scripts before running the new bot.")
		}
	})
}

func (st *templateImportState) canCreate() bool {
	return !st.busy && !st.importing && st.reviewed && strings.TrimSpace(st.name) != "" && st.runnerID != "" && st.preview != nil && st.preview.Template != nil && st.preview.CanImport && st.preview.Digest != "" && len(st.preview.Issues) == 0
}

func (st *templateImportState) create(s *sheet) {
	if !st.canCreate() {
		return
	}
	options := st.options()
	options.Reviewed, options.ExpectedDigest = true, st.preview.Digest
	st.importing, st.busy, st.failed = true, true, false
	st.status = L("Creating independent bot…")
	store.ImportTemplate(options, func(chatID string, err error) {
		if s.window == nil {
			return
		}
		st.importing, st.busy = false, false
		if err != nil {
			st.reviewed, st.failed, st.status = false, true, model.ErrorText(err)
			return
		}
		s.dismiss()
		if st.onCreate != nil {
			st.onCreate(chatID)
		}
	})
}

func (st *templateImportState) view(c *ui.Context, s *sheet) {
	p := colors(c)
	result := sheetFrame(c, sheetOptions{Title: L("Import Bot Template"), Subtitle: L("Review %@ and choose your own Runner and connections. Import creates an independent bot with its routines paused.", filepath.Base(st.path)), Width: 640, Confirm: L("Create Independent Bot"), ConfirmDisabled: !st.canCreate(), ReturnInContent: true, Leading: func() {
		if pushButton(c, L("Refresh Preview"), pushOptions{Disabled: st.importing}).Clicked() {
			st.refresh(s)
		}
	}}, func() {
		newBotRow(c, L("Name"), false, func() {
			textField(c.Key("template-name"), &st.name, fieldOptions{Label: L("New bot name"), Placeholder: L("New bot name"), Disabled: st.importing}).Grow(1).OnChange(func() { st.loadedName = true; st.refresh(s) })
		})
		newBotRow(c, L("Runner"), false, func() {
			var options []popUpOption
			for _, runner := range store.Runners() {
				options = append(options, popUpOption{Value: runner.ID, Label: runner.Name})
			}
			if picked, changed, _ := popUpButton(c.Key("template-runner"), popUp{Options: options, Value: st.runnerID, Width: 514, Label: L("Runner"), Disabled: st.importing}); changed {
				st.runnerID = picked
				st.mappings = map[string]string{}
				st.refresh(s)
			}
		})
		newBotRow(c, L("Provider"), false, func() {
			var options []popUpOption
			for _, kind := range store.ProviderKinds() {
				options = append(options, popUpOption{Value: kind, Label: model.ProviderName(kind, store.Providers)})
			}
			if picked, changed, _ := popUpButton(c.Key("template-provider"), popUp{Options: options, Value: st.provider, Width: 514, Label: L("Provider"), Disabled: st.importing}); changed {
				st.provider = picked
				st.reviewed = false
			}
		})
		if st.preview != nil {
			if len(st.preview.Requirements) == 0 {
				ui.Text(c, L("No integration requirements")).FontSize(textCaption).TextColor(p.Label2)
			}
			for _, requirement := range st.preview.Requirements {
				service := requirement.ServiceID
				ui.Column(c.Key("requirement:" + service)).Gap(5).Children(func() {
					ui.Text(c, L("Connection for %@", service)).FontSize(12).TextColor(p.Label2)
					options := []popUpOption{{Value: "", Label: L("Choose your connection…")}}
					for _, candidate := range requirement.Candidates {
						label := firstNonEmpty(candidate.Name, candidate.ID)
						if candidate.State != "ready" {
							label += " · " + candidate.Detail
						}
						options = append(options, popUpOption{Value: candidate.ID, Label: label})
					}
					if picked, changed, _ := popUpButton(c.Key("connection:"+service), popUp{Options: options, Value: st.mappings[service], Width: 600, Label: L("Connection for %@", service), Disabled: st.importing}); changed {
						if picked == "" {
							delete(st.mappings, service)
						} else {
							st.mappings[service] = picked
						}
						st.refresh(s)
					}
				})
			}
		}
		text := ""
		if st.preview != nil {
			text = st.preview.Text()
		}
		templatePreviewView(c.Key("template-import-preview"), text, 235)
		if st.status != "" {
			tint := p.Label2
			if st.failed {
				tint = p.Red
			} else if st.preview != nil && len(st.preview.Issues) > 0 {
				tint = p.Orange
			}
			ui.Text(c, st.status).FontSize(textCaption).LineHeight(1.4).TextColor(tint)
		}
		ui.Checkbox(c.Key("template-import-review"), &st.reviewed, L("I reviewed the contents and selected my own connections.")).Disabled(st.busy || st.importing || st.preview == nil || !st.preview.CanImport || len(st.preview.Issues) > 0)
	})
	if result.Cancelled && !st.importing {
		s.dismiss()
	}
	if result.Confirmed {
		st.create(s)
	}
}
