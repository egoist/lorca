package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

func playbookScopeLabel(scope model.PlaybookScope) string {
	if scope.Kind == "project" {
		name := L("Group")
		if chat := store.Chat(scope.ID); chat != nil {
			name = firstNonEmpty(chat.CustomTitle, name)
		}
		return L("Project · %@", name)
	}
	name := scope.ID
	if bot := store.Bot(scope.ID); bot != nil {
		name = bot.Name
	}
	return L("Bot · %@", name)
}

func (m *mainWindow) inspectorPlaybooks(c *ui.Context, chat *model.Chat) {
	section(c, L("Playbooks"), sectionCaption, nil, func(k *card) {
		value := L("This bot")
		if chat.IsGroup() {
			value = L("Project and bots")
		}
		if summaryActionRow(c, k, L("Skills"), value, L("Manage…")) {
			m.presentPlaybooks(chat.ID)
		}
	})
}

func playbookScopePicker(c *ui.Context, scopes []model.PlaybookScope, current model.PlaybookScope, disabled bool) (model.PlaybookScope, bool) {
	options := make([]popUpOption, 0, len(scopes))
	for _, scope := range scopes {
		options = append(options, popUpOption{Value: scope.Key(), Label: playbookScopeLabel(scope)})
	}
	picked, changed, _ := popUpButton(c.Key("scope"), popUp{Options: options, Value: current.Key(), Disabled: disabled, Label: L("Scope")})
	for _, scope := range scopes {
		if changed && picked == scope.Key() {
			return scope, true
		}
	}
	return current, false
}

type playbooksManager struct {
	w                     *appWindow
	chatID                string
	scopes                []model.PlaybookScope
	scope                 model.PlaybookScope
	items                 []model.PlaybookSummary
	selected              string
	loading, busy, closed bool
	loads                 int
	problem               string
}

func (w *appWindow) presentPlaybooks(chatID string) *playbooksManager {
	s := &playbooksManager{w: w, chatID: chatID, scopes: store.PlaybookScopes(chatID)}
	if len(s.scopes) == 0 {
		return nil
	}
	s.scope = s.scopes[0]
	w.present(s.view, func() { s.closed = true; s.loads++ })
	s.reload()
	return s
}

func (s *playbooksManager) reload() {
	if s.closed {
		return
	}
	s.loads++
	generation, scope := s.loads, s.scope
	s.loading, s.problem, s.selected = true, "", ""
	s.items = nil
	store.Playbooks(scope, func(items []model.PlaybookSummary, err error) {
		if s.closed || generation != s.loads || scope != s.scope {
			return
		}
		s.loading = false
		if err != nil {
			s.problem = model.ErrorText(err)
			return
		}
		s.items = items
	})
}

func (s *playbooksManager) picked() *model.PlaybookSummary {
	for _, item := range s.items {
		if item.ID == s.selected {
			return &item
		}
	}
	return nil
}

func (s *playbooksManager) open() {
	item := s.picked()
	if item == nil || s.busy {
		return
	}
	s.busy = true
	store.Playbook(item.Scope, item.ID, func(record model.PlaybookRecord, err error) {
		if s.closed {
			return
		}
		s.busy = false
		if err != nil {
			s.problem = model.ErrorText(err)
			return
		}
		s.w.presentPlaybookEditor(record.Scope, &record, s.reload)
	})
}

func (s *playbooksManager) remove() {
	item := s.picked()
	if item == nil || s.busy {
		return
	}
	s.w.showAlert(alertOptions{
		Message: L("Remove %@?", item.Name), Informative: L("This skill becomes unavailable to the bot. Its revision history records the removal."),
		Buttons: []alertButton{{Title: L("Remove"), Destructive: true}, {Title: L("Cancel")}},
	}, func(index int) {
		if index != 0 || s.closed {
			return
		}
		s.busy = true
		// Delete the version the list showed, with both guards; no fetch silently advances it.
		store.RemovePlaybook(*item, func(_ model.PlaybookRecord, err error) {
			if s.closed {
				return
			}
			s.busy = false
			if err != nil {
				s.problem = model.ErrorText(err)
				return
			}
			s.reload()
		})
	})
}

func (s *playbooksManager) export() {
	item := s.picked()
	if item == nil || item.Status != "saved" || s.busy {
		return
	}
	s.busy = true
	store.ExportPlaybook(*item, func(data json.RawMessage, err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.busy = false
			s.problem = model.ErrorText(err)
			return
		}
		parent, filename := s.w.win, item.Name+".lorca-playbook.json"
		go func() {
			path, err := mygo.Dialog.Save(mygo.SaveDialogOptions{Parent: parent, Title: L("Export…"), DefaultPath: filename,
				Message: L("Export this skill's current instructions and bundled files. Revision history and chat provenance stay private."),
				Filters: []mygo.FileFilter{{Name: L("Playbooks"), Extensions: []string{"json"}}}})
			if err == nil && path != "" {
				var formatted bytes.Buffer
				if err = json.Indent(&formatted, data, "", "  "); err == nil {
					err = os.WriteFile(path, formatted.Bytes(), 0o600)
				}
			}
			post(func() {
				if s.closed {
					return
				}
				s.busy = false
				if err != nil {
					s.problem = model.ErrorText(err)
				}
			})
		}()
	})
}

func (s *playbooksManager) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	picked := s.picked()
	result := sheetFrame(c, sheetOptions{Title: L("Playbooks"), Width: 640, Confirm: L("Done"), NoCancel: true,
		Subtitle: L("Reusable instructions, examples, references, and scripts. Drafts wait for your review; saved skills are available on later turns.")}, func() {
		if scope, changed := playbookScopePicker(c, s.scopes, s.scope, s.busy); changed {
			s.scope = scope
			s.reload()
		}
		ui.Scroll(c.Key("playbook-list")).Height(240).Background(p.Field).Radius(6).Border(1, p.FieldBorder).Children(func() {
			if s.loading {
				ui.Text(c, L("Loading…")).Padding(10).TextColor(p.Label2)
			}
			if !s.loading && len(s.items) == 0 {
				ui.Text(c, L("No skills in this scope. Create one or save a completed chat workflow.")).Padding(12).FontSize(12).TextColor(p.Label2)
			}
			for _, item := range s.items {
				title := item.Name
				if item.Status == "draft" {
					title += " · " + L("Draft")
				}
				row := ui.ButtonBase(c.Key(item.ID)).Padding(10).MinWidth(0).WidthPercent(100).Justify(ui.Start).Label(title).Disabled(s.busy)
				if s.selected == item.ID {
					row.Background(p.Hover)
				}
				row.Children(func() {
					ui.Column(c).Gap(3).MinWidth(0).Grow(1).Children(func() {
						ui.Text(c, title).FontSize(13).FontWeight(500).SingleLine()
						ui.Text(c, item.Description).FontSize(11.5).TextColor(p.Label2).SingleLine()
					})
				})
				if row.Clicked() {
					s.selected = item.ID
				}
			}
		})
		ui.Row(c).Gap(8).Children(func() {
			if pushButton(c, L("New Skill"), pushOptions{Disabled: s.busy}).Clicked() {
				s.w.presentPlaybookEditor(s.scope, nil, s.reload)
			}
			if pushButton(c, L("Edit / Review"), pushOptions{Disabled: picked == nil || s.busy}).Clicked() {
				s.open()
			}
			if pushButton(c, L("Export…"), pushOptions{Disabled: picked == nil || picked.Status != "saved" || s.busy}).Clicked() {
				s.export()
			}
			if pushButton(c, L("Remove"), pushOptions{Disabled: picked == nil || s.busy, Kind: buttonDestructive}).Clicked() {
				s.remove()
			}
			if pushButton(c, L("Reload"), pushOptions{Disabled: s.busy}).Clicked() {
				s.reload()
			}
		})
		if s.problem != "" {
			ui.Text(c, s.problem).FontSize(12).TextColor(p.Red).LineHeight(1.4)
		}
	})
	if result.Confirmed || result.Cancelled {
		sh.dismiss()
	}
}

type playbookFile struct {
	key        int
	name, text string
}

// Fields belong to this sheet's persistent state. File keys survive rename/removal and tab switches.
type playbookEditor struct {
	w            *appWindow
	scope        model.PlaybookScope
	previous     *model.PlaybookRecord
	content      model.PlaybookContent
	files        map[string][]*playbookFile
	selected     map[string]int
	nextFile     int
	tab          int
	history      string
	busy, closed bool
	problem      string
	onSaved      func()
}

func (w *appWindow) presentPlaybookEditor(scope model.PlaybookScope, record *model.PlaybookRecord, onSaved func()) *playbookEditor {
	s := &playbookEditor{w: w, scope: scope, onSaved: onSaved}
	s.apply(record)
	w.present(s.view, func() { s.closed = true })
	return s
}

func (s *playbookEditor) apply(record *model.PlaybookRecord) {
	s.previous = record
	s.content = model.PlaybookContent{}
	if record != nil && record.Content != nil {
		s.content = record.Content.Clone()
	}
	s.files, s.selected = map[string][]*playbookFile{}, map[string]int{}
	for kind, resources := range map[string][]model.PlaybookResource{"references": s.content.References, "scripts": s.content.Scripts} {
		for _, resource := range resources {
			s.nextFile++
			s.files[kind] = append(s.files[kind], &playbookFile{s.nextFile, strings.TrimPrefix(resource.Path, kind+"/"), resource.Text})
		}
		if len(s.files[kind]) > 0 {
			s.selected[kind] = s.files[kind][0].key
		}
	}
	s.history = ""
	if record != nil {
		revisions := slices.Clone(record.Revisions)
		slices.SortStableFunc(revisions, func(a, b model.PlaybookRevision) int {
			if a.Revision > b.Revision {
				return -1
			}
			if a.Revision < b.Revision {
				return 1
			}
			return 0
		})
		for i, r := range revisions {
			if i > 0 {
				s.history += "\n\n────────────\n\n"
			}
			s.history += L("Revision %d · %@ · %@", r.Revision, time.Unix(int64(r.CreatedAt), 0).Format("2006-01-02 15:04"), r.Status)
			s.history += "\n" + r.Provenance.Kind + " · " + r.Provenance.Note
			if len(r.Provenance.MessageIDs) > 0 {
				s.history += "\n" + L("Source messages: %@", strings.Join(r.Provenance.MessageIDs, ", "))
			}
			if r.Content != nil {
				s.history += "\n\n" + r.Content.Instructions
			} else {
				s.history += "\n\n" + L("Removed")
			}
		}
	}
}

func (s *playbookEditor) value() model.PlaybookContent {
	content := s.content.Clone()
	for kind, target := range map[string]*[]model.PlaybookResource{"references": &content.References, "scripts": &content.Scripts} {
		*target = nil
		for _, file := range s.files[kind] {
			*target = append(*target, model.PlaybookResource{Path: kind + "/" + file.name, Text: file.text})
		}
	}
	return content
}

func (s *playbookEditor) save(sh *sheet) {
	content := s.value()
	if strings.TrimSpace(content.Name) == "" || strings.TrimSpace(content.Description) == "" || strings.TrimSpace(content.Instructions) == "" {
		s.problem = L("Skill name, description, and instructions are required")
		return
	}
	s.busy, s.problem = true, ""
	store.SavePlaybook(s.scope, content, s.previous, func(_ model.PlaybookRecord, err error) {
		if s.closed {
			return
		}
		s.busy = false
		if err == nil {
			if s.onSaved != nil {
				s.onSaved()
			}
			sh.dismiss()
			return
		}
		s.problem = model.ErrorText(err)
		if strings.Contains(err.Error(), "changed since") {
			s.resolveConflict()
		}
	})
}

func (s *playbookEditor) resolveConflict() {
	s.w.showAlert(alertOptions{Message: L("This skill changed while you were editing"),
		Informative: L("Keep your draft to copy it, or reload the current revision before editing again."),
		Buttons:     []alertButton{{Title: L("Keep Draft")}, {Title: L("Reload")}},
	}, func(index int) {
		if index != 1 || s.closed || s.previous == nil {
			return
		}
		s.busy = true
		store.Playbook(s.scope, s.previous.ID, func(fresh model.PlaybookRecord, err error) {
			if s.closed {
				return
			}
			s.busy = false
			if err != nil {
				s.problem = model.ErrorText(err)
				return
			}
			if fresh.Content == nil {
				s.problem = L("This skill was removed")
				return
			}
			s.apply(&fresh)
			s.problem = ""
		})
	})
}

func (s *playbookEditor) resourceView(c *ui.Context, kind string) {
	p := colors(c)
	files := s.files[kind]
	options := make([]popUpOption, 0, len(files))
	for _, file := range files {
		options = append(options, popUpOption{Value: strconv.Itoa(file.key), Label: kind + "/" + file.name})
	}
	ui.Row(c).Gap(8).Children(func() {
		if picked, changed, _ := popUpButton(c.Key(kind+"-picker"), popUp{Options: options, Value: strconv.Itoa(s.selected[kind]), Label: L("Bundled file"), Disabled: s.busy}); changed {
			s.selected[kind], _ = strconv.Atoi(picked)
		}
		if pushButton(c, L("Add File"), pushOptions{Disabled: s.busy || len(files) >= 16}).Clicked() {
			s.nextFile++
			ext, stem := "md", "reference"
			if kind == "scripts" {
				ext, stem = "sh", "script"
			}
			file := &playbookFile{key: s.nextFile, name: fmt.Sprintf("%s-%d.%s", stem, s.nextFile, ext)}
			s.files[kind] = append(files, file)
			s.selected[kind] = file.key
		}
		if pushButton(c, L("Remove File"), pushOptions{Disabled: s.busy || len(files) == 0}).Clicked() {
			s.files[kind] = slices.DeleteFunc(files, func(file *playbookFile) bool { return file.key == s.selected[kind] })
			s.selected[kind] = 0
			if len(s.files[kind]) > 0 {
				s.selected[kind] = s.files[kind][0].key
			}
		}
	})
	for _, file := range s.files[kind] {
		if file.key != s.selected[kind] {
			continue
		}
		ui.Column(c.Key(file.key)).Gap(8).Children(func() {
			textField(c.Key("file-name"), &file.name, fieldOptions{Label: L("Relative file name (for example, checklist.md)"), Mono: true, Disabled: s.busy})
			textArea(c.Key("file-text"), &file.text, 0, fieldOptions{Label: L("Bundled file text"), Mono: true, Disabled: s.busy}).Height(210)
		})
	}
	note := L("Bundle reusable reference text with this skill.")
	if kind == "scripts" {
		note = L("Scripts are saved as text. The bot's usual permissions apply when it runs them.")
	}
	ui.Text(c, note).FontSize(11.5).TextColor(p.Label2).LineHeight(1.4)
}

func (s *playbookEditor) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	title, status := L("Edit Skill"), L("New skill · instructions are required")
	if s.previous != nil {
		status = L("Revision %d", s.previous.Revision)
		if s.previous.Status == "draft" {
			title, status = L("Review Skill Draft"), L("Draft · unavailable to the bot until you save")
		}
	}
	result := sheetFrame(c, sheetOptions{Title: title, Subtitle: L("Scope: %@. Save makes this skill available on later turns. Instructions and scripts keep the bot's usual action permissions.", playbookScopeLabel(s.scope)),
		Width: 680, Confirm: L("Save Skill"), ConfirmDisabled: s.busy, ReturnInContent: true}, func() {
		ui.Column(c.Key("name")).Gap(4).Children(func() {
			ui.Text(c, L("Name")).FontSize(12)
			textField(c, &s.content.Name, fieldOptions{Label: L("Skill name (for example, weekly-review)"), Disabled: s.busy, AutoFocus: s.previous == nil})
		})
		ui.Column(c.Key("description")).Gap(4).Children(func() {
			ui.Text(c, L("Description")).FontSize(12)
			textField(c, &s.content.Description, fieldOptions{Label: L("Describe when to use this skill"), Disabled: s.busy})
		})
		labels := []string{L("Instructions"), L("Examples"), L("References"), L("Scripts")}
		if s.previous != nil {
			labels = append(labels, L("History"))
		}
		tab := s.tab
		segmented(c.Key("tabs"), &tab, L("Skill sections"), labels...).Disabled(s.busy).OnChange(func() { s.tab = tab })
		ui.Column(c.Key(s.tab)).Gap(8).Children(func() {
			switch s.tab {
			case 0:
				textArea(c.Key("instructions"), &s.content.Instructions, 0, fieldOptions{Label: L("Skill instructions"), Mono: true, Disabled: s.busy}).Height(280)
			case 1:
				textArea(c.Key("examples"), &s.content.Examples, 0, fieldOptions{Label: L("Skill examples"), Mono: true, Disabled: s.busy}).Height(280)
			case 2:
				s.resourceView(c, "references")
			case 3:
				s.resourceView(c, "scripts")
			case 4:
				textArea(c.Key("history"), &s.history, 0, fieldOptions{Label: L("Revision history"), Mono: true, ReadOnly: true}).Height(280)
			}
		})
		ui.Text(c, status).FontSize(11.5).TextColor(p.Label2)
		if s.problem != "" {
			ui.Text(c, s.problem).FontSize(12).TextColor(p.Red).LineHeight(1.4)
		}
	})
	if result.Confirmed && !s.busy {
		s.save(sh)
	}
	if result.Cancelled {
		sh.dismiss()
	}
}

type playbookCapture struct {
	w                   *appWindow
	chatID, botID, kind string
	scopes              []model.PlaybookScope
	scope               model.PlaybookScope
	sources             []*model.Message
	picked              map[string]bool
	busy, closed        bool
	problem             string
}

func (w *appWindow) presentPlaybookCapture(chatID string, message *model.Message) *playbookCapture {
	chat := store.Chat(chatID)
	botID, kind, sources, picked := model.CaptureSources(chat, message)
	if botID == "" {
		return nil
	}
	scope := model.PlaybookScope{Kind: "bot", ID: botID}
	s := &playbookCapture{w: w, chatID: chatID, botID: botID, kind: kind, sources: sources, picked: picked, scope: scope, scopes: []model.PlaybookScope{scope}}
	if chat.IsGroup() {
		s.scopes = append(s.scopes, model.PlaybookScope{Kind: "project", ID: chat.ID})
	}
	w.present(s.view, func() { s.closed = true })
	return s
}

func (s *playbookCapture) draft(sh *sheet) {
	var ids []string
	for _, source := range s.sources {
		if s.picked[source.ID] {
			ids = append(ids, source.ID)
		}
	}
	if len(ids) == 0 || len(ids) > 20 || (s.kind == "corrections" && len(ids) < 2) {
		s.problem = L("Select 1–20 source messages")
		if s.kind == "corrections" {
			s.problem = L("Select 2–20 related user corrections")
		}
		return
	}
	s.busy, s.problem = true, ""
	store.DraftPlaybook(s.scope, s.botID, s.chatID, s.kind, ids, func(record model.PlaybookRecord, err error) {
		if s.closed {
			return
		}
		s.busy = false
		if err != nil {
			s.problem = model.ErrorText(err)
			return
		}
		s.w.presentPlaybookEditor(record.Scope, &record, func() { sh.dismiss() })
	})
}

func (s *playbookCapture) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	title, subtitle := L("Save Workflow as Skill"), L("Select the request and successful replies to capture. Review and edit the draft before saving.")
	if s.kind == "corrections" {
		title, subtitle = L("Propose Standing Instructions"), L("Select at least two related user corrections. Review the proposed standing instruction before saving; action permissions stay the same.")
	}
	result := sheetFrame(c, sheetOptions{Title: title, Subtitle: subtitle, Width: 700, Confirm: L("Draft for Review"), ConfirmDisabled: s.busy}, func() {
		if scope, changed := playbookScopePicker(c, s.scopes, s.scope, s.busy); changed {
			s.scope = scope
		}
		ui.Scroll(c.Key("source-list")).Height(250).Children(func() {
			for _, source := range s.sources {
				on := s.picked[source.ID]
				who := L("You")
				if source.Author.Kind == model.AuthorBot {
					who = L("Bot")
					if bot := store.Bot(source.Author.BotID); bot != nil {
						who = bot.Name
					}
				}
				label := who + ": " + strings.Join(strings.Fields(source.Body.Text), " ")
				id := source.ID
				ui.Checkbox(c.Key(id), &on, label).Margin(0, 0, 8, 0).Label(label).Disabled(s.busy).Tooltip(source.Body.Text).OnChange(func() { s.picked[id] = on })
			}
		})
		if s.busy {
			ui.Text(c, L("Drafting…")).FontSize(12).TextColor(p.Label2)
		}
		if s.problem != "" {
			ui.Text(c, s.problem).FontSize(12).TextColor(p.Red)
		}
	})
	if result.Confirmed && !s.busy {
		s.draft(sh)
	}
	if result.Cancelled {
		sh.dismiss()
	}
}
