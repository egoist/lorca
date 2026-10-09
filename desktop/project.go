package main

import (
	"fmt"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A group's project context, after the macOS app's Project section and ProjectEntryViewController:
// a row per entry in the inspector, which opens it, and a + menu that adds one. The sheet is the
// entry's title, text, and source link, or its file; Save writes a new version every bot in the
// group reads from its next turn.

// projectInspectorState is each group's context as the CLI last listed it, fetched when the group
// shows and again when it changes; the fetches on their way, and the groups that changed during
// one; and the group whose section shows every entry rather than the first few.
type projectInspectorState struct {
	contexts   map[string]model.ProjectContext
	fetching   map[string]bool
	again      map[string]bool
	showingAll string
}

// refresh asks the CLI for the group's context; the section redraws when it answers. A change while
// a fetch is on its way asks again once it lands.
func (s *projectInspectorState) refresh(chatID string) {
	if s.contexts == nil {
		s.contexts, s.fetching, s.again = map[string]model.ProjectContext{}, map[string]bool{}, map[string]bool{}
	}
	if s.fetching[chatID] {
		s.again[chatID] = true
		return
	}
	s.fetching[chatID] = true
	store.ProjectContext(chatID, func(context model.ProjectContext, err error) {
		delete(s.fetching, chatID)
		if err == nil {
			s.contexts[chatID] = context
		}
		if s.again[chatID] {
			delete(s.again, chatID)
			s.refresh(chatID)
		}
	})
}

// inspectorProject is what every bot in the group can read: an entry a row, which opens it, the
// briefs and decisions first. Past five rows the rest wait behind Show More. The title's + adds one;
// with none yet, the title and its + are all the section shows.
func (m *mainWindow) inspectorProject(c *ui.Context, chat *model.Chat) {
	p := colors(c)
	s := &m.inspector.project
	context := s.contexts[chat.ID]
	entries := context.Entries
	shown := entries
	if s.showingAll != chat.ID && len(entries) > 5 {
		shown = entries[:4]
	}
	chatID := chat.ID
	add := func() {
		menu := hoverButton(c, hoverButtonOptions{Symbol: "plus", Size: 14, Tooltip: L("Add to Project")})
		menu.Menu(func(menu *ui.Menu) {
			for _, kind := range model.ProjectKinds {
				if kind == "document" {
					menu.Separator()
				}
				if menu.Item(model.ProjectKindTitle(kind) + "…").Chosen() {
					m.addProjectEntry(chatID, kind)
				}
			}
		})
	}
	var rows func(k *card)
	if len(entries) > 0 {
		rows = func(k *card) {
			for _, entry := range shown {
				// What needs the user is said after the kind, and marked in orange at the end; a
				// title keeps the row's width.
				problem := ""
				switch {
				case len(context.OtherVersions(entry.ID)) > 0:
					problem = L("Two versions")
				case entry.Freshness == "unavailable":
					problem = L("Unavailable")
				}
				detail := problem
				if detail == "" && entry.IsSuggestion() {
					detail = L("Suggested by %@", entry.Source.Label)
				} else if detail == "" {
					detail = entry.Host()
				}
				subtitle := model.ProjectKindTitle(entry.Kind)
				if detail != "" {
					subtitle += " · " + detail
				}
				o := statusRowOptions{Symbol: model.ProjectKindSymbol(entry.Kind), Title: entry.Title, Subtitle: subtitle, Clickable: true, Tooltip: entry.Title}
				if problem != "" {
					o.State, o.StateSymbol, o.StateColor = problem, "exclamationmark.circle.fill", &p.Orange
				}
				id := entry.ID
				var row statusRowResult
				ui.Box(c.Key("project/" + id)).Children(func() { _, row = statusRow(c, k, o) })
				if row.Clicked {
					m.openProjectEntry(chatID, id)
				}
			}
			if len(shown) < len(entries) {
				var more statusRowResult
				ui.Box(c.Key("project/more")).Children(func() {
					_, more = statusRow(c, k, statusRowOptions{Symbol: "ellipsis.circle", Title: L("Show %d More", len(entries)-len(shown)), Clickable: true})
				})
				if more.Clicked {
					s.showingAll = chatID
				}
			}
		}
	}
	section(c.Key("project"), L("Project"), sectionCaption, add, rows)
}

// openProjectEntry opens the entry as the section last listed it.
func (m *mainWindow) openProjectEntry(chatID, id string) {
	context := m.inspector.project.contexts[chatID]
	for _, entry := range context.Entries {
		if entry.ID == id {
			current := entry
			m.presentProjectEntry(chatID, &current, entry.Kind, context.OtherVersions(id))
			return
		}
	}
}

// addProjectEntry starts a new entry of `kind`; a file is picked in an open panel and added at once.
func (m *mainWindow) addProjectEntry(chatID, kind string) {
	if kind != "asset" {
		m.presentProjectEntry(chatID, nil, kind, nil)
		return
	}
	chooseFiles(m.win, L("Add to Project"), "", L("Add"), false, false, func(files []fileInfo) {
		if len(files) == 0 {
			return
		}
		name := files[0].Name
		store.AddProjectFile(chatID, files[0].Path, func(err error) {
			if err != nil {
				m.showAlert(alertOptions{Message: L("Couldn't add “%@”", name), Informative: model.ErrorText(err)}, nil)
			}
		})
	})
}

// projectEntrySheet is the sheet's state, kept outside the build pass: the entry it opened (nil for
// a new one) and the form, what the form compares the edits against, and the other versions it
// replaces when two Devices changed the entry at once.
type projectEntrySheet struct {
	w                 *appWindow
	chatID, kind      string
	entry             *model.ProjectEntry
	otherVersions     []string
	title, text, link string
	saved             [3]string
	busy, closed      bool
	generation        int
}

// presentProjectEntry opens an entry, or a new one of `kind`.
func (w *appWindow) presentProjectEntry(chatID string, entry *model.ProjectEntry, kind string, otherVersions []string) *projectEntrySheet {
	st := &projectEntrySheet{w: w, chatID: chatID, kind: kind, otherVersions: otherVersions}
	if entry != nil {
		st.kind = entry.Kind
	}
	st.show(entry)
	var shown *sheet
	// A group deleted here or on another Device takes its context along.
	stop := pluginWatchStore(func(event model.Event) {
		if event.Kind == model.EventChatsChanged && store.Chat(chatID) == nil && shown != nil {
			shown.dismiss()
		}
	})
	shown = w.present(st.view, func() { st.closed = true; stop() })
	return st
}

// show fills the form from `entry`, which becomes what the form compares the edits against.
func (st *projectEntrySheet) show(entry *model.ProjectEntry) {
	st.entry = entry
	st.title, st.text, st.link = "", "", ""
	if entry != nil {
		st.title, st.text, st.link = entry.Title, entry.Text, entry.Source.URL
	}
	st.saved = [3]string{st.title, st.text, st.link}
	st.generation++
}

func (st *projectEntrySheet) trimmedLink() string { return strings.TrimSpace(st.link) }

func (st *projectEntrySheet) edited() bool {
	return [3]string{st.title, st.text, st.trimmedLink()} != st.saved
}

// provenance says who wrote the entry, or where it came from.
func projectProvenance(entry *model.ProjectEntry) string {
	day := model.ProjectDay(entry.Updated())
	switch {
	case entry.Source.Kind == "user":
		return L("Saved by you on %@", day)
	case entry.Source.Kind == "bot" && entry.IsSuggestion():
		return L("Suggested by %@ on %@", entry.Source.Label, day)
	case entry.Source.Kind == "bot":
		return L("Added by %@ on %@", entry.Source.Label, day)
	case entry.Source.Kind == "url":
		return L("From %@", firstNonEmpty(entry.Host(), entry.Source.Label))
	}
	return L("From %@ on %@", entry.Source.Label, day)
}

func (st *projectEntrySheet) confirmTitle() string {
	switch {
	case st.entry == nil:
		return L("Add")
	case len(st.otherVersions) > 0:
		return L("Keep This Version")
	case st.entry.IsSuggestion():
		return L("Accept")
	}
	return L("Save")
}

// whatStopsSaving is why the entry can't be saved as it stands, in words for the form.
func (st *projectEntrySheet) whatStopsSaving() string {
	link := st.trimmedLink()
	switch {
	case len(st.text) > 32_000:
		return L("This is too long for one entry. Split it into a few.")
	case len(st.title) > 240:
		return L("The title is too long.")
	case link != "" && (!strings.HasPrefix(link, "https://") || len(link) <= len("https://")):
		return L("Links start with https://.")
	}
	return ""
}

func (st *projectEntrySheet) canSave() bool {
	changes := st.entry == nil || st.edited() || st.entry.IsSuggestion() || len(st.otherVersions) > 0
	needsLink := st.kind == "document" && st.trimmedLink() == ""
	return !st.busy && strings.TrimSpace(st.title) != "" && !needsLink && st.whatStopsSaving() == "" && changes
}

func (st *projectEntrySheet) view(c *ui.Context, s *sheet) {
	p := colors(c)
	title, subtitle := model.ProjectKindNewTitle(st.kind), L("Every bot in this group can read it.")
	if st.entry != nil {
		title, subtitle = model.ProjectKindTitle(st.kind), projectProvenance(st.entry)
	}
	var leading func()
	if st.entry != nil {
		// Removing sits apart from saving, red, at the leading edge.
		leading = func() {
			if pushButton(c, L("Remove…"), pushOptions{Kind: buttonDestructive, Disabled: st.busy}).Clicked() {
				st.confirmRemove(s)
			}
		}
	}
	result := sheetFrame(c, sheetOptions{Title: title, Subtitle: subtitle, Width: 480, Confirm: st.confirmTitle(), ConfirmDisabled: !st.canSave(), Leading: leading, ReturnInContent: true}, func() {
		if len(st.otherVersions) > 0 {
			ui.Text(c, L("Two Devices changed this at the same time. Keep This Version saves this one and replaces the other.")).FontSize(12).LineHeight(1.4).TextColor(p.Orange)
		}
		// Keyed by the version shown, so another version gives the fields its text.
		field := func(name string) *ui.Context {
			return c.Key(fmt.Sprintf("project-entry/%s/%s/%d", st.chatID, name, st.generation))
		}
		textField(field("title"), &st.title, fieldOptions{Placeholder: L("Title"), Label: L("Title"), AutoFocus: st.entry == nil})
		text := func(lines int) {
			textArea(field("text"), &st.text, lines, fieldOptions{Label: title, AutoFocus: st.entry != nil})
		}
		switch st.kind {
		case "asset":
			st.fileRow(c, s)
			text(4)
		case "document":
			st.linkField(c, field("link"))
			text(4)
		default:
			text(7)
			st.linkField(c, field("link"))
		}
		if problem := st.whatStopsSaving(); problem != "" {
			ui.Text(c, problem).FontSize(textCaption).LineHeight(1.4).TextColor(p.Orange)
		}
	})
	switch {
	case result.Confirmed:
		st.save(s, st.entry)
	case result.Cancelled:
		s.dismiss()
	}
}

// linkField is the source link, and under it the link as the CLI last read it, with Check Now.
func (st *projectEntrySheet) linkField(c *ui.Context, key *ui.Context) {
	p := colors(c)
	placeholder := L("Source link (optional)")
	if st.kind == "document" {
		placeholder = "https://…"
	}
	ui.Column(c).Gap(6).Children(func() {
		textField(key, &st.link, fieldOptions{Placeholder: placeholder, Label: L("Link")})
		entry := st.entry
		if entry == nil || entry.Source.URL == "" || entry.Source.URL != st.trimmedLink() {
			return
		}
		ui.Row(c).Gap(8).Children(func() {
			status, tint := L("Not checked yet"), p.Label2
			if checked, ok := entry.Checked(); ok {
				status = L("Checked %@", model.DateTime(checked))
			}
			if entry.Freshness == "unavailable" {
				status, tint = L("Couldn't open this link. Bots use the copy from before."), p.Orange
			}
			line := ui.Text(c, status).Grow(1).Shrink(1).MinWidth(0).FontSize(textCaption).TextColor(tint).SingleLine()
			if entry.Freshness == "unavailable" && entry.RefreshError != "" {
				line.Tooltip(entry.RefreshError)
			}
			if entry.CanCheckLink() && pushButton(c, L("Check Now"), pushOptions{Small: true, Disabled: st.busy || st.edited()}).Clicked() {
				st.checkLink()
			}
		})
	})
}

// fileRow is a file entry's file: its name and size, and Open.
func (st *projectEntrySheet) fileRow(c *ui.Context, s *sheet) {
	p := colors(c)
	entry := st.entry
	if entry == nil || entry.Asset == nil {
		return
	}
	asset := entry.Asset
	ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
		ui.Row(c).Width(32).Justify(ui.Center).TextColor(p.Label2).Children(func() { symbol(c, "doc.fill", 26, 1.5) })
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			ui.Text(c, asset.Name).FontSize(13).FontWeight(500).SingleLine()
			if asset.Size > 0 {
				ui.Text(c, model.Kilobytes(int(asset.Size))).FontSize(textCaption).TextColor(p.Label2)
			}
		})
		if pushButton(c, L("Open"), pushOptions{}).Clicked() {
			st.openFile(asset.Name)
		}
	})
}

func (st *projectEntrySheet) save(s *sheet, base *model.ProjectEntry) {
	st.busy = true
	save := model.ProjectSave{Kind: st.kind, Title: strings.TrimSpace(st.title), Text: st.text, Link: st.trimmedLink(), Replacing: base}
	if base != nil {
		save.AlsoReplacing = st.otherVersions
	}
	store.SaveProjectEntry(st.chatID, save, func(err error) {
		if st.closed {
			return
		}
		st.busy = false
		switch {
		case err == nil:
			s.dismiss()
		case base != nil && err.Error() == model.StaleProjectEntry:
			st.resolveConflict(s, *base)
		default:
			st.w.showAlert(alertOptions{Message: L("Couldn't save this entry"), Informative: model.ErrorText(err)}, nil)
		}
	})
}

// resolveConflict is another Device having changed or removed the entry while it was open here:
// show its version, or save these edits over it; a removed one can be added back.
func (st *projectEntrySheet) resolveConflict(s *sheet, base model.ProjectEntry) {
	store.CurrentProjectEntry(st.chatID, base.ID, func(newer model.ProjectEntry, found bool, _ error) {
		if st.closed {
			return
		}
		o := alertOptions{
			Message:     L("This entry was removed on another Device"),
			Informative: L("Add it back with your edits, or cancel to leave it removed."),
			Buttons:     []alertButton{{Title: L("Add It Back")}, {Title: L("Cancel")}},
		}
		if found {
			o = alertOptions{
				Message:     L("This entry changed on another Device"),
				Informative: L("Reload shows the other version and discards your edits. Overwrite saves yours over it."),
				Buttons:     []alertButton{{Title: L("Reload")}, {Title: L("Overwrite with mine")}, {Title: L("Cancel")}},
			}
		}
		st.w.showAlert(o, func(index int) {
			if st.closed {
				return
			}
			st.otherVersions = nil
			switch {
			case found && index == 0:
				st.show(&newer)
			case found && index == 1:
				st.save(s, &newer)
			case !found && index == 0:
				st.save(s, nil)
			}
		})
	})
}

func (st *projectEntrySheet) checkLink() {
	entry := st.entry
	if entry == nil {
		return
	}
	st.busy = true
	store.CheckProjectLink(st.chatID, entry.ID, func(fresh model.ProjectEntry, err error) {
		if st.closed {
			return
		}
		st.busy = false
		if err != nil {
			st.w.showAlert(alertOptions{Message: L("Couldn't check this link"), Informative: model.ErrorText(err)}, nil)
			return
		}
		if fresh.ID != "" {
			st.show(&fresh)
		}
	})
}

func (st *projectEntrySheet) openFile(name string) {
	entry := st.entry
	if entry == nil {
		return
	}
	store.ProjectFile(st.chatID, entry.ID, func(path string, err error) {
		if st.closed {
			return
		}
		if err != nil || path == "" {
			st.w.showAlert(alertOptions{Message: L("Couldn't open “%@”", name), Informative: L("The file hasn't reached this Device yet. Try again when the Device that added it is online.")}, nil)
			return
		}
		openFile(path)
	})
}

func (st *projectEntrySheet) confirmRemove(s *sheet) {
	entry := st.entry
	if entry == nil {
		return
	}
	removing := *entry
	st.w.showAlert(alertOptions{
		Message:     L("Remove “%@”?", removing.Title),
		Informative: L("Bots in this group stop seeing it."),
		Buttons:     []alertButton{{Title: L("Remove"), Destructive: true}, {Title: L("Cancel")}},
		Style:       alertWarning,
	}, func(index int) {
		if index != 0 || st.closed {
			return
		}
		store.RemoveProjectEntry(st.chatID, removing, func(err error) {
			if st.closed {
				return
			}
			switch {
			case err == nil:
				s.dismiss()
			case err.Error() == model.StaleProjectEntry:
				st.w.showAlert(alertOptions{Message: L("This entry changed on another Device"), Informative: L("Close it and open it again to see the other version.")}, nil)
			default:
				st.w.showAlert(alertOptions{Message: L("Couldn't remove this entry"), Informative: model.ErrorText(err)}, nil)
			}
		})
	})
}
