package main

import (
	"errors"
	"fmt"
	"os"
	"slices"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// A bot's or a group's skills, after the macOS app's skills section in InspectorViewController,
// PlaybookViewController, and PlaybookCaptureViewController: the list in the inspector, a sheet per
// skill, and Save as Skill on a message.

// inspectorSkills are the skills of the bot in a DM, or of the group, hidden while there are none:
// past five rows the rest wait behind Show More. A row opens its skill, and its menu exports or
// deletes it; + adds one.
func (m *mainWindow) inspectorSkills(c *ui.Context, chat *model.Chat, members []*model.Bot) {
	scope, ok := model.SkillScope(chat, members)
	if !ok {
		return
	}
	skills := store.Skills(scope)
	if len(skills) == 0 {
		return
	}
	s := &m.inspector
	shown := skills
	if s.skillsShowingAll != scope && len(skills) > 5 {
		shown = skills[:4]
	}
	section(c.Key("skills"), L("Skills"), sectionCaption, func() {
		if hoverButton(c, hoverButtonOptions{Symbol: "plus", Size: 13, Tooltip: L("New Skill")}).Clicked() {
			m.presentPlaybook(scope, nil)
		}
	}, func(k *card) {
		for _, skill := range shown {
			var row statusRowResult
			ui.Box(c.Key(skill.ID)).Children(func() {
				state := ""
				if skill.IsDraft() {
					state = L("Draft")
				}
				var e ui.Element
				e, row = statusRow(c, k, statusRowOptions{Symbol: "book.closed", Title: skill.Name, Subtitle: skill.Description, SubtitleLines: 2, State: state, Clickable: true})
				e.ContextMenu(func(menu *ui.Menu) {
					if menu.Item(L("Open")).Chosen() {
						m.openPlaybook(skill)
					}
					if menu.Item(L("Export…")).Disabled(skill.IsDraft()).Chosen() {
						m.exportPlaybook(skill)
					}
					menu.Separator()
					if menu.Item(L("Delete…")).Chosen() {
						m.deletePlaybook(skill)
					}
				})
			})
			if row.Clicked {
				m.openPlaybook(skill)
			}
		}
		if len(shown) < len(skills) {
			var more statusRowResult
			ui.Box(c.Key("skills/more")).Children(func() {
				_, more = statusRow(c, k, statusRowOptions{Symbol: "ellipsis.circle", Title: L("Show %d More", len(skills)-len(shown)), Clickable: true})
			})
			if more.Clicked {
				s.skillsShowingAll = scope
			}
		}
	})
}

// newSkill is Chat › New Skill…: one for the bot in the DM on screen, or for the group.
func (m *mainWindow) newSkill() {
	chat := store.Chat(m.selection.ChatID)
	if scope, ok := model.SkillScope(chat, store.BotsIn(chat)); ok {
		m.presentPlaybook(scope, nil)
	}
}

// openPlaybook fetches the skill's body and history, then opens it.
func (w *appWindow) openPlaybook(skill model.PlaybookSummary) {
	store.Playbook(skill.Scope, skill.ID, func(record model.PlaybookRecord, err error) {
		if err != nil {
			w.showAlert(alertOptions{Message: L("Couldn't open %@", skill.Name), Informative: model.ErrorText(err)}, nil)
			return
		}
		w.presentPlaybook(record.Scope, &record)
	})
}

// exportPlaybook writes the skill's portable file where the user picks.
func (w *appWindow) exportPlaybook(skill model.PlaybookSummary) {
	fail := func(err error) {
		w.showAlert(alertOptions{Message: L("Couldn't export the skill"), Informative: model.ErrorText(err)}, nil)
	}
	store.ExportPlaybook(skill.Scope, skill.ID, func(data []byte, err error) {
		if err != nil {
			fail(err)
			return
		}
		parent := w.win
		go func() {
			path, err := mygo.Dialog.Save(mygo.SaveDialogOptions{Parent: parent, Title: L("Export…"), DefaultPath: skill.Name + ".json",
				Filters: []mygo.FileFilter{{Name: "JSON", Extensions: []string{"json"}}}})
			if err == nil && path != "" {
				err = os.WriteFile(path, data, 0o600)
			}
			if err != nil {
				post(func() { fail(err) })
			}
		}()
	})
}

// deletePlaybook asks, then deletes the version the row shows: a skill edited elsewhere since is
// not deleted.
func (w *appWindow) deletePlaybook(skill model.PlaybookSummary) {
	fail := func(err error) {
		w.showAlert(alertOptions{Message: L("Couldn't delete the skill"), Informative: model.ErrorText(err)}, nil)
	}
	w.confirmDeletePlaybook(skill.Name, skill.IsDraft(), func() {
		store.Playbook(skill.Scope, skill.ID, func(record model.PlaybookRecord, err error) {
			if err == nil && record.Revision != skill.Revision {
				err = errors.New(L("This skill changed on another Device. Open it to see what changed."))
			}
			if err != nil {
				fail(err)
				return
			}
			store.RemovePlaybook(record, func(err error) {
				if err != nil {
					fail(err)
				}
			})
		})
	})
}

func (w *appWindow) confirmDeletePlaybook(name string, draft bool, confirmed func()) {
	informative := L("Your bots stop using this skill. This can't be undone.")
	if draft {
		informative = L("The draft is deleted.")
	}
	w.showAlert(alertOptions{
		Message: L("Delete “%@”?", name), Informative: informative, Style: alertWarning,
		Buttons: []alertButton{{Title: L("Delete")}, {Title: L("Cancel")}},
	}, func(index int) {
		if index == 0 {
			confirmed()
		}
	})
}

// MARK: - A skill

// playbookFile is a reference or a script being edited: its name in its folder, the last name that
// was a good one, and its text. Its key stays through renames.
type playbookFile struct {
	key            int
	name, goodName string
	text           string
}

// playbookSheet is a skill of a bot's or a group's: its name and when to use it, the instructions,
// an example, the reference and script files it carries, and how it changed. A draft opens here
// too, and is used only once it is saved. A save carries the revision it was opened at, so it is
// refused when the skill changed on another Device meanwhile.
type playbookSheet struct {
	w      *appWindow
	scope  model.PlaybookScope
	record *model.PlaybookRecord

	name, description, instructions, examples string
	files                                     map[string][]*playbookFile
	selected                                  map[string]int
	nextKey                                   int
	tab, step                                 int
	// nameTaken is the CLI refusing the name: another skill here has it.
	nameTaken    string
	busy, closed bool
}

func (w *appWindow) presentPlaybook(scope model.PlaybookScope, record *model.PlaybookRecord) *playbookSheet {
	s := &playbookSheet{w: w, scope: scope}
	s.fill(record)
	w.present(s.view, func() { s.closed = true })
	return s
}

func (s *playbookSheet) fill(record *model.PlaybookRecord) {
	s.record = record
	content := model.PlaybookContent{}
	if record != nil && record.Content != nil {
		content = *record.Content
	}
	s.name, s.description, s.instructions, s.examples = content.Name, content.Description, content.Instructions, content.Examples
	s.files, s.selected, s.step, s.nameTaken = map[string][]*playbookFile{}, map[string]int{}, 0, ""
	for folder, files := range map[string][]model.PlaybookFile{"references": content.References, "scripts": content.Scripts} {
		for _, file := range files {
			s.nextKey++
			name := strings.TrimPrefix(file.Path, folder+"/")
			s.files[folder] = append(s.files[folder], &playbookFile{key: s.nextKey, name: name, goodName: name, text: file.Text})
		}
		if len(s.files[folder]) > 0 {
			s.selected[folder] = s.files[folder][0].key
		}
	}
}

// subtitle is who uses the skill, and for a draft, that it waits for a save.
func (s *playbookSheet) subtitle() string {
	draft := s.record != nil && s.record.IsDraft()
	if s.scope.IsGroup() {
		group := L("Group")
		if chat := store.Chat(s.scope.ID); chat != nil {
			group = store.Title(chat)
		}
		if draft {
			return L("A draft. Once you save it, the bots in %@ can use it there.", group)
		}
		return L("The bots in %@ can use it there.", group)
	}
	bot := L("The bot")
	if each := store.Bot(s.scope.ID); each != nil {
		bot = each.Name
	}
	if draft {
		return L("A draft. Once you save it, %@ can use it in every chat.", bot)
	}
	return L("%@ can use it in every chat.", bot)
}

// slug is the name as the CLI takes it: lowercase, hyphens for spaces.
func (s *playbookSheet) slug() string {
	name := strings.ToLower(strings.TrimSpace(s.name))
	return strings.NewReplacer(" ", "-", "_", "-").Replace(name)
}

func (s *playbookSheet) nameProblem() string {
	name := s.slug()
	if name == "" {
		return ""
	}
	ok := len(name) <= 64 && !strings.HasPrefix(name, "-") && !strings.HasSuffix(name, "-") && !strings.Contains(name, "--")
	for _, r := range name {
		ok = ok && (r >= 'a' && r <= 'z' || r >= '0' && r <= '9' || r == '-')
	}
	if !ok {
		return L("Use lowercase letters, numbers, and hyphens.")
	}
	if name == s.nameTaken {
		return L("Another skill here has this name.")
	}
	return ""
}

func (s *playbookSheet) canSave() bool {
	return !s.busy && s.slug() != "" && s.nameProblem() == "" && strings.TrimSpace(s.description) != "" && strings.TrimSpace(s.instructions) != ""
}

func (s *playbookSheet) content() model.PlaybookContent {
	content := model.PlaybookContent{Name: s.slug(), Description: strings.TrimSpace(s.description), Instructions: s.instructions, Examples: s.examples,
		References: []model.PlaybookFile{}, Scripts: []model.PlaybookFile{}}
	for folder, target := range map[string]*[]model.PlaybookFile{"references": &content.References, "scripts": &content.Scripts} {
		for _, file := range s.files[folder] {
			*target = append(*target, model.PlaybookFile{Path: folder + "/" + file.goodName, Text: file.text})
		}
	}
	return content
}

func (s *playbookSheet) save(sh *sheet, over *model.PlaybookRecord) {
	s.busy = true
	store.SavePlaybook(s.scope, s.content(), over, func(_ model.PlaybookRecord, err error) {
		if s.closed {
			return
		}
		s.busy = false
		switch {
		case err == nil:
			sh.dismiss()
		case errors.Is(err, model.ErrPlaybookChanged):
			s.resolveConflict(sh)
		case strings.Contains(err.Error(), "already exists"):
			s.nameTaken = s.slug()
		case strings.Contains(err.Error(), "was removed"):
			s.w.showAlert(alertOptions{Message: L("This skill was deleted on another Device.")}, nil)
		default:
			s.w.showAlert(alertOptions{Message: L("Couldn't save the skill"), Informative: model.ErrorText(err)}, nil)
		}
	})
}

// resolveConflict is the skill changed on another Device while the sheet was open: show that
// version, or save these edits over it.
func (s *playbookSheet) resolveConflict(sh *sheet) {
	if s.record == nil {
		return
	}
	id := s.record.ID
	s.w.showAlert(alertOptions{
		Message:     L("This skill changed on another Device"),
		Informative: L("Reload shows the latest version and discards your edits. Overwrite saves yours over it."),
		Buttons:     []alertButton{{Title: L("Reload")}, {Title: L("Overwrite with mine")}, {Title: L("Cancel")}},
	}, func(index int) {
		if index == 2 || s.closed {
			return
		}
		store.Playbook(s.scope, id, func(fresh model.PlaybookRecord, err error) {
			if s.closed {
				return
			}
			switch {
			case err != nil:
				s.w.showAlert(alertOptions{Message: L("This skill was deleted on another Device.")}, nil)
			case index == 0:
				s.fill(&fresh)
			default:
				s.save(sh, &fresh)
			}
		})
	})
}

func (s *playbookSheet) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	title := L("New Skill")
	var leading func()
	if s.record != nil {
		title = s.record.Content.Name
		leading = func() {
			if pushButton(c, L("Delete…"), pushOptions{Kind: buttonDestructive, Disabled: s.busy}).Clicked() {
				record := *s.record
				s.w.confirmDeletePlaybook(record.Content.Name, record.IsDraft(), func() {
					store.RemovePlaybook(record, func(err error) {
						if err != nil {
							s.w.showAlert(alertOptions{Message: L("Couldn't delete the skill"), Informative: model.ErrorText(err)}, nil)
							return
						}
						sh.dismiss()
					})
				})
			}
		}
	}
	result := sheetFrame(c, sheetOptions{Title: title, Subtitle: s.subtitle(), Width: 600, Confirm: L("Save"), ConfirmDisabled: !s.canSave(),
		Leading: leading, ReturnInContent: true}, func() {
		labelWidth := mcpFormLabelWidth(c, L("Name"), L("Description"))
		ui.Column(c).Gap(8).Children(func() {
			providerFormRow(c, labelWidth, L("Name"), func() {
				textField(c.Key("name"), &s.name, fieldOptions{Placeholder: "weekly-report", Mono: true, Disabled: s.busy, AutoFocus: s.record == nil, Label: L("Name")})
			})
			if problem := s.nameProblem(); problem != "" {
				providerFormNote(c, labelWidth, problem, &p.Red)
			}
			providerFormRow(c, labelWidth, L("Description"), func() {
				textField(c.Key("description"), &s.description, fieldOptions{Placeholder: L("When a bot should use it"), Disabled: s.busy, Label: L("Description")})
			})
		})
		labels := []string{L("Instructions"), L("Examples"), L("References"), L("Scripts")}
		if s.record != nil {
			labels = append(labels, L("History"))
		}
		ui.Row(c).Justify(ui.Center).Margin(4, 0, 0, 0).Children(func() {
			segmented(c.Key("tabs"), &s.tab, L("Skills"), labels...)
		})
		ui.Column(c.Key(s.tab)).Height(300).Children(func() {
			switch s.tab {
			case 0:
				textArea(c.Key("instructions"), &s.instructions, 0, fieldOptions{Placeholder: L("What to do, step by step"), Disabled: s.busy, Label: L("Instructions")}).Grow(1)
			case 1:
				textArea(c.Key("examples"), &s.examples, 0, fieldOptions{Placeholder: L("A good result to aim for (optional)"), Disabled: s.busy, Label: L("Examples")}).Grow(1)
			case 2:
				s.filesView(c, "references")
			case 3:
				s.filesView(c, "scripts")
			case 4:
				s.historyView(c)
			}
		})
	})
	if result.Confirmed && s.canSave() {
		s.save(sh, s.record)
	}
	if result.Cancelled {
		sh.dismiss()
	}
}

// listBox is the bordered list beside a pane's text: the files, or the history.
func listBox(c *ui.Context, width float32) ui.Element {
	p := colors(c)
	return ui.Scroll(c).Width(width).Grow(1).MinHeight(0).Radius(6).Background(p.Field).Border(1, p.FieldBorder)
}

// listRow is a row of a listBox, filled in the selection's color while selected.
func listRow(c *ui.Context, selected bool, label string) ui.Element {
	p := colors(c)
	row := ui.Row(c).Padding(4, 8).MinHeight(24).Label(label).Cursor(ui.CursorPointer)
	if selected {
		row.Background(p.Selection).TextColor(p.SelectionText)
	}
	return row
}

// filesView is the references or the scripts a skill carries: a file list with + and −, the
// selected file's name edited in place, and its text. Files are text the skill holds; nothing here
// reads or writes a file on disk, and a script runs only when a bot runs it, with the usual
// Auto-review.
func (s *playbookSheet) filesView(c *ui.Context, folder string) {
	p := colors(c)
	files := s.files[folder]
	var shown *playbookFile
	for _, file := range files {
		if file.key == s.selected[folder] {
			shown = file
		}
	}
	ui.Row(c).Gap(8).Grow(1).MinHeight(0).AlignItems(ui.Stretch).Children(func() {
		ui.Column(c).Width(168).Gap(0).MinHeight(0).Children(func() {
			listBox(c, 168).Children(func() {
				for _, file := range files {
					selected := file == shown
					row := listRow(c.Key(file.key), selected, file.name)
					row.Children(func() {
						if !selected {
							ui.Text(c, file.name).Font(monoFont).FontSize(12).SingleLine()
							return
						}
						field := textField(c.Key("file-name"), &file.name, fieldOptions{Mono: true, Plain: true, Label: L("Name")}).Padding(0).MinHeight(16).TextColor(p.SelectionText).Grow(1)
						// A rename keeps the folder; a name the CLI would refuse goes back to what
						// it was.
						if !field.Focused() && file.name != file.goodName {
							if safeFileName(file.name) && !slices.ContainsFunc(files, func(other *playbookFile) bool { return other != file && other.goodName == file.name }) {
								file.goodName = file.name
							} else {
								file.name = file.goodName
								beep()
							}
						}
					})
					if !selected && row.Clicked() {
						s.selected[folder] = file.key
					}
				}
			})
			ui.Row(c).Gap(2).Margin(4, 0, 0, 0).Children(func() {
				add := L("Add Reference")
				if folder == "scripts" {
					add = L("Add Script")
				}
				if hoverButton(c, hoverButtonOptions{Symbol: "plus", Size: 13, Tooltip: add, Disabled: s.busy || len(files) >= 16}).Clicked() {
					stem, ext := "notes", "md"
					if folder == "scripts" {
						stem, ext = "script", "sh"
					}
					name := stem + "." + ext
					for n := 2; slices.ContainsFunc(files, func(f *playbookFile) bool { return f.goodName == name }); n++ {
						name = fmt.Sprintf("%s-%d.%s", stem, n, ext)
					}
					s.nextKey++
					s.files[folder] = append(slices.Clone(files), &playbookFile{key: s.nextKey, name: name, goodName: name})
					s.selected[folder] = s.nextKey
				}
				if hoverButton(c, hoverButtonOptions{Symbol: "minus", Size: 13, Tooltip: L("Remove"), Disabled: s.busy || shown == nil}).Clicked() {
					index := slices.Index(files, shown)
					s.files[folder] = slices.Delete(slices.Clone(files), index, index+1)
					s.selected[folder] = 0
					if rest := s.files[folder]; len(rest) > 0 {
						s.selected[folder] = rest[min(index, len(rest)-1)].key
					}
				}
			})
		})
		if shown == nil {
			empty := L("No references")
			if folder == "scripts" {
				empty = L("No scripts")
			}
			ui.Column(c).Grow(1).Justify(ui.Center).AlignItems(ui.Center).Children(func() {
				ui.Text(c, empty).FontSize(12).TextColor(p.Label3)
			})
			return
		}
		textArea(c.Key(fmt.Sprint("file-", shown.key)), &shown.text, 0, fieldOptions{Mono: true, Disabled: s.busy, Label: shown.name}).Grow(1)
	})
}

// safeFileName is a name the CLI takes for a bundled file: letters, numbers, dots, hyphens, and
// underscores, in folders, none of them hidden.
func safeFileName(name string) bool {
	if name == "" {
		return false
	}
	for _, part := range strings.Split(name, "/") {
		if part == "" || strings.HasPrefix(part, ".") {
			return false
		}
		for _, r := range part {
			if !(r >= 'a' && r <= 'z' || r >= 'A' && r <= 'Z' || r >= '0' && r <= '9' || r == '-' || r == '_' || r == '.') {
				return false
			}
		}
	}
	return true
}

// historyView is how the skill changed, newest first; picking a step shows its instructions then.
func (s *playbookSheet) historyView(c *ui.Context) {
	p := colors(c)
	steps := slices.Clone(s.record.Revisions)
	slices.SortStableFunc(steps, func(a, b model.PlaybookRevision) int {
		if a.Revision != b.Revision {
			return int(b.Revision) - int(a.Revision)
		}
		if a.CreatedAt > b.CreatedAt {
			return -1
		}
		return 1
	})
	what := func(step model.PlaybookRevision) string {
		switch {
		case step.Status == "deleted":
			return L("Deleted")
		case step.Status == "draft" && step.Provenance.Kind == "corrections":
			return L("Drafted from corrections")
		case step.Status == "draft" && step.Provenance.Kind == "recording":
			return L("Drafted from a recording")
		case step.Status == "draft":
			return L("Drafted from a chat")
		case step.Provenance.Kind == "edit" && step.Revision > 1:
			for _, before := range steps {
				if before.Revision == step.Revision-1 && before.Status == "draft" {
					return L("Saved")
				}
			}
			return L("Edited")
		}
		return L("Created")
	}
	ui.Row(c).Gap(8).Grow(1).MinHeight(0).AlignItems(ui.Stretch).Children(func() {
		listBox(c, 196).Children(func() {
			for i, step := range steps {
				selected := i == s.step
				when := model.DaySeparator(time.Unix(int64(step.CreatedAt), 0))
				if device := store.Device(step.DeviceID); device != nil {
					when += " · " + device.Name
				}
				row := listRow(c.Key(step.ID), selected, what(step)).MinHeight(38)
				row.Children(func() {
					ui.Column(c).Gap(1).MinWidth(0).Children(func() {
						ui.Text(c, what(step)).FontSize(12.5).FontWeight(500).SingleLine()
						detail := ui.Text(c, when).FontSize(textCaption).SingleLine()
						if !selected {
							detail.TextColor(p.Label2)
						}
					})
				})
				if row.Clicked() {
					s.step = i
				}
			}
		})
		if s.step >= len(steps) || steps[s.step].Content == nil {
			ui.Column(c).Grow(1).Justify(ui.Center).AlignItems(ui.Center).Children(func() {
				ui.Text(c, L("Deleted")).FontSize(12).TextColor(p.Label3)
			})
			return
		}
		text := steps[s.step].Content.Instructions
		textArea(c.Key("step-"+steps[s.step].ID), &text, 0, fieldOptions{ReadOnly: true, Label: L("Instructions")}).Grow(1)
	})
}

// MARK: - Save as Skill

// playbookCapture is Save as Skill on a bot's reply, or Save as Standing Instruction on the user's
// own message: the messages around it to pick from, and in a group whether the skill is the
// group's or the bot's. Only the picked messages go to the bot's provider, which writes a draft;
// the draft opens for review and is used once it is saved.
type playbookCapture struct {
	w                   *appWindow
	chatID, botID, kind string
	sources             []*model.Message
	picked              map[string]bool
	// forBot is the bot's skill rather than the group's, in a group.
	forBot       bool
	busy, closed bool
}

func (w *appWindow) presentPlaybookCapture(chatID string, message *model.Message) *playbookCapture {
	botID, kind, sources, picked := model.CaptureSources(store.Chat(chatID), message)
	if botID == "" {
		return nil
	}
	s := &playbookCapture{w: w, chatID: chatID, botID: botID, kind: kind, sources: sources, picked: picked}
	w.present(s.view, func() { s.closed = true })
	return s
}

// enough is what a draft needs: a reply from the bot for a workflow, two corrections for a
// standing instruction.
func (s *playbookCapture) enough() bool {
	count, reply := 0, false
	for _, source := range s.sources {
		if s.picked[source.ID] {
			count++
			reply = reply || source.Author.Kind != model.AuthorYou
		}
	}
	if s.kind == "workflow" {
		return reply
	}
	return count >= 2
}

func (s *playbookCapture) draft(sh *sheet) {
	chat := store.Chat(s.chatID)
	scope := model.BotScope(s.botID)
	if chat != nil && chat.IsGroup() && !s.forBot {
		scope = model.GroupScope(s.chatID)
	}
	var ids []string
	for _, source := range s.sources {
		if s.picked[source.ID] {
			ids = append(ids, source.ID)
		}
	}
	s.busy = true
	store.DraftPlaybook(scope, s.botID, s.chatID, s.kind, ids, func(draft model.PlaybookRecord, err error) {
		if s.closed {
			return
		}
		s.busy = false
		if err != nil {
			s.w.showAlert(alertOptions{Message: L("Couldn't write the draft"), Informative: model.ErrorText(err)}, nil)
			return
		}
		sh.dismiss()
		s.w.presentPlaybook(draft.Scope, &draft)
	})
}

// pickerMessageRow is a message to pick for a skill: a check, who wrote it, and its first lines.
func pickerMessageRow(c *ui.Context, k *card, author, text string, selected, enabled bool) bool {
	p := colors(c)
	preview := strings.Join(strings.Fields(text), " ")
	label := preview
	if author != "" {
		label = author + ": " + preview
	}
	r := k.row(rowBox(c).AlignItems(ui.Start).Padding(9, 12).Label(label).Tooltip(text).Role(ui.RoleCheckBox).Checked(selected))
	clicked := false
	if enabled {
		r.Cursor(ui.CursorPointer)
		if r.Hovered() {
			r.Background(p.RowHover)
		}
		clicked = r.Clicked()
	} else {
		r.Disabled(true)
	}
	r.Children(func() {
		check, tint := "circle", p.Label3
		if selected {
			check, tint = "checkmark.circle.fill", p.Accent
		}
		ui.Row(c).TextColor(tint).Children(func() { symbol(c, check, 16, 1.8) })
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(2).Children(func() {
			if author != "" {
				ui.Text(c, author).FontSize(12).FontWeight(600).SingleLine()
			}
			body := ui.Text(c, preview).FontSize(12).MaxLines(2)
			if author != "" {
				body.TextColor(p.Label2)
			}
		})
	})
	return clicked
}

func (s *playbookCapture) view(c *ui.Context, sh *sheet) {
	chat := store.Chat(s.chatID)
	botName := L("The bot")
	if bot := store.Bot(s.botID); bot != nil {
		botName = bot.Name
	}
	title, subtitle, listTitle := L("Save as Skill"), L("Pick the messages that show how it's done. You review the draft next."), L("Messages")
	if s.kind == "corrections" {
		title, subtitle, listTitle = L("Save as Standing Instruction"), L("Pick the corrections it should cover. You review the draft next."), L("Your messages")
	}
	var leading func()
	if s.busy {
		leading = func() {
			spinner(c, 14)
			caption(c, L("Writing a draft…")).FontSize(12)
		}
	}
	result := sheetFrame(c, sheetOptions{Title: title, Subtitle: subtitle, Width: 480, Confirm: L("Continue"), ConfirmDisabled: s.busy || !s.enough(), Leading: leading}, func() {
		if chat != nil && chat.IsGroup() {
			labelWidth := mcpFormLabelWidth(c, L("Used by"))
			providerFormRow(c, labelWidth, L("Used by"), func() {
				value := "group"
				if s.forBot {
					value = "bot"
				}
				options := []popUpOption{{Value: "group", Label: L("The bots in %@", store.Title(chat))}, {Value: "bot", Label: L("%@, in every chat", botName)}}
				ui.Row(c).Children(func() {
					if picked, changed, _ := popUpButton(c.Key("scope"), popUp{Options: options, Value: value, Disabled: s.busy, Label: L("Used by")}); changed {
						s.forBot = picked == "bot"
					}
				})
			})
		}
		ui.Scroll(c.Key("sources")).MaxHeight(320).Margin(0, -3).Padding(0, 3).Children(func() {
			section(c, listTitle, sectionCaption, nil, func(k *card) {
				for _, source := range s.sources {
					author := ""
					if s.kind == "workflow" {
						author = L("You")
						if source.Author.Kind != model.AuthorYou {
							author = botName
						}
					}
					var clicked bool
					ui.Box(c.Key(source.ID)).Children(func() {
						clicked = pickerMessageRow(c, k, author, source.Body.Text, s.picked[source.ID], !s.busy)
					})
					if clicked {
						s.picked[source.ID] = !s.picked[source.ID]
					}
				}
			})
		})
	})
	if result.Confirmed && !s.busy && s.enough() {
		s.draft(sh)
	}
	if result.Cancelled {
		sh.dismiss()
	}
}
