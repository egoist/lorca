package main

import (
	"fmt"
	"net/url"
	"slices"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

const projectEntryMaxBytes = 32_000

// Draft fields live outside MyGo's build pass. Controls bind directly to these values and use
// explicit group/entry/field keys; a new frame or an unrelated CLI event keeps the user's edits.
type projectDraft struct {
	kind, title, text, sourceLabel, sourceURL, verification string
}

type projectContextSheet struct {
	w                               *appWindow
	chatID                          string
	page                            model.ProjectContext
	selected                        string
	draft, baseline                 projectDraft
	history, loadedHistory          bool
	loading, busy, closed, outdated bool
	loads                           int
	status                          *providerStatus
}

func (w *appWindow) presentProjectContext(chatID string) *projectContextSheet {
	if chat := store.Chat(chatID); chat == nil || !chat.IsGroup() {
		return nil
	}
	st := &projectContextSheet{w: w, chatID: chatID}
	st.selectEntry("")
	stop := pluginWatchStore(func(event model.Event) {
		if st.closed {
			return
		}
		if event.Kind == model.EventProjectContextChanged && event.ChatID == st.chatID {
			st.contextChanged()
		}
	})
	w.present(st.view, func() { st.closed = true; st.loads++; stop() })
	st.load("")
	return st
}

func (s *projectContextSheet) contextChanged() {
	s.outdated = true
	if !s.dirty() && !s.busy && !s.loading {
		s.load(s.selected)
	}
}

func (s *projectContextSheet) entry() *model.ProjectEntry {
	for i := range s.page.Entries {
		if s.page.Entries[i].ID == s.selected {
			return &s.page.Entries[i]
		}
	}
	return nil
}

func (s *projectContextSheet) dirty() bool { return s.draft != s.baseline }
func (s *projectContextSheet) groupExists() bool {
	chat := store.Chat(s.chatID)
	return chat != nil && chat.IsGroup()
}
func (s *projectContextSheet) editable() bool {
	entry := s.entry()
	return s.groupExists() && !s.loading && !s.busy && (entry == nil && s.selected == "" || entry != nil && entry.Current)
}

func (s *projectContextSheet) selectEntry(id string) {
	s.selected = id
	if entry := s.entry(); entry != nil {
		verification := entry.Verification
		if verification != "agreed" && verification != "verified" {
			verification = "unverified"
		}
		s.draft = projectDraft{entry.Kind, entry.Title, entry.Text, entry.Source.Label, entry.Source.URL, verification}
	} else {
		s.selected = ""
		s.draft = projectDraft{kind: "brief", sourceLabel: L("User"), verification: "agreed"}
	}
	s.baseline = s.draft
}

func (s *projectContextSheet) load(selectID string) {
	if s.closed {
		return
	}
	s.loads++
	generation := s.loads
	s.loading, s.outdated = true, false
	s.status = nil
	wantedHistory := s.history
	store.ProjectContext(s.chatID, wantedHistory, func(page model.ProjectContext, err error) {
		if s.closed || generation != s.loads {
			return
		}
		s.loading = false
		if !s.groupExists() {
			return
		}
		if err != nil {
			s.status = &providerStatus{text: model.ErrorText(err), tone: model.ToneRed}
			return
		}
		slices.SortFunc(page.Entries, func(a, b model.ProjectEntry) int {
			if a.UpdatedAt > b.UpdatedAt {
				return -1
			}
			if a.UpdatedAt < b.UpdatedAt {
				return 1
			}
			return strings.Compare(a.ID, b.ID)
		})
		s.page, s.loadedHistory = page, wantedHistory
		s.selectEntry(selectID)
		if s.outdated {
			s.status = &providerStatus{text: L("Project context changed. Reload to review current revisions."), tone: model.ToneOrange}
		}
	})
}

// clean preserves a draft until the user explicitly discards it. The action contains only
// persistent state and copied identifiers; no element or Context survives the build pass.
func (s *projectContextSheet) clean(action func()) {
	if !s.dirty() {
		action()
		return
	}
	s.w.showAlert(alertOptions{
		Message:     L("Discard unsaved project context changes?"),
		Informative: L("Your draft stays until you choose Discard."),
		Buttons:     []alertButton{{Title: L("Keep Editing")}, {Title: L("Discard"), Destructive: true}},
	}, func(index int) {
		if !s.closed && index == 1 {
			action()
		}
	})
}

func (s *projectContextSheet) requestEntry(id string) {
	s.clean(func() { s.status = nil; s.selectEntry(id) })
}

func (s *projectContextSheet) setHistory() {
	wanted := s.history
	s.history = s.loadedHistory
	s.clean(func() { s.history = wanted; s.load(s.selected) })
}

func (s *projectContextSheet) problem() string {
	if !s.editable() || s.outdated {
		return ""
	}
	if strings.TrimSpace(s.draft.title) == "" {
		return L("Add a title for this entry.")
	}
	if len(s.draft.title) > 240 || len(s.draft.text) > projectEntryMaxBytes {
		return L("The title or entry exceeds its byte budget.")
	}
	if s.draft.sourceURL != "" {
		value, err := url.Parse(s.draft.sourceURL)
		if err != nil || value.Scheme != "https" || value.Host == "" || value.User != nil {
			return L("Source URLs use HTTPS and contain no embedded credentials")
		}
	}
	return ""
}

func (s *projectContextSheet) input(removing bool) model.ProjectContextSave {
	source := model.ProjectSource{Kind: "user", Label: s.draft.sourceLabel, URL: s.draft.sourceURL}
	input := model.ProjectContextSave{Kind: s.draft.kind, Title: s.draft.title, Text: s.draft.text, Source: source, Verification: s.draft.verification, Removed: removing}
	if entry := s.entry(); entry != nil {
		input.Supersedes, input.ExpectedRevision, input.MaxAgeSecs = []string{entry.ID}, s.page.Revision, entry.MaxAgeSecs
		// An unchanged URL retains message/output provenance and its immutable version reference.
		input.Source = entry.Source
		input.Source.Label = s.draft.sourceLabel
		if input.Source.URL != s.draft.sourceURL {
			input.Source = source
			if source.URL != "" {
				input.Source.Kind = "url"
			}
		}
	} else if source.URL != "" {
		input.Source.Kind = "url"
	}
	return input
}

func (s *projectContextSheet) saved(entry model.ProjectEntry, err error) {
	if s.closed {
		return
	}
	s.busy = false
	if err != nil {
		s.status = &providerStatus{text: model.ErrorText(err), tone: model.ToneRed}
		if strings.Contains(err.Error(), "changed") {
			s.outdated = true
		}
		return
	}
	if !s.groupExists() {
		return
	}
	if entry.Removed {
		s.load("")
	} else {
		s.load(entry.ID)
	}
}

func (s *projectContextSheet) save(removing bool) {
	if !s.editable() || s.outdated || s.problem() != "" {
		return
	}
	s.busy = true
	store.SaveProjectContext(s.chatID, s.input(removing), s.saved)
}

func (s *projectContextSheet) refresh() {
	entry := s.entry()
	if !s.editable() || entry == nil || entry.Source.URL == "" {
		return
	}
	id := entry.ID
	s.clean(func() {
		s.busy = true
		store.RefreshProjectContext(s.chatID, id, s.saved)
	})
}

func (s *projectContextSheet) attach() {
	if s.loading || s.busy || !s.groupExists() {
		return
	}
	s.clean(func() {
		chooseFiles(s.w.win, L("Add reference file…"), "", L("Add"), false, false, func(files []fileInfo) {
			if s.closed || !s.groupExists() || len(files) == 0 {
				return
			}
			s.addAsset(files[0].Path)
		})
	})
}

func (s *projectContextSheet) addAsset(path string) {
	if s.closed || !s.groupExists() || s.busy || s.loading {
		return
	}
	s.busy = true
	store.AddProjectAsset(s.chatID, path, s.saved)
}

func (s *projectContextSheet) openAsset() {
	entry := s.entry()
	if !s.editable() || entry == nil || entry.Asset == nil {
		return
	}
	s.busy = true
	store.ProjectAssetPath(s.chatID, entry.ID, func(path string, err error) {
		if s.closed {
			return
		}
		s.busy = false
		if !s.groupExists() {
			return
		}
		if err != nil {
			s.status = &providerStatus{text: model.ErrorText(err), tone: model.ToneRed}
			return
		}
		if path != "" {
			openFile(path)
		}
	})
}

func projectFreshness(state string) (string, model.Tone) {
	switch state {
	case "agreed":
		return L("Agreed"), model.ToneSecondary
	case "verified":
		return L("Verified"), model.ToneSecondary
	case "fetched":
		return L("Fetched"), model.ToneSecondary
	case "stale":
		return L("Stale"), model.ToneOrange
	case "unavailable":
		return L("Unavailable"), model.ToneRed
	default:
		return L("Unverified"), model.ToneOrange
	}
}

func projectKinds() []popUpOption {
	return []popUpOption{{Value: "brief", Label: L("Brief")}, {Value: "goal", Label: L("Goal")}, {Value: "constraint", Label: L("Constraint")}, {Value: "decision", Label: L("Decision")}, {Value: "fact", Label: L("Fact")}, {Value: "document", Label: L("Document link")}}
}

func (s *projectContextSheet) view(c *ui.Context, sheet *sheet) {
	c = c.Key("project-context/" + s.chatID)
	p := colors(c)
	editable := s.editable()
	result := sheetFrame(c, sheetOptions{Title: L("Project context"), Subtitle: L("Shared with this group's bots. Corrections keep source and revision history."), Width: 620, Confirm: L("Save"), Cancel: L("Close"), ConfirmDisabled: !editable || s.outdated || s.problem() != "", ReturnInContent: true}, func() {
		if !s.groupExists() {
			ui.Text(c, L("This group is no longer available.")).TextColor(p.Red)
			return
		}
		if s.loading {
			ui.Text(c, L("Loading…")).TextColor(p.Label2)
		}
		labelWidth := float32(100)
		options := []popUpOption{{Value: "", Label: L("New entry")}}
		for _, entry := range s.page.Entries {
			label, _ := projectFreshness(entry.Freshness)
			title := entry.Title + " · " + label
			if !entry.Current {
				title = "↳ " + title
			}
			options = append(options, popUpOption{Value: entry.ID, Label: title})
		}
		providerFormRow(c, labelWidth, L("Entry"), func() {
			if picked, changed, _ := popUpButton(c.Key("entry"), popUp{Options: options, Value: s.selected, Label: L("Project entry"), Width: 450, Disabled: s.loading || s.busy}); changed {
				s.requestEntry(picked)
			}
		})
		kinds := projectKinds()
		if entry := s.entry(); entry != nil && entry.Kind == "asset" {
			kinds = append(kinds, popUpOption{Value: "asset", Label: L("Reference asset")})
		}
		providerFormRow(c, labelWidth, L("Type"), func() {
			if picked, changed, _ := popUpButton(c.Key("kind"), popUp{Options: kinds, Value: s.draft.kind, Label: L("Context type"), Width: 220, Disabled: !editable || s.selected != ""}); changed {
				s.draft.kind = picked
			}
		})
		key := "draft/" + s.selected + "/"
		providerFormRow(c, labelWidth, L("Title"), func() {
			textField(c.Key(key+"title"), &s.draft.title, fieldOptions{Label: L("Entry title"), Disabled: !editable})
		})
		providerFormRow(c, labelWidth, L("Source"), func() {
			textField(c.Key(key+"source"), &s.draft.sourceLabel, fieldOptions{Label: L("Source or decision author"), Disabled: !editable})
		})
		providerFormRow(c, labelWidth, L("Document URL"), func() {
			textField(c.Key(key+"url"), &s.draft.sourceURL, fieldOptions{Label: L("Document URL"), Placeholder: "https://…", Disabled: !editable})
		})
		providerFormRow(c, labelWidth, L("Status"), func() {
			if picked, changed, _ := popUpButton(c.Key("verification"), popUp{Options: []popUpOption{{Value: "agreed", Label: L("Agreed")}, {Value: "verified", Label: L("Verified")}, {Value: "unverified", Label: L("Unverified")}}, Value: s.draft.verification, Label: L("Verification status"), Width: 220, Disabled: !editable}); changed {
				s.draft.verification = picked
			}
		})
		textArea(c.Key(key+"text"), &s.draft.text, 0, fieldOptions{Label: L("Project context text"), ReadOnly: !editable, Disabled: s.busy || s.loading}).Height(190)
		ui.Text(c, L("%d / %d bytes · bots discover up to %d bytes each turn", len(s.draft.text), projectEntryMaxBytes, firstContextBudget(s.page.MaxContextBytes))).FontSize(textCaption).TextColor(p.Label2)
		if entry := s.entry(); entry != nil {
			s.provenance(c, entry)
		}
		if len(s.page.Conflicts) > 0 {
			ui.Text(c, L("Concurrent corrections are visible. Review each current version before deciding which to keep.")).FontSize(textCaption).TextColor(p.Orange).LineHeight(1.4)
		}
		if s.outdated {
			ui.Text(c, L("Project context changed. Reload to review current revisions.")).FontSize(textCaption).TextColor(p.Orange)
		}
		if s.status != nil {
			ui.Text(c, s.status.text).FontSize(textCaption).TextColor(p.tone(s.status.tone)).LineHeight(1.4).Selectable()
		}
		if problem := s.problem(); problem != "" {
			ui.Text(c, problem).FontSize(textCaption).TextColor(p.Label2)
		}
		ui.Row(c).Gap(8).Wrap().Children(func() {
			entry := s.entry()
			if pushButton(c.Key("refresh"), L("Refresh source"), pushOptions{Disabled: !editable || entry == nil || entry.Source.URL == ""}).Clicked() {
				s.refresh()
			}
			if pushButton(c.Key("open"), L("Open asset"), pushOptions{Disabled: !editable || entry == nil || entry.Asset == nil}).Clicked() {
				s.openAsset()
			}
			if pushButton(c.Key("remove"), L("Remove"), pushOptions{Disabled: !editable || entry == nil || s.outdated}).Clicked() {
				s.clean(func() { s.draft = s.baseline; s.save(true) })
			}
			if pushButton(c.Key("reload"), L("Reload"), pushOptions{Disabled: s.loading || s.busy}).Clicked() {
				s.clean(func() { s.load(s.selected) })
			}
			if pushButton(c.Key("attach"), L("Add reference file…"), pushOptions{Disabled: s.loading || s.busy}).Clicked() {
				s.attach()
			}
		})
		ui.Checkbox(c.Key("history"), &s.history, L("Show revision history")).Disabled(s.loading || s.busy).OnChange(s.setHistory)
	})
	if result.Confirmed {
		s.save(false)
	}
	if result.Cancelled {
		s.clean(sheet.dismiss)
	}
}

func firstContextBudget(budget int) int {
	if budget > 0 {
		return budget
	}
	return 8_000
}

func (s *projectContextSheet) provenance(c *ui.Context, entry *model.ProjectEntry) {
	p := colors(c)
	status, tone := projectFreshness(entry.Freshness)
	ui.Column(c).Gap(3).Children(func() {
		ui.Text(c, status+" · "+entry.Source.Label+" · "+model.DateTime(time.Unix(entry.UpdatedAt, 0))).FontSize(textCaption).TextColor(p.tone(tone)).Selectable()
		if entry.VerifiedAt != nil {
			ui.Text(c, L("Verified: %@", model.DateTime(time.Unix(*entry.VerifiedAt, 0)))).FontSize(textCaption).TextColor(p.Label2)
		}
		if entry.FetchedAt != nil {
			ui.Text(c, L("Fetched: %@", model.DateTime(time.Unix(*entry.FetchedAt, 0)))).FontSize(textCaption).TextColor(p.Label2)
		}
		if entry.RefreshError != "" {
			ui.Text(c, entry.RefreshError).FontSize(textCaption).TextColor(p.Red).LineHeight(1.4).Selectable()
		}
		if entry.Source.Output != nil {
			ref := entry.Source.Output
			ui.Text(c, L("Output version: %@", fmt.Sprintf("%s · %d · %s", ref.OutputID, ref.Version, ref.MessageID))).FontSize(textCaption).TextColor(p.Label2).Selectable()
		}
		if entry.Asset != nil {
			ui.Text(c, entry.Asset.Name).FontSize(textCaption).TextColor(p.Label2)
		}
	})
}
