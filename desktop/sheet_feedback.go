package main

import (
	"encoding/json"
	"slices"
	"strconv"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Field state belongs to the modal, while elements and contexts belong to one build pass.
// Callbacks receive ordered main-thread replies from model.Store and ignore closed sheets.
type feedbackCaptureState struct {
	lastPayload                                                                string
	botID, chatID, messageID, original, edited, note, kind, targetKey, eventID string
	excluded, loading, saving, closed                                          bool
	targets                                                                    []model.FeedbackTargetChoice
	err                                                                        string
}

func (m *mainWindow) presentFeedbackCapture(chatID string, message *model.Message) {
	chat := store.Chat(chatID)
	if chat == nil || message == nil || !message.CanBeQuoted() {
		return
	}
	botID := message.Author.BotID
	if botID == "" {
		botID = chat.OwnerBotID
		if botID == "" && len(chat.BotIDs) > 0 {
			botID = chat.BotIDs[0]
		}
	}
	if store.Bot(botID) == nil {
		return
	}
	st := &feedbackCaptureState{botID: botID, chatID: chatID, messageID: message.ID, original: model.MessageText(message), edited: model.MessageText(message), kind: "accepted", eventID: model.NewMessageID(), loading: true}
	m.present(func(c *ui.Context, s *sheet) { st.view(c, s) }, func() { st.closed = true })
	store.WorkflowFeedback(botID, func(data model.WorkflowFeedback, err error) {
		if st.closed {
			return
		}
		st.loading = false
		if err != nil {
			st.err = model.ErrorText(err)
			return
		}
		st.targets = data.Targets
	})
}
func feedbackKindTitle(kind string) string {
	switch kind {
	case "accepted":
		return L("Accepted")
	case "rejected":
		return L("Rejected")
	case "edited":
		return L("User edited")
	case "routine_failure":
		return L("Routine failure")
	case "ignored_alert":
		return L("Ignored alert · neutral")
	default:
		return L("Explicit feedback")
	}
}
func (st *feedbackCaptureState) view(c *ui.Context, s *sheet) {
	result := sheetFrame(c, sheetOptions{Title: L("Record workflow feedback"), Subtitle: L("Record your decision or correction with a link to this work. Ignored alerts stay neutral."), Width: 580, Confirm: L("Record"), ConfirmDisabled: st.saving, ReturnInContent: true}, func() {
		p := colors(c)
		ui.Row(c).Gap(12).Children(func() {
			ui.Text(c, L("Feedback kind")).FontSize(12).Width(110)
			var options []popUpOption
			for _, kind := range []string{"accepted", "rejected", "edited", "explicit", "ignored_alert"} {
				options = append(options, popUpOption{Value: kind, Label: feedbackKindTitle(kind)})
			}
			if picked, changed, _ := popUpButton(c.Key("feedback-kind"), popUp{Value: st.kind, Options: options, Style: popUpBordered, Disabled: st.saving, Label: L("Feedback kind"), Width: 310}); changed {
				st.kind = picked
			}
		})
		ui.Row(c).Gap(12).Children(func() {
			ui.Text(c, L("Workflow")).FontSize(12).Width(110)
			options := []popUpOption{{Value: "", Label: L("This work")}}
			for i, target := range st.targets {
				options = append(options, popUpOption{Value: strconv.Itoa(i + 1), Label: target.Name})
			}
			if picked, changed, _ := popUpButton(c.Key("feedback-target"), popUp{Value: st.targetKey, Options: options, Style: popUpBordered, Disabled: st.saving || st.loading, Label: L("Workflow"), Width: 310}); changed {
				st.targetKey = picked
			}
		})
		textArea(c.Key("feedback-note"), &st.note, 3, fieldOptions{Label: L("Your feedback or explanation"), Placeholder: L("Your feedback or explanation"), Disabled: st.saving})
		if st.kind == "edited" {
			ui.Text(c, L("For user edits, put the corrected draft below:")).FontSize(12).TextColor(p.Label2)
			textArea(c.Key("feedback-edit"), &st.edited, 0, fieldOptions{Mono: true, Label: L("Corrected draft"), Disabled: st.saving}).Height(160)
		}
		ui.Checkbox(c.Key("feedback-excluded"), &st.excluded, L("Exclude this material from feedback processing")).Disabled(st.saving)
		if st.loading {
			ui.Text(c, L("Loading…")).FontSize(textCaption).TextColor(p.Label2)
		}
		if st.err != "" {
			ui.Text(c, st.err).FontSize(12).TextColor(p.Red).LineHeight(1.4)
		}
	})
	if result.Cancelled {
		s.dismiss()
		return
	}
	if result.Confirmed && !st.saving {
		if st.kind == "explicit" && strings.TrimSpace(st.note) == "" && !st.excluded {
			st.err = L("Write your feedback before recording it.")
			return
		}
		input := model.FeedbackInput{Kind: st.kind, Origin: model.FeedbackOrigin{ChatID: st.chatID, MessageID: st.messageID}, Note: st.note, Excluded: st.excluded, EventID: st.eventID}
		if index, err := strconv.Atoi(st.targetKey); err == nil && index > 0 && index <= len(st.targets) {
			target := st.targets[index-1].Target
			input.Target = &target
		}
		if st.kind == "edited" {
			before, after := st.original, st.edited
			input.Before = &before
			input.After = &after
		}
		input.EventID = ""
		encoded, _ := json.Marshal(input)
		if st.lastPayload != "" && st.lastPayload != string(encoded) {
			st.eventID = model.NewMessageID()
		}
		st.lastPayload = string(encoded)
		input.EventID = st.eventID
		st.saving = true
		st.err = ""
		store.RecordFeedback(st.botID, input, func(err error) {
			if st.closed {
				return
			}
			st.saving = false
			if err != nil {
				st.err = model.ErrorText(err)
				return
			}
			s.dismiss()
		})
	}
}

type feedbackReviewState struct {
	m                     *mainWindow
	botID, chatID         string
	data                  model.WorkflowFeedback
	loading, busy, closed bool
	err                   string
	version               uint64
}

func (m *mainWindow) presentWorkflowFeedback(botID, chatID string) {
	if store.Bot(botID) == nil {
		return
	}
	st := &feedbackReviewState{m: m, botID: botID, chatID: chatID}
	m.present(func(c *ui.Context, s *sheet) { st.view(c, s) }, func() { st.closed = true })
	st.refresh()
}
func (st *feedbackReviewState) refresh() {
	if st.closed || st.loading || st.busy {
		return
	}
	st.loading = true
	st.err = ""
	st.version = store.WorkflowFeedbackVersions[st.botID]
	store.WorkflowFeedback(st.botID, func(data model.WorkflowFeedback, err error) {
		if st.closed {
			return
		}
		st.loading = false
		if err != nil {
			st.err = model.ErrorText(err)
			return
		}
		st.data = data
	})
}
func (st *feedbackReviewState) changed(err error) {
	if st.closed {
		return
	}
	st.busy = false
	if err != nil {
		st.err = model.ErrorText(err)
		return
	}
	st.refresh()
}
func (st *feedbackReviewState) view(c *ui.Context, s *sheet) {
	if st.version != store.WorkflowFeedbackVersions[st.botID] && !st.busy && !st.loading {
		st.refresh()
	}
	name := st.botID
	if bot := store.Bot(st.botID); bot != nil {
		name = bot.Name
	}
	result := sheetFrame(c, sheetOptions{Title: L("Workflow feedback"), Subtitle: L("Review specific improvements to %@'s routines and skills. Every change shows its evidence and diff before you accept it.", name), Width: 720, Confirm: L("Done"), NoCancel: true}, func() {
		p := colors(c)
		disabled := st.busy || st.loading
		ui.Row(c).Gap(12).Children(func() {
			ui.Text(c, L("Periodic review")).FontSize(12).FontWeight(600)
			current := "off"
			options := []popUpOption{{Value: "off", Label: L("Off")}, {Value: "86400", Label: L("Daily")}, {Value: "604800", Label: L("Weekly")}}
			if interval := st.data.Settings.ReviewEverySecs; interval != nil {
				current = strconv.FormatInt(*interval, 10)
				if *interval != 86400 && *interval != 604800 {
					options = append(options, popUpOption{Value: current, Label: L("Every %d days", *interval/86400)})
				}
			}
			if picked, changed, _ := popUpButton(c.Key("feedback-periodic"), popUp{Value: current, Options: options, Style: popUpBordered, Disabled: disabled, Label: L("Periodic review")}); changed {
				var interval *int64
				if picked != "off" {
					value, _ := strconv.ParseInt(picked, 10, 64)
					interval = &value
				}
				st.busy = true
				store.SetFeedbackReview(st.botID, interval, st.changed)
			}
		})
		ui.Text(c, L("Reviews stay quiet when there is no useful proposal. Ignored alerts are neutral; silence does not imply a preference.")).FontSize(12).TextColor(p.Label2).LineHeight(1.4)
		ui.Row(c).Gap(8).Children(func() {
			if pushButton(c.Key("feedback-review-now"), L("Review now"), pushOptions{Disabled: disabled}).Clicked() {
				st.busy = true
				st.err = ""
				store.ReviewFeedback(st.botID, st.changed)
			}
			if pushButton(c.Key("feedback-refresh"), L("Refresh"), pushOptions{Disabled: disabled}).Clicked() {
				st.refresh()
			}
			excluded := slices.Contains(st.data.Settings.ExcludedChats, st.chatID)
			title := L("Exclude this chat")
			if excluded {
				title = L("This chat is excluded")
			}
			if pushButton(c.Key("feedback-exclude-chat"), title, pushOptions{Disabled: disabled || excluded}).Clicked() {
				st.busy = true
				store.ExcludeFeedback(st.botID, model.FeedbackExclusion{ChatID: st.chatID}, st.changed)
			}
		})
		if st.loading {
			ui.Text(c, L("Loading…")).FontSize(12).TextColor(p.Label2)
		}
		if st.busy {
			ui.Text(c, L("Working…")).FontSize(12).TextColor(p.Label2)
		}
		if st.err != "" {
			ui.Text(c, st.err).FontSize(12).TextColor(p.Red).LineHeight(1.4)
		}
		st.proposals(c, s)
		st.examples(c, s)
		st.revisions(c)
	})
	if result.Confirmed || result.Cancelled {
		s.dismiss()
	}
}
func feedbackCode(c *ui.Context, text string, height float32) {
	p := colors(c)
	ui.ScrollBoth(c).Height(height).Padding(10).Background(p.Code).Radius(6).Children(func() { ui.Text(c, text).Font(monoFont).FontSize(12).LineHeight(1.45).NoWrap().Selectable() })
}
func (st *feedbackReviewState) targetName(target model.FeedbackTarget) string {
	switch target.Kind {
	case "plugin_skill":
		return L("Skill · %@ / %@", target.PluginID, target.Name)
	case "playbook":
		return L("Playbook · %@", target.ID)
	default:
		name := target.ID
		if routine := store.Routine(target.ID); routine != nil {
			name = routine.Name
		}
		return L("Routine · %@", name)
	}
}
func (st *feedbackReviewState) origin(c *ui.Context, s *sheet, origin model.FeedbackOrigin) {
	if pushButton(c.Key(origin.ChatID+":"+origin.MessageID), L("Open originating work"), pushOptions{Small: true, Disabled: st.busy || st.loading, Tooltip: origin.ChatID + " / " + origin.MessageID}).Clicked() {
		st.busy = true
		st.err = ""
		store.LoadFeedbackOrigin(origin, func(err error) {
			if st.closed {
				return
			}
			st.busy = false
			if err != nil {
				st.err = model.ErrorText(err)
				return
			}
			s.dismiss()
			st.m.open(origin.ChatID)
			source := st.m.chatStateFor(origin.ChatID)
			source.rows = buildRows(store.Chat(origin.ChatID), store.WorkingBots(origin.ChatID), source.stoppedNotice)
			st.m.revealMessage(st.m.chat, origin.MessageID)
		})
	}
}
func (st *feedbackReviewState) proposals(c *ui.Context, s *sheet) {
	p := colors(c)
	ui.Text(c, L("Proposed improvements")).FontSize(13).FontWeight(600)
	pending := 0
	for _, proposal := range st.data.Proposals {
		if proposal.State != "pending" {
			continue
		}
		pending++
		ui.Column(c.Key("proposal:"+proposal.ID)).Gap(10).Padding(12).Radius(8).Border(1, p.FieldBorder).Children(func() {
			ui.Text(c, st.targetName(proposal.Target)).FontSize(13).FontWeight(600)
			ui.Text(c, proposal.Explanation).FontSize(12).LineHeight(1.4)
			for _, origin := range proposal.Origins {
				st.origin(c, s, origin)
			}
			feedbackCode(c.Key("proposal-diff"), proposal.Diff, 145)
			ui.Row(c).Gap(8).Children(func() {
				disabled := st.busy || st.loading || proposal.Diff == "" || proposal.DiffHash == ""
				if pushButton(c.Key("accept"), L("Accept revision"), pushOptions{Disabled: disabled}).Clicked() {
					st.busy = true
					st.err = ""
					store.DecideFeedback(st.botID, proposal, true, st.changed)
				}
				if pushButton(c.Key("reject"), L("Reject"), pushOptions{Disabled: disabled}).Clicked() {
					st.busy = true
					st.err = ""
					store.DecideFeedback(st.botID, proposal, false, st.changed)
				}
				if pushButton(c.Key("exclude-workflow"), L("Exclude this workflow"), pushOptions{Disabled: st.busy || st.loading}).Clicked() {
					st.busy = true
					target := proposal.Target
					store.ExcludeFeedback(st.botID, model.FeedbackExclusion{Target: &target}, st.changed)
				}
			})
		})
	}
	if pending == 0 {
		ui.Text(c, L("No improvements waiting for review.")).FontSize(12).TextColor(p.Label2)
	}
}
func (st *feedbackReviewState) examples(c *ui.Context, s *sheet) {
	p := colors(c)
	ui.Text(c, L("Feedback examples")).FontSize(13).FontWeight(600)
	if len(st.data.Feedback) == 0 {
		ui.Text(c, L("Use Record workflow feedback… on a message to record acceptance, rejection, edits, or an explicit request.")).FontSize(12).TextColor(p.Label2).LineHeight(1.4)
	}
	for i := len(st.data.Feedback) - 1; i >= max(0, len(st.data.Feedback)-20); i-- {
		f := st.data.Feedback[i]
		ui.Column(c.Key("example:" + f.ID)).Gap(8).Children(func() {
			title := feedbackKindTitle(f.Kind)
			if f.Excluded {
				title = L("%@ · excluded", title)
			}
			ui.Text(c, title).FontSize(12).FontWeight(600)
			if !f.Excluded {
				ui.Text(c, f.Note+"\n"+f.Example).FontSize(12).LineHeight(1.4).Selectable()
				if f.Before != nil && f.After != nil {
					feedbackCode(c.Key("edit-comparison"), L("Original: %@\nEdited: %@", *f.Before, *f.After), 100)
				}
			}
			ui.Row(c).Gap(8).Children(func() {
				st.origin(c, s, f.Origin)
				if !f.Excluded && pushButton(c.Key("exclude-example"), L("Exclude"), pushOptions{Small: true, Disabled: st.busy || st.loading}).Clicked() {
					st.busy = true
					store.ExcludeFeedback(st.botID, model.FeedbackExclusion{ID: f.ID}, st.changed)
				}
			})
		})
	}
}
func (st *feedbackReviewState) revisions(c *ui.Context) {
	p := colors(c)
	ui.Text(c, L("Revision history")).FontSize(13).FontWeight(600)
	for i := len(st.data.Revisions) - 1; i >= max(0, len(st.data.Revisions)-10); i-- {
		r := st.data.Revisions[i]
		ui.Column(c.Key("revision:" + r.ID)).Gap(8).Children(func() {
			ui.Text(c, L("Version %d · %@", r.Version, r.State)).FontSize(12).FontWeight(600)
			if r.State == "applied" && r.CanRollback && r.CurrentHash != "" && r.RollbackDiff != "" {
				feedbackCode(c.Key("rollback-diff"), r.RollbackDiff, 120)
				if pushButton(c.Key("rollback"), L("Roll back this revision"), pushOptions{Disabled: st.busy || st.loading}).Clicked() {
					st.busy = true
					st.err = ""
					store.RollbackFeedback(st.botID, r, st.changed)
				}
			} else if r.State == "applied" {
				ui.Text(c, L("Later changes prevent this rollback. Refresh to review the current version.")).FontSize(12).TextColor(p.Label2).LineHeight(1.4)
			}
		})
	}
}
