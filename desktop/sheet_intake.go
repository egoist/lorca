package main

import (
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A channel's Intake, in its sheet: whether it takes on work by itself, who can ask it directly, the
// rules it reads each message against, and what it decided lately. The channel's Runner keeps all
// of it: a change replaces the whole intake there, and the decisions come from there, so a channel
// on a Runner that is offline says so and offers Retry. Off keeps the people and rules for when it
// is turned on again.

// channelSheet is what the channel's sheet keeps while it is up.
type channelSheet struct {
	w         *appWindow
	channelID string
	closed    bool
	// decisions are what the Runner last answered, newest first; failure is why it didn't.
	decisions []model.IntakeDecision
	asked     bool
	loading   bool
	failure   string
	loads     int
	// showsIgnored lists the messages intake left alone.
	showsIgnored bool
}

func (st *channelSheet) intake() *model.ChannelIntake {
	if channel := store.Channel(st.channelID); channel != nil {
		return channel.Intake
	}
	return nil
}

// write replaces the channel's intake; the store takes a refused change back, and the user hears why.
func (st *channelSheet) write(next *model.ChannelIntake) {
	store.SetChannelIntake(st.channelID, next, func(err error) {
		if err != nil {
			st.w.showAlert(alertOptions{Message: L("Couldn’t change Intake"), Informative: model.ErrorText(err)}, nil)
		}
	})
}

// edit changes the intake the store has now, if the channel still has one.
func (st *channelSheet) edit(change func(model.ChannelIntake) model.ChannelIntake) {
	if intake := st.intake(); intake != nil {
		next := change(*intake)
		st.write(&next)
	}
}

func (st *channelSheet) setMode(value string) {
	base := model.ChannelIntake{}
	if intake := st.intake(); intake != nil {
		base = *intake
	}
	next := base.WithMode(model.IntakeMode(value))
	st.write(&next)
}

func (st *channelSheet) loadDecisions() {
	st.asked, st.loading, st.failure = true, true, ""
	st.loads++
	load := st.loads
	store.IntakeDecisions(st.channelID, func(decisions []model.IntakeDecision, err error) {
		if st.closed || load != st.loads {
			return
		}
		st.loading = false
		if err != nil {
			st.failure = model.ErrorText(err)
			return
		}
		st.decisions = decisions
	})
}

// intakeSections are the choice and its note, and while it isn't Off, who can ask, the rules, and
// the decisions.
func (st *channelSheet) intakeSections(c *ui.Context, s *sheet, channel *model.Channel, openChat func(chatID string)) {
	section(c.Key("intake"), L("Intake"), sectionCaption, nil, func(k *card) {
		value, note := string(model.IntakeOff), L("Each message the channel takes starts a turn.")
		if intake := channel.Intake; intake != nil && intake.Mode != model.IntakeOff {
			value = string(intake.Mode)
			note = L("The bot reads each message against your rules and records what it would take on. Someone you list still gets what they ask for.")
			if intake.Mode == model.IntakeAuto {
				note = L("The bot takes on what your rules cover, or what someone you list asks for, as a task in its own chat.")
			}
		}
		options := []popUpOption{
			{Value: string(model.IntakeOff), Label: L("Off")},
			{Value: string(model.IntakeObserve), Label: L("Only Record")},
			{Value: string(model.IntakeAuto), Label: L("On")},
		}
		if picked, changed := popUpRow(c.Key("intake-mode"), k, L("Takes on work"), popUp{Options: options, Value: value, Label: L("Takes on work")}, false); changed {
			st.setMode(picked)
		}
		noteRow(c.Key("intake-note"), k, note, nil)
	})
	intake := channel.Intake
	if intake == nil || intake.Mode == model.IntakeOff {
		return
	}
	st.peopleSection(c, intake, channel.Service)
	st.rulesSection(c, intake)
	st.decisionsSection(c, s, intake, openChat)
}

// peopleSection lists who can ask directly, and Add… asks for someone's id.
func (st *channelSheet) peopleSection(c *ui.Context, intake *model.ChannelIntake, service string) {
	p := colors(c)
	var rows func(k *card)
	if len(intake.People) > 0 {
		rows = func(k *card) {
			for _, person := range intake.People {
				value := ""
				if person.Name != "" {
					value = person.ID
				}
				_, row := actionRow(c.Key("person-"+person.ID), k, person.Title(), actionRowOptions{Value: value, Tint: &p.Label2, Action: L("Remove"), Tooltip: person.ID})
				if row.Action {
					id := person.ID
					st.edit(func(intake model.ChannelIntake) model.ChannelIntake { return intake.WithoutPerson(id) })
				}
			}
		}
	}
	section(c.Key("people"), L("Can Ask Directly"), sectionCaption, func() {
		if linkButton(c, L("Add…"), false).Clicked() {
			st.askPerson(service)
		}
	}, rows)
}

// askPerson asks for someone's id on the service; only that counts, and the name is for the list.
func (st *channelSheet) askPerson(service string) {
	id, name := "", ""
	informative := L("Paste their Telegram user id, a number such as 123456789. Only the id counts, since anyone can change their name.")
	if service == "slack" {
		informative = L("Paste their Slack member id, such as U0123456789. Only the id counts, since anyone can change their name.")
	}
	st.w.showAlert(alertOptions{
		Message:     L("Add Someone"),
		Informative: informative,
		Buttons:     []alertButton{{Title: L("Add")}, {Title: L("Cancel")}},
		Accessory: func(c *ui.Context) {
			ui.Column(c).Gap(8).Children(func() {
				textField(c, &id, fieldOptions{Placeholder: L("User ID"), AutoFocus: true, Label: L("User ID")})
				textField(c, &name, fieldOptions{Placeholder: L("Name (optional)"), Label: L("Name (optional)")})
			})
		},
	}, func(answer int) {
		id, name := strings.TrimSpace(id), strings.TrimSpace(name)
		if answer != 0 || id == "" {
			return
		}
		st.edit(func(intake model.ChannelIntake) model.ChannelIntake { return intake.WithPerson(id, name) })
	})
}

// rulesSection lists the rules, each with its switch and opening its sheet.
func (st *channelSheet) rulesSection(c *ui.Context, intake *model.ChannelIntake) {
	p := colors(c)
	section(c.Key("rules"), L("Rules"), sectionCaption, func() {
		if hoverButton(c, hoverButtonOptions{Symbol: "plus", Size: 13, Tooltip: L("Add Rule")}).Clicked() {
			st.presentRule(nil)
		}
	}, func(k *card) {
		if len(intake.Rules) == 0 {
			noteRow(c.Key("rules-empty"), k, L("No rules yet: it only takes on what someone you list asks for."), nil)
		}
		for _, rule := range intake.Rules {
			on, tint := rule.Enabled, p.Label2
			if !on {
				tint = p.Label3
			}
			detail := []ui.Span{{Text: strings.Join(strings.Fields(rule.Instruction), " "), Color: p.Label2}}
			id := rule.ID
			change := func(enabled bool) {
				st.edit(func(intake model.ChannelIntake) model.ChannelIntake {
					current := intake.Rule(id)
					if current == nil {
						return intake
					}
					edited := *current
					edited.Enabled = enabled
					return intake.WithRule(edited)
				})
			}
			if switchRow(c.Key("rule-"+id), k, "checklist", tint, rule.Name, detail, &on, L("Use “%@”", rule.Name), rule.Instruction, change) {
				edited := rule
				st.presentRule(&edited)
			}
		}
	})
}

// decisionsSection is the newest of what intake decided, asked of the Runner as it first shows.
func (st *channelSheet) decisionsSection(c *ui.Context, s *sheet, intake *model.ChannelIntake, openChat func(chatID string)) {
	p := colors(c)
	if !st.asked {
		st.loadDecisions()
	}
	shown := model.ShownDecisions(st.decisions, st.showsIgnored)
	section(c.Key("decisions"), L("Decisions"), sectionCaption, nil, func(k *card) {
		switch {
		case st.failure != "":
			if _, row := actionRow(c.Key("decisions-failed"), k, L("Decisions"), actionRowOptions{Value: st.failure, Tint: &p.Label2, Action: L("Retry"), Tooltip: st.failure}); row.Action {
				st.loadDecisions()
			}
		case st.loading:
			keyValueRow(c.Key("decisions-loading"), k, L("Decisions"), L("Loading…"), false, &p.Label2)
		case len(shown) == 0:
			noteRow(c.Key("decisions-empty"), k, L("None yet"), nil)
		}
		for _, decision := range shown {
			st.decisionRow(c.Key(decision.ID), k, s, decision, intake, openChat)
		}
		if model.HasIgnored(st.decisions) {
			accessoryRow(c.Key("show-ignored"), k, L("Show Ignored"), "", func() {
				toggleSwitch(c, &st.showsIgnored, true).Label(L("Show Ignored"))
			})
		}
	})
}

// decisionRow is what intake made of a message: the title or reason over the kind, who wrote it, and
// when. A click opens its task, else its conversation, and its menu lets the sender ask directly.
// Everything but the kind comes from outside Lorca, so it stays plain text.
func (st *channelSheet) decisionRow(c *ui.Context, k *card, s *sheet, decision model.IntakeDecision, intake *model.ChannelIntake, openChat func(chatID string)) {
	p := colors(c)
	parts := []string{decision.Kind.Title()}
	if name := decision.SenderName(); name != "" {
		parts = append(parts, name)
	}
	parts = append(parts, model.Ago(decision.Time()))
	task := store.DurableTask(decision.TaskID)
	chat := store.Chat(decision.ConversationID)
	if chat != nil && chat.IsWorkChat() {
		chat = nil
	}
	opens := task != nil || (chat != nil && openChat != nil)
	subject := decision.Subject()
	r := k.row(rowBox(c).MinHeight(44).Label(subject).Tooltip(subject))
	clicked := false
	if opens {
		r.Cursor(ui.CursorPointer).Role(ui.RoleButton)
		if r.Hovered() {
			r.Background(p.RowHover)
		}
		clicked = r.Clicked()
	}
	r.Children(func() {
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			if subject != "" {
				ui.Text(c, subject).FontSize(12.5).FontWeight(500).MaxLines(2)
			}
			ui.Text(c, strings.Join(parts, " · ")).FontSize(textCaption).TextColor(p.Label2).SingleLine()
		})
	})
	if decision.SenderID != "" && !intake.Lists(decision.SenderID) {
		senderID, sender, name := decision.SenderID, decision.Sender, decision.SenderName()
		r.ContextMenu(func(menu *ui.Menu) {
			if menu.Item(L("Let %@ Ask Directly", name)).Chosen() {
				st.edit(func(intake model.ChannelIntake) model.ChannelIntake { return intake.WithPerson(senderID, sender) })
			}
		})
	}
	switch {
	case !clicked:
	case task != nil:
		st.w.presentDurableTask(decision.ConversationID, task)
	default:
		s.dismiss()
		openChat(chat.ID)
	}
}

// ruleSheet edits one rule, or a new one when it has no id. It waits for the Runner, so a change the
// Runner refuses, such as a name past its limit, keeps the user's words.
type ruleSheet struct {
	parent            *channelSheet
	id                string
	enabled           bool
	name, instruction string
	busy, closed      bool
}

func (st *channelSheet) presentRule(rule *model.IntakeRule) {
	r := &ruleSheet{parent: st, enabled: true}
	if rule != nil {
		r.id, r.enabled, r.name, r.instruction = rule.ID, rule.Enabled, rule.Name, rule.Instruction
	}
	st.w.present(r.view, func() { r.closed = true })
}

func (r *ruleSheet) canSave() bool {
	return !r.busy && strings.TrimSpace(r.name) != "" && strings.TrimSpace(r.instruction) != ""
}

func (r *ruleSheet) view(c *ui.Context, s *sheet) {
	title, confirm := L("Rule"), L("Save")
	if r.id == "" {
		title, confirm = L("New Rule"), L("Add Rule")
	}
	result := sheetFrame(c, sheetOptions{
		Title: title, Width: 520, Confirm: confirm, ConfirmDisabled: !r.canSave(),
		Leading: func() {
			if r.id != "" && pushButton(c.Key("delete-rule"), L("Delete…"), pushOptions{Kind: buttonDestructive, Disabled: r.busy}).Clicked() {
				r.confirmDelete(s)
			}
		},
	}, func() {
		formRow(c.Key("name"), L("Name"), false, func() {
			textField(c, &r.name, fieldOptions{Placeholder: L("Name"), AutoFocus: true, Label: L("Name")}).Grow(1).MinWidth(0)
		})
		// Return starts a new line here.
		formRow(c.Key("instruction"), L("Instruction"), true, func() {
			textArea(c, &r.instruction, 0, fieldOptions{Placeholder: L("The kind of work, and how far it goes"), Label: L("Instruction")}).
				Lines(4, 12).Grow(1).MinWidth(0).MinHeight(72)
		})
	})
	switch {
	case result.Cancelled:
		s.dismiss()
	case result.Confirmed:
		r.save(s)
	}
}

func (r *ruleSheet) save(s *sheet) {
	if !r.canSave() {
		return
	}
	intake := r.parent.intake()
	if intake == nil {
		s.dismiss()
		return
	}
	id := r.id
	if id == "" {
		id = intake.NewRuleID()
	}
	next := intake.WithRule(model.IntakeRule{ID: id, Name: strings.TrimSpace(r.name), Enabled: r.enabled, Instruction: strings.TrimSpace(r.instruction)})
	r.write(s, &next)
}

func (r *ruleSheet) write(s *sheet, next *model.ChannelIntake) {
	r.busy = true
	store.SetChannelIntake(r.parent.channelID, next, func(err error) {
		if r.closed {
			return
		}
		r.busy = false
		if err != nil {
			r.parent.w.showAlert(alertOptions{Message: L("Couldn’t change Intake"), Informative: model.ErrorText(err)}, nil)
			return
		}
		s.dismiss()
	})
}

func (r *ruleSheet) confirmDelete(s *sheet) {
	r.parent.w.showAlert(alertOptions{
		Message:     L("Delete “%@”?", strings.TrimSpace(r.name)),
		Informative: L("The bot stops taking on what this rule covers."),
		Style:       alertWarning,
		Buttons:     []alertButton{{Title: L("Delete"), Destructive: true}, {Title: L("Cancel")}},
	}, func(answer int) {
		intake := r.parent.intake()
		if answer != 0 || intake == nil {
			return
		}
		next := intake.WithoutRule(r.id)
		r.write(s, &next)
	})
}
