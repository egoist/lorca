package main

import (
	"fmt"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Sheet state is persistent; elements and contexts remain in the current build pass.
type attentionSheet struct {
	w                 *appWindow
	busy, closed      bool
	errorText         string
	summaries, urgent bool
	coordinatorID     string
	seen              attentionPreferenceValues
}
type attentionPreferenceValues struct {
	summaries, urgent bool
	coordinatorID     string
}

func attentionPrefs() attentionPreferenceValues {
	p := store.Attention.Preferences
	id := ""
	if p.DefaultCoordinatorBotID != nil {
		id = *p.DefaultCoordinatorBotID
	}
	return attentionPreferenceValues{p.Summaries, p.UrgentDirect, id}
}
func (a *attentionSheet) syncPreferences() {
	current := attentionPrefs()
	if !a.busy && current != a.seen {
		a.resetPreferences()
	}
}
func (a *attentionSheet) resetPreferences() {
	a.seen = attentionPrefs()
	a.summaries, a.urgent, a.coordinatorID = a.seen.summaries, a.seen.urgent, a.seen.coordinatorID
}
func (m *mainWindow) presentAttention() {
	a := &attentionSheet{w: &m.appWindow}
	a.resetPreferences()
	m.present(a.view, func() { a.closed = true })
}
func (a *attentionSheet) completed(err error) {
	if a.closed {
		return
	}
	a.busy = false
	a.errorText = model.ErrorText(err)
	if err != nil {
		a.resetPreferences()
	}
	a.w.invalidate()
}
func (a *attentionSheet) savePreferences() {
	if a.busy {
		return
	}
	a.busy = true
	a.errorText = ""
	store.SetAttentionPreferences(a.summaries, a.urgent, a.coordinatorID, a.completed)
}
func attentionBotName(id string) string {
	if bot := store.Bot(id); bot != nil {
		return bot.Name
	}
	return L("Coordinator")
}
func (a *attentionSheet) view(c *ui.Context, s *sheet) {
	if store.HasIdentity == nil || !*store.HasIdentity {
		s.dismiss()
		return
	}
	a.syncPreferences()
	p := colors(c)
	result := sheetFrame(c, sheetOptions{Title: L("Attention"), Subtitle: L("Decisions, reviews, blockers and commitments across your chats"), Width: 660, Confirm: L("Done"), NoCancel: true}, func() {
		section(c.Key("attention-preferences"), L("Notifications"), sectionCaption, nil, func(k *card) {
			accessoryRow(c.Key("summaries"), k, L("Coordinator summaries"), "", func() {
				toggleSwitch(c.Key("summaries-switch"), &a.summaries, true).Label(L("Coordinator summaries")).Disabled(a.busy).OnChange(a.savePreferences)
			})
			accessoryRow(c.Key("urgent"), k, L("Urgent direct alerts"), "", func() {
				toggleSwitch(c.Key("urgent-switch"), &a.urgent, true).Label(L("Urgent direct alerts")).Disabled(a.busy).OnChange(a.savePreferences)
			})
			options := []popUpOption{{Value: "", Label: L("Chat owner")}}
			for _, bot := range store.Bots {
				options = append(options, popUpOption{Value: bot.ID, Label: bot.Name})
			}
			if picked, changed := popUpRow(c.Key("coordinator"), k, L("Default coordinator"), popUp{Options: options, Value: a.coordinatorID, Disabled: a.busy, Label: L("Default coordinator")}); changed {
				a.coordinatorID = picked
				a.savePreferences()
			}
		})
		if a.errorText != "" {
			ui.Text(c.Key("attention-error"), a.errorText).FontSize(12).TextColor(p.Red).LineHeight(1.4).Selectable()
		}
		ui.Row(c.Key("attention-refresh")).Children(func() {
			ui.Spacer(c)
			if pushButton(c, L("Refresh"), pushOptions{Disabled: a.busy}).Clicked() {
				a.busy = true
				a.errorText = ""
				store.RefreshAttention(a.completed)
			}
		})
		for _, brief := range store.Attention.Briefs {
			section(c.Key("brief-"+brief.CoordinatorBotID), L("%@’s brief", attentionBotName(brief.CoordinatorBotID)), sectionCaption, nil, func(k *card) {
				k.row(ui.Column(c).Gap(6).Padding(12).Children(func() {
					for _, decision := range brief.Decisions {
						attentionText(c, L("Decision: %@", decision))
					}
					for _, change := range brief.Changes {
						attentionText(c, L("Changed: %@", change))
					}
					attentionText(c, L("Next: %@", brief.NextAction))
					if linkButton(c, L("Open brief"), store.Chat(brief.ChatID) == nil).Clicked() {
						a.openSource(s, model.AttentionSource{ChatID: brief.ChatID, MessageID: brief.MessageID})
					}
				}))
			})
		}
		if len(store.Attention.Items) == 0 {
			ui.Text(c.Key("attention-empty"), L("Nothing needs attention")).FontSize(12).TextColor(p.Label2).Padding(8, 0)
		}
		for _, item := range store.Attention.Items {
			a.itemView(c.Key("item-"+item.ID), s, item)
		}
	})
	if result.Confirmed || result.Cancelled {
		s.dismiss()
	}
}
func attentionText(c *ui.Context, text string) {
	ui.Text(c, text).FontSize(12).LineHeight(1.4).Selectable()
}
func (a *attentionSheet) itemView(c *ui.Context, s *sheet, item model.AttentionItem) {
	p := colors(c)
	category := L("Important change")
	switch item.Category {
	case "review":
		category = L("Pending review")
	case "blocker":
		category = L("Blocker")
	case "commitment":
		category = L("Commitment")
	}
	tint := p.Label2
	if item.Urgent {
		category = L("Urgent")
		tint = p.Red
	}
	section(c, category+" · "+attentionBotName(item.CoordinatorBotID), sectionCaption, nil, func(k *card) {
		k.row(ui.Column(c).Gap(6).Padding(12).Children(func() {
			if item.Urgent {
				ui.Text(c, L("Urgent")).FontSize(11).FontWeight(600).TextColor(tint)
			}
			ui.Text(c, item.Title).FontSize(13).FontWeight(600)
			attentionText(c, item.Summary)
			attentionText(c, L("Next: %@", item.NextAction))
			for _, source := range item.Sources {
				title := L("Source chat")
				if chat := store.Chat(source.ChatID); chat != nil {
					title = store.Title(chat)
				}
				ref := source.ReviewID
				if ref == "" {
					ref = source.TaskID
				}
				if ref != "" {
					title += " · " + ref
				}
				if linkButton(c.Key(fmt.Sprintf("source-%s-%s-%s-%s", source.ChatID, source.MessageID, source.TaskID, source.ReviewID)), title, store.Chat(source.ChatID) == nil).Clicked() {
					a.openSource(s, source)
				}
			}
			if pushButton(c.Key("resolve-"+item.ID), L("Mark resolved"), pushOptions{Disabled: a.busy, Tooltip: item.Title}).Label(L("Mark resolved") + " · " + item.Title).Clicked() {
				a.busy = true
				a.errorText = ""
				store.ResolveAttention(item.ID, item.Revision, a.completed)
			}
		}))
	})
}
func (a *attentionSheet) openSource(s *sheet, source model.AttentionSource) {
	if store.Chat(source.ChatID) == nil {
		return
	}
	s.dismiss()
	if app.main != nil {
		app.main.selectChat(source.ChatID)
	}
}
