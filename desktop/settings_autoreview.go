package main

import (
	"slices"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// autoReview is the switch, the model that reviews, and the rules, shared by every Device through
// the roster. Add and Edit use a sheet; a card's Always allow adds a rule here.
func (s settingsPane) autoReview(c *ui.Context) {
	p := colors(c)
	review := store.AutoReview
	s.frame(c, string(model.PaneAutoReview), func() {
		s.section(c, L("Auto-review"), nil, func(k *card) {
			label := autoReviewSwitchEntry().row
			s.mark(c, accessoryRow(c, k, label, "", func() {
				on := review.IsEnabled
				if settingsSwitch(c, &on, label, false) {
					next := store.AutoReview
					next.IsEnabled = on
					store.SetAutoReview(next)
				}
			}), label)
			noteRow(c, k, L("Lorca checks each action before it runs and asks you first when needed. Add rules to customize what bots can do automatically."), nil)
		})
		s.reviewsWith(c, review)
		addRule := func() {
			if hoverButton(c, hoverButtonOptions{Symbol: "plus", Size: 13, Tooltip: L("Add rule")}).Clicked() {
				s.w.presentRuleEditor(nil)
			}
		}
		s.section(c, autoReviewRulesEntry().row, addRule, func(k *card) {
			if len(review.Rules) == 0 {
				noteRow(c, k, L("No rules yet. Always allow on a card adds one, or write one below."), nil)
				return
			}
			removed := ""
			for _, rule := range review.Rules {
				ui.Column(c.Key(firstNonEmpty(rule.ID, rule.Text))).Children(func() {
					row := k.row(ui.Column(c).Gap(5).Padding(10, 8, 8, 12).Label(rule.Text))
					row.Children(func() {
						ui.Text(c, rule.Text).MaxWidth(900).Padding(0, 4, 0, 0).FontSize(12).LineHeight(1.4).Selectable()
						ui.Row(c).Gap(4).Children(func() {
							behavior := model.BehaviorTitle(rule.Behavior)
							ui.Text(c, behavior).Grow(1).Shrink(1).MinWidth(0).FontSize(textCaption).TextColor(p.Label2).Tooltip(behavior)
							if hoverButton(c, hoverButtonOptions{Symbol: "square.and.pencil", Size: 13, Tooltip: L("Edit rule")}).Clicked() {
								s.w.presentRuleEditor(&rule)
							}
							if hoverButton(c, hoverButtonOptions{Symbol: "trash", Size: 13, Tooltip: L("Delete rule")}).Clicked() {
								removed = rule.ID
							}
						})
					})
					s.mark(c, row, rule.Text)
				})
			}
			if removed != "" {
				next := store.AutoReview
				next.Rules = slices.DeleteFunc(slices.Clone(next.Rules), func(rule model.AutoReviewRule) bool { return rule.ID == removed })
				store.SetAutoReview(next)
			}
		})
		s.footnote(c, L("Read-only commands and commands inside Lorca's own folders run at once. Auto-review checks effectful plugin actions and every other shell command before they run: the model under Reviews with (a small, fast model on the bot's provider unless you pick another) applies your rules and latest request, so safe work normally runs automatically and risky work asks. Off, every such action asks. Write one short, natural-language rule for each action; \"Ask first\" takes priority if rules conflict. Built-in safety checks always apply."))
	})
}

// reviewsWith is the model that reviews, as the inspector's Runs with picks a bot's: the bot's own
// provider, or any connected one, decision providers included, and then any of its models. A
// provider no longer connected reads as the bot's own, as the CLI falls back to it.
func (s settingsPane) reviewsWith(c *ui.Context, review model.AutoReview) {
	kinds := store.ReviewProviderKinds()
	provider := review.Provider
	if !slices.Contains(kinds, provider) {
		provider = ""
	}
	s.section(c, autoReviewModelEntry().row, nil, func(k *card) {
		providers := []popUpOption{{Value: "", Label: L("Bot's provider")}}
		for i, kind := range kinds {
			providers = append(providers, popUpOption{Value: kind, Label: model.ProviderName(kind, store.Providers), Separated: i == 0})
		}
		providerLabel := L("Provider")
		accessoryRow(c, k, providerLabel, "", func() {
			// A new provider starts on its review model, which the CLI picks.
			if picked, changed, _ := popUpButton(c, popUp{Options: providers, Value: provider, Style: popUpSettings, Label: providerLabel}); changed {
				store.SetReviewModel(picked, "")
			}
		})
		if provider == "" {
			return
		}
		// Every model of the provider, decision models too; a stored one it no longer lists still
		// shows, by its id.
		credential := store.Credential(provider)
		decides := credential != nil && credential.Decides()
		var options []popUpOption
		listed := false
		for _, each := range model.ReviewModels(model.WithCustomModels(store.Models, store.Providers), provider) {
			options = append(options, popUpOption{Value: each.ID, Label: each.Label})
			if each.ID == review.Model {
				listed = true
				decides = decides || each.Decides
			}
		}
		if !listed && review.Model != "" {
			options = append(options, popUpOption{Value: review.Model, Label: review.Model})
		}
		modelLabel := L("Model")
		accessoryRow(c, k, modelLabel, "", func() {
			if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: review.Model, Style: popUpSettings, Label: modelLabel}); changed {
				store.SetReviewModel(provider, picked)
			}
		})
		if decides {
			noteRow(c, k, L("A decision model picks allow or what the action could harm, and writes no rule, so a card it pauses offers Allow once and Deny."), nil)
		}
	})
}

// presentRuleEditor adds a rule, or edits one: Add and Edit share one sheet. Rule text is
// editable; a rule for an exact plugin tool keeps its tool, while its Allow/Ask behavior can
// still change.
func (w *appWindow) presentRuleEditor(rule *model.AutoReviewRule) {
	exact := rule != nil && rule.Tool != ""
	text, behavior := "", "allow"
	title, subtitle, confirm := L("Add rule"), L("Describe when a bot should be allowed automatically or asked first."), L("Add rule")
	if rule != nil {
		text = rule.Text
		if rule.Behavior == "ask" {
			behavior = "ask"
		}
		title, confirm = L("Edit rule"), L("Save")
	}
	if exact {
		subtitle = L("Exact actions cannot be changed. Delete this rule and allow a different action instead.")
	}
	w.present(func(c *ui.Context, s *sheet) {
		p := colors(c)
		result := sheetFrame(c, sheetOptions{Title: title, Subtitle: subtitle, Width: 500, Confirm: confirm, ReturnInContent: true}, func() {
			ui.Column(c).Gap(5).Children(func() {
				ui.Text(c, strings.ToUpper(L("Rule"))).FontSize(10).FontWeight(600).TextColor(p.Label3).LetterSpacing(0.2)
				textArea(c, &text, 0, fieldOptions{ReadOnly: exact, AutoFocus: !exact, Label: L("Rule")}).Height(90).FontSize(12)
			})
			section(c, L("Behavior"), sectionCaption, nil, func(k *card) {
				accessoryRow(c, k, L("Auto-review"), "", func() {
					picked, changed, _ := popUpButton(c, popUp{
						Options: []popUpOption{{Value: "allow", Label: model.BehaviorTitle("allow")}, {Value: "ask", Label: model.BehaviorTitle("ask")}},
						Value:   behavior,
						Style:   popUpSettings,
						Label:   L("Auto-review"),
					})
					if changed {
						behavior = picked
					}
				})
			})
		})
		if result.Cancelled {
			s.dismiss()
		}
		if !result.Confirmed {
			return
		}
		value := text
		if exact {
			value = rule.Text
		}
		value = strings.TrimSpace(value)
		if value == "" {
			beep()
			return
		}
		settingsSaveRule(rule, value, behavior)
		s.dismiss()
	}, nil)
}

// settingsSaveRule saves a rule the editor changed, or adds the one it wrote.
func settingsSaveRule(rule *model.AutoReviewRule, text, behavior string) {
	next := store.AutoReview
	rules := slices.Clone(next.Rules)
	index := -1
	if rule != nil {
		index = slices.IndexFunc(rules, func(each model.AutoReviewRule) bool { return each.ID == rule.ID })
	}
	if index >= 0 {
		if rules[index].Tool == "" {
			rules[index].Text = text
		}
		rules[index].Behavior = behavior
	} else {
		rules = append(rules, model.AutoReviewRule{Text: text, Behavior: behavior})
	}
	next.Rules = rules
	store.SetAutoReview(next)
}
