package main

import (
	"encoding/json"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A draft or an exact call a bot left for the user, after the macOS app's ReviewViewController and
// the permission card it would have asked with: who wants to do what and why, the command, call,
// or draft, editable, and one decision. Approve runs what the editor shows on the bot's Runner,
// saving an edit first as a new version; Reject drops it. A change from another Device shows here
// as it lands, unless the user is editing, and then Approve offers the new version instead. Once
// decided, the sheet shows how it went.

// reviewSheet is what one review sheet keeps across build passes: the item as shown, the editor's
// text, and whether a decision is on its way.
type reviewSheet struct {
	item   model.ReviewItem
	text   string
	busy   bool
	closed bool
}

func (w *appWindow) presentReview(item model.ReviewItem) *reviewSheet {
	st := &reviewSheet{}
	st.display(item, false)
	w.present(func(c *ui.Context, s *sheet) { st.view(w, c, s) }, func() { st.closed = true })
	return st
}

func (st *reviewSheet) display(item model.ReviewItem, keepingEdits bool) {
	st.item = item.Clone()
	if !keepingEdits {
		st.text = item.Payload.EditorText()
	}
}

// edited is whether the editor holds something other than the version on screen.
func (st *reviewSheet) edited() bool { return st.text != st.item.Payload.EditorText() }

// follow takes the item as another Device or the Runner left it. An edit in progress keeps its
// version until Approve, which then offers the new one. It answers whether the item is gone.
func (st *reviewSheet) follow() bool {
	latest := store.Review(st.item.ID)
	if latest == nil {
		return true
	}
	if st.busy || latest.Revision == st.item.Revision {
		return false
	}
	sameVersion := latest.Pending() && latest.Version == st.item.Version
	if sameVersion || !(latest.Pending() && st.edited()) {
		st.display(*latest, sameVersion)
	}
	return false
}

// reviewPlugin is the installed plugin, or account of one, a call goes to; nil once it is gone.
func reviewPlugin(item model.ReviewItem) *model.InstalledPlugin {
	if runner := store.Device(item.RunnerID); runner != nil {
		for i := range runner.Plugins {
			if runner.Plugins[i].ID == item.Payload.PluginID {
				return &runner.Plugins[i]
			}
		}
	}
	return nil
}

// reviewPluginName is the plugin a call goes to, as its Runner lists it and the permission card
// names it: "GitHub", or "Gmail · Work" for one of several accounts.
func reviewPluginName(item model.ReviewItem) string {
	if plugin := reviewPlugin(item); plugin != nil {
		return plugin.Name
	}
	return item.Target.Account
}

// reviewTitle is "Chef wants to run a command on Workbench", as the permission card says it.
func reviewTitle(item model.ReviewItem) string {
	bot := L("The bot")
	if b := store.Bot(item.BotID); b != nil {
		bot = b.Name
	}
	switch item.Payload.Kind {
	case "draft":
		return L("%@ wrote a draft", bot)
	case "shell":
		name := L("its Runner")
		if runner := store.Device(item.RunnerID); runner != nil {
			name = runner.Name
		}
		return bot + " " + L("wants to run a command on %@", name)
	}
	return bot + " " + L("wants to use %@", reviewPluginName(item))
}

func (st *reviewSheet) view(w *appWindow, c *ui.Context, s *sheet) {
	if st.follow() {
		s.dismiss()
		return
	}
	p := colors(c)
	item := st.item
	pending := item.Pending()
	o := sheetOptions{Title: reviewTitle(item), Subtitle: item.Rationale, Width: 520, Confirm: L("Done"), NoCancel: true}
	if pending {
		o.Confirm, o.NoCancel, o.ConfirmDisabled, o.ReturnInContent = L("Approve"), false, st.busy, true
		// Rejecting is the one way to say no, so it stands apart from Cancel, which only closes.
		o.Leading = func() {
			if pushButton(c, L("Reject"), pushOptions{Kind: buttonDestructive, Disabled: st.busy}).Clicked() {
				st.decide(w, s, L("Couldn't reject"), func(done func(model.ReviewItem, error)) { store.RejectReview(st.item.Clone(), done) })
			}
		}
	}
	result := sheetFrame(c, o, func() {
		// The editor is a field, as the memory sheet's is; what the bot said and what came of it
		// are cards, as the routine sheet's are.
		label, mono := L("Arguments"), true
		switch item.Payload.Kind {
		case "draft":
			label, mono = L("Draft"), false
		case "shell":
			label = L("Command")
		}
		editor := textArea(c.Key("review-editor:"+item.ID), &st.text, 0, fieldOptions{Mono: mono, Label: label, ReadOnly: !pending || st.busy, AutoFocus: pending})
		// A command is usually a line; a draft or a call's arguments get room to edit.
		if item.Payload.Kind == "shell" {
			lines := min(max(strings.Count(st.text, "\n")+1, 2), 6)
			editor.Height(float32(lines)*12*1.4 + 12)
		} else {
			editor.Height(200)
		}
		section(c, L("Details"), sectionCaption, nil, func(k *card) {
			if state := item.StateText(); state != "" {
				tint := p.Label2
				if item.State == "uncertain" || item.State == "failed" {
					tint = p.Orange
				}
				keyValueRow(c, k, L("Status"), state, false, &tint)
			}
			keyValueRow(c, k, L("Account"), item.Target.Account, false, nil)
			keyValueRow(c, k, L("Resource"), item.Target.Resource, false, nil)
			if len(item.Preconditions.Files) > 0 {
				paths := make([]string, 0, len(item.Preconditions.Files))
				for _, file := range item.Preconditions.Files {
					paths = append(paths, file.Path)
				}
				keyValueRow(c, k, L("Files"), strings.Join(paths, "\n"), true, nil).Tooltip(L("If these files change, this needs another review."))
			}
			// Why it needs another look, or why it may have run without a result.
			if item.Outcome != nil && (pending || item.State == "uncertain") {
				noteRow(c, k, item.Outcome.Summary, nil)
			}
		})
		if output := item.Output(); output != "" {
			section(c, L("Output"), sectionCaption, nil, func(k *card) {
				k.row(ui.Scroll(c).MaxHeight(160).Padding(8, 12).Children(func() {
					ui.Text(c, output).Font(monoFont).FontSize(11).LineHeight(1.45).Selectable()
				}))
			})
		}
	})
	switch {
	case result.Confirmed && pending:
		st.approve(w, s)
	case result.Confirmed || result.Cancelled:
		s.dismiss()
	}
}

func (st *reviewSheet) approve(w *appWindow, s *sheet) {
	if st.busy {
		return
	}
	// Edited here while it changed elsewhere: approving would mean the wrong text.
	if latest := store.Review(st.item.ID); latest != nil && latest.Version != st.item.Version && st.edited() {
		w.showAlert(alertOptions{
			Message:     L("This changed on another Device"),
			Informative: L("Show Latest discards your changes and shows the current version to review."),
			Buttons:     []alertButton{{Title: L("Show Latest")}, {Title: L("Cancel")}},
		}, func(index int) {
			if latest := store.Review(st.item.ID); index == 0 && latest != nil {
				st.display(*latest, false)
			}
		})
		return
	}
	var payload json.RawMessage
	if st.edited() {
		var err error
		if payload, err = st.item.Payload.EditedJSON(st.text); err != nil {
			w.showAlert(alertOptions{Message: L("Couldn't approve"), Informative: model.ErrorText(err)}, nil)
			return
		}
	}
	st.decide(w, s, L("Couldn't approve"), func(done func(model.ReviewItem, error)) { store.ApproveReview(st.item.Clone(), payload, done) })
}

// decide sends a decision on the version shown. Done, the sheet closes; refused, it shows the item
// as it now is and says why, keeping an edit when the version is the same.
func (st *reviewSheet) decide(w *appWindow, s *sheet, failure string, request func(done func(model.ReviewItem, error))) {
	if st.busy {
		return
	}
	shown := st.item.Clone()
	st.busy = true
	request(func(_ model.ReviewItem, err error) {
		if st.closed {
			return
		}
		st.busy = false
		if err == nil {
			s.dismiss()
			return
		}
		if latest := store.Review(shown.ID); latest != nil && latest.Revision != shown.Revision {
			st.display(*latest, latest.Pending() && latest.Version == shown.Version)
		}
		w.showAlert(alertOptions{Message: failure, Informative: model.ErrorText(err)}, nil)
	})
}

// inspectorReviews is what the chat's bots left for the user to approve, oldest first, while any
// waits or runs; a row opens it. How each ended stays in the chat, so the section goes once none
// is open.
func (m *mainWindow) inspectorReviews(c *ui.Context, chat *model.Chat) {
	items := store.OpenReviewsFor(chat.ID)
	if len(items) == 0 {
		return
	}
	section(c.Key("reviews:"+chat.ID), L("Waiting for review"), sectionCaption, nil, func(k *card) {
		for _, item := range items {
			o := statusRowOptions{Title: item.Headline(), Subtitle: item.Rationale, SubtitleLines: 2, State: item.StateText(), Clickable: true, Tooltip: item.Rationale}
			switch item.Payload.Kind {
			case "draft":
				o.Symbol = "doc.text"
			case "shell":
				o.Symbol = "terminal"
			default:
				o.Title, o.Symbol = reviewPluginName(item), "puzzlepiece.extension"
				// The logo is the service's, which every account of it shares.
				if plugin := reviewPlugin(item); plugin != nil {
					o.Symbol, o.PluginID = plugin.Symbol(), plugin.MarketplaceID()
				}
			}
			if _, row := statusRow(c.Key("review:"+item.ID), k, o); row.Clicked {
				m.presentReview(item)
			}
		}
	})
}
