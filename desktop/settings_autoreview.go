package main

import (
	"slices"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// autoReview is the switch, the provider that reviews, and the rules, shared by every Device
// through the roster. Add and Edit use a sheet; a card's Always allow adds a rule here.
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
		s.footnote(c, L("Read-only commands and commands inside Lorca's own folders run at once. Auto-review checks effectful plugin actions and every other shell command before they run: the review model of the bot's provider, or of the provider under Reviews with, applies your rules and latest request, so safe work normally runs automatically and risky work asks. Each provider's review model is in Providers: a small, fast one unless you pick another. Off, every such action asks. Write one short, natural-language rule for each action; \"Ask first\" takes priority if rules conflict. Built-in safety checks always apply."))
	})
}

// reviewsWith is the provider that reviews: the bot's own, or any connected one, decision providers
// included. Auto-review runs its review model, which Providers picks. A provider no longer
// connected reads as the bot's own, as the CLI falls back to it.
func (s settingsPane) reviewsWith(c *ui.Context, review model.AutoReview) {
	kinds := store.ReviewProviderKinds()
	provider := review.Provider
	if !slices.Contains(kinds, provider) {
		provider = ""
	}
	s.section(c, autoReviewProviderEntry().row, nil, func(k *card) {
		options := []popUpOption{{Value: "", Label: L("Bot's provider")}}
		for i, kind := range kinds {
			options = append(options, popUpOption{Value: kind, Label: model.ProviderName(kind, store.Providers), Separated: i == 0})
		}
		label := L("Provider")
		accessoryRow(c, k, label, "", func() {
			if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: provider, Style: popUpSettings, Label: label}); changed {
				store.SetReviewProvider(picked)
			}
		})
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
