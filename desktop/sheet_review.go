package main

import (
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// reviewSheet retains the displayed version and editable strings across build passes.
// Replies touch only persistent state, on the store's ordered main-thread post queue.
type reviewSheet struct {
	item                      model.ReviewItem
	account, resource, reason string
	payload                   string
	status                    string
	busy, closed              bool
	requests                  uint64
}

func (st *reviewSheet) display(item model.ReviewItem) {
	st.item = item.Clone()
	st.account, st.resource, st.reason = item.Target.Account, item.Target.Resource, item.Rationale
	st.payload, st.status = item.Payload.EditorText(), ""
}

func (st *reviewSheet) dirty() bool {
	return st.account != st.item.Target.Account || st.resource != st.item.Target.Resource ||
		st.reason != st.item.Rationale || st.payload != st.item.Payload.EditorText()
}

func (w *appWindow) presentReview(item model.ReviewItem) *reviewSheet {
	st := &reviewSheet{}
	st.display(item)
	w.present(func(c *ui.Context, s *sheet) { st.view(c, s) }, func() { st.closed = true; st.requests++ })
	return st
}

func (st *reviewSheet) stale() bool {
	latest := store.Review(st.item.ID)
	return latest == nil || latest.Revision != st.item.Revision
}

func (st *reviewSheet) view(c *ui.Context, s *sheet) {
	p := colors(c)
	latest := store.Review(st.item.ID)
	stale := st.stale()
	editable := !st.busy && st.item.Editable() && latest != nil && latest.Editable()
	result := sheetFrame(c, sheetOptions{
		Title: L("Review item"), Subtitle: L("Approve the saved version to resume this proposal on its Runner."),
		Width: 600, Confirm: L("Done"), NoCancel: true, ReturnInContent: true,
	}, func() {
		fieldRow := func(label, key string, value *string) {
			ui.Row(c).Gap(12).AlignItems(ui.Center).Children(func() {
				ui.Text(c, label).Width(68).FontSize(12).TextColor(p.Label2)
				textField(c.Key(key), value, fieldOptions{Label: label, ReadOnly: !editable, Disabled: st.busy}).Grow(1).MinWidth(0)
			})
		}
		fieldRow(L("Account"), "review-account", &st.account)
		fieldRow(L("Resource"), "review-resource", &st.resource)
		fieldRow(L("Rationale"), "review-rationale", &st.reason)
		bot, runner := st.item.BotID, st.item.RunnerID
		if b := store.Bot(bot); b != nil {
			bot = b.Name
		}
		if d := store.Device(runner); d != nil {
			runner = d.Name
		}
		ui.Text(c, L("%@ on %@", bot, runner)).FontSize(12).TextColor(p.Label2)
		if st.item.Payload.Kind == "plugin" {
			ui.Text(c, st.item.Payload.PluginID+" / "+st.item.Payload.ServerName+" / "+st.item.Payload.Tool).FontSize(12).TextColor(p.Label2).Selectable()
		}
		if len(st.item.Preconditions.Files) > 0 {
			paths := make([]string, 0, len(st.item.Preconditions.Files))
			for _, file := range st.item.Preconditions.Files {
				paths = append(paths, file.Path)
			}
			ui.Text(c, L("Guarded files: %@", strings.Join(paths, ", "))).FontSize(12).TextColor(p.Label2).LineHeight(1.4).Selectable()
		}
		label := L("Draft")
		if st.item.Payload.Kind != "draft" {
			label = L("Proposed call arguments")
		}
		textArea(c.Key("review-payload"), &st.payload, 0, fieldOptions{
			Label: label, Mono: st.item.Payload.Kind != "draft", ReadOnly: !editable || !st.item.Payload.Supported(), Disabled: st.busy,
		}).Height(220)
		ui.Text(c, L("Version %d · %@", st.item.Version, st.item.StateText())).FontSize(12).TextColor(p.Label2)
		if st.item.Outcome != nil {
			ui.Text(c, st.item.Outcome.Summary).FontSize(12).TextColor(p.Label2).LineHeight(1.4).Selectable()
		}
		if stale {
			ui.Text(c, L("This review changed. Reload it before deciding.")).FontSize(12).TextColor(p.Orange).LineHeight(1.4)
		}
		if st.status != "" {
			ui.Text(c, st.status).FontSize(12).TextColor(p.Orange).LineHeight(1.4)
		}
		// Click queries run after bound fields are constructed, so edits in the same frame
		// reach the local guard before a version-bound request can be admitted.
		ui.Row(c).Gap(8).Children(func() {
			if pushButton(c.Key("review-save"), L("Save Changes"), pushOptions{Disabled: !editable || stale || !st.item.Payload.Supported()}).Clicked() {
				st.act("edit")
			}
			if pushButton(c.Key("review-approve"), L("Approve"), pushOptions{Disabled: !editable || stale || !st.item.Payload.Supported()}).Clicked() {
				st.act("approve")
			}
			if pushButton(c.Key("review-reject"), L("Reject"), pushOptions{Disabled: !editable || stale}).Clicked() {
				st.act("reject")
			}
			if pushButton(c.Key("review-cancel"), L("Cancel Item"), pushOptions{Disabled: !editable || stale}).Clicked() {
				st.act("cancel")
			}
			if pushButton(c.Key("review-reload"), L("Reload"), pushOptions{Disabled: st.busy}).Clicked() {
				st.reload()
			}
		})
	})
	if result.Confirmed || result.Cancelled {
		s.dismiss()
	}
}

func (st *reviewSheet) act(action string) {
	if st.busy || st.closed || st.stale() {
		return
	}
	if action == "approve" && st.dirty() {
		st.status = L("Save your changes, then review and approve the new version.")
		return
	}
	var edits *model.ReviewEdits
	if action == "edit" {
		payload, err := st.item.Payload.EditedJSON(st.payload)
		if err != nil {
			st.status = model.ErrorText(err)
			return
		}
		edits = &model.ReviewEdits{Payload: payload, Target: model.ReviewTarget{Account: st.account, Resource: st.resource}, Rationale: st.reason}
	}
	st.busy = true
	st.requests++
	request := st.requests
	store.ChangeReview(st.item.Clone(), action, edits, func(item model.ReviewItem, err error) {
		if st.closed || st.requests != request {
			return
		}
		st.busy = false
		if err != nil {
			st.status = model.ErrorText(err)
			return
		}
		st.display(item)
	})
}

func (st *reviewSheet) reload() {
	if st.busy || st.closed {
		return
	}
	st.busy = true
	st.requests++
	request := st.requests
	store.RefreshReview(st.item.ID, func(item model.ReviewItem, err error) {
		if st.closed || st.requests != request {
			return
		}
		st.busy = false
		if err != nil {
			st.status = model.ErrorText(err)
			return
		}
		st.display(item)
	})
}

func (m *mainWindow) inspectorReviews(c *ui.Context, chat *model.Chat) {
	p := colors(c)
	section(c.Key("review-queue:"+chat.ID), L("Review queue"), sectionCaption, nil, func(k *card) {
		items := store.ReviewsFor(chat.ID)
		if len(items) == 0 {
			noteRow(c, k, L("Drafts and proposed actions wait here for your review."), nil)
			return
		}
		for _, item := range items {
			_, result := actionRow(c.Key("review:"+item.ID), k, item.Target.Resource, actionRowOptions{
				Value: item.StateText(), Tint: &p.Label2, Action: L("Review…"), Tooltip: item.Rationale,
			})
			if result.Action {
				m.presentReview(item)
			}
		}
	})
}
