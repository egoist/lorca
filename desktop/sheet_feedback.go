package main

import (
	"strconv"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Workflow feedback, after the macOS app's FeedbackViewController: what the user says about a
// bot's messages, the changes to its routines and skills the bot suggests from it, and the changes
// the user accepted. The bot's Runner keeps all of it and applies every decision; these sheets
// only ask. A sheet's fields live in its state, which a closed sheet keeps from late replies.

func feedbackKindSymbol(kind string) string {
	switch kind {
	case model.FeedbackAccepted:
		return "checkmark.circle"
	case model.FeedbackRejected:
		return "xmark.circle"
	case model.FeedbackEdited:
		return "pencil"
	case model.FeedbackRoutineFailure:
		return "exclamationmark.triangle"
	case model.FeedbackIgnoredAlert:
		return "eye.slash"
	default:
		return "bubble.left"
	}
}

func feedbackKindTitle(kind string) string {
	switch kind {
	case model.FeedbackAccepted:
		return L("Good")
	case model.FeedbackRejected:
		return L("Not right")
	case model.FeedbackEdited:
		return L("Corrected")
	case model.FeedbackRoutineFailure:
		return L("Run failed")
	case model.FeedbackIgnoredAlert:
		return L("Ignored")
	default:
		return L("Note")
	}
}

// feedbackProblem is what the Runner refused, in the app's terms.
func feedbackProblem(err error, name string) string {
	message := model.ErrorText(err)
	switch {
	case strings.Contains(message, "review a fresh proposal"), strings.Contains(message, "cannot overwrite later edits"):
		return L("“%@” changed after this, so the change can't be made.", name)
	case strings.Contains(message, "displayed diff changed"), strings.Contains(message, "no longer pending"):
		return L("This suggestion changed on another Device.")
	case strings.Contains(message, "no longer included"):
		return L("This suggestion is based on feedback you excluded.")
	}
	return message
}

func (m *mainWindow) feedbackAlert(title, text string) {
	m.showAlert(alertOptions{Message: title, Informative: text, Style: alertWarning}, nil)
}

// feedbackRow is a note, a suggestion, or a change as a row: a symbol, a title over a detail line.
// It answers a click, which opens what it is about, and fills on hover, as a routine's row does.
func feedbackRow(c *ui.Context, k *card, symbolName, title, detail, tooltip string) ui.Element {
	p := colors(c)
	r := k.row(rowBox(c).MinHeight(44).Label(title).Cursor(ui.CursorPointer))
	if tooltip != "" {
		r.Tooltip(tooltip)
	}
	if r.Hovered() {
		r.Background(p.RowHover)
	}
	r.Children(func() {
		ui.Row(c).Width(18).Justify(ui.Center).TextColor(p.Label2).Children(func() { symbol(c, symbolName, 15, 1.7) })
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			ui.Text(c, title).FontSize(12.5).FontWeight(500).SingleLine()
			ui.Text(c, detail).FontSize(textCaption).TextColor(p.Label2).SingleLine()
		})
	})
	return r
}

// feedbackNoteRow is a note: its words, or the routine a failed run belongs to, over its kind
// and when.
func feedbackNoteRow(c *ui.Context, k *card, f model.BotFeedback, note model.FeedbackNote) ui.Element {
	title := note.Text
	if note.Kind == model.FeedbackRoutineFailure && note.Target != nil {
		title = store.FeedbackTargetName(f, *note.Target)
	}
	if title == "" {
		title = feedbackKindTitle(note.Kind)
	}
	return feedbackRow(c.Key(note.ID), k, feedbackKindSymbol(note.Kind), title, feedbackKindTitle(note.Kind)+" · "+model.Stamp(note.CreatedAt), note.Text)
}

// feedbackSuggestionRow is a suggestion: what it changes; why is its tooltip and in its sheet.
func feedbackSuggestionRow(c *ui.Context, k *card, f model.BotFeedback, suggestion model.FeedbackSuggestion) ui.Element {
	return feedbackRow(c.Key(suggestion.ID), k, "sparkles", store.FeedbackTargetName(f, suggestion.Target), L("Suggested change"), suggestion.Explanation)
}

// feedbackDiff is the changed lines of a suggestion or a change: what goes on red, what comes in
// on green, as a document's tracked changes read. Long changes scroll.
func feedbackDiff(c *ui.Context, k *card, diff string) {
	p := colors(c)
	k.row(ui.Scroll(c).MaxHeight(240).Children(func() {
		ui.Column(c).Children(func() {
			for i, line := range model.DiffLines(diff) {
				mark, tint := "+", p.Green
				if line.Removed {
					mark, tint = "−", p.Red
				}
				text := line.Text
				if text == "" {
					text = " "
				}
				ui.Row(c.Key(i)).Gap(8).Padding(5, 12).AlignItems(ui.Start).Background(tint.Alpha(0.13)).Children(func() {
					ui.Text(c, mark).Width(12).Shrink(0).Font(monoFont).FontSize(11).FontWeight(500).TextColor(tint).LineHeight(1.45)
					ui.Text(c, text).Grow(1).Shrink(1).MinWidth(0).FontSize(12).LineHeight(1.4).Selectable()
				})
			}
		})
	}))
}

// showFeedbackMessage opens a chat on the message a note is about, loading older pages until it
// is there. One that is gone beeps, as a quote's does.
func (m *mainWindow) showFeedbackMessage(chatID, messageID string) {
	store.LoadMessage(chatID, messageID, func(err error) {
		if err != nil {
			beep()
			return
		}
		m.open(chatID)
		chat := m.chatStateFor(chatID)
		chat.rows = buildRows(store.Chat(chatID), store.WorkingBots(chatID), chat.stoppedNotice)
		m.revealMessage(chat, messageID)
	})
}

// MARK: - Giving feedback

type feedbackFormState struct {
	botID, chatID, messageID, original string
	kind                               string
	corrected, note, target            string
	targets                            []model.FeedbackTargetName
	saving, closed                     bool
}

var feedbackKinds = []string{model.FeedbackAccepted, model.FeedbackRejected, model.FeedbackEdited, model.FeedbackExplicit}

func feedbackKindChoice(kind string) string {
	switch kind {
	case model.FeedbackAccepted:
		return L("Looks good")
	case model.FeedbackRejected:
		return L("Not what I wanted")
	case model.FeedbackEdited:
		return L("I corrected it")
	default:
		return L("A note for next time")
	}
}

// presentFeedback is feedback on one of a bot's messages: whether it was right, a corrected
// version, or a note for next time, and optionally which routine or skill it is about. Opened
// from the message's menu.
func (m *mainWindow) presentFeedback(chatID string, message *model.Message) {
	if message == nil || message.Author.BotID == "" || store.Bot(message.Author.BotID) == nil {
		return
	}
	text := model.MessageText(message)
	st := &feedbackFormState{botID: message.Author.BotID, chatID: chatID, messageID: message.ID, original: text, corrected: text, kind: model.FeedbackAccepted}
	m.present(func(c *ui.Context, s *sheet) { m.feedbackFormView(c, s, st) }, func() { st.closed = true })
	// The routines and skills the note can be about, as the Runner names them.
	store.Feedback(st.botID, func(f model.BotFeedback, err error) {
		if err == nil && !st.closed {
			st.targets = f.Targets
		}
	})
}

func (st *feedbackFormState) canSave() bool {
	switch st.kind {
	case model.FeedbackExplicit:
		return strings.TrimSpace(st.note) != ""
	case model.FeedbackEdited:
		return st.corrected != st.original
	}
	return true
}

func (m *mainWindow) feedbackFormView(c *ui.Context, s *sheet, st *feedbackFormState) {
	name := ""
	if bot := store.Bot(st.botID); bot != nil {
		name = bot.Name
	}
	result := sheetFrame(c, sheetOptions{
		Title:           L("Feedback"),
		Subtitle:        L("%@ uses it to suggest changes to its routines and skills.", name),
		Width:           460,
		Confirm:         L("Save"),
		ConfirmDisabled: st.saving || !st.canSave(),
		ReturnInContent: st.kind == model.FeedbackEdited,
	}, func() {
		labelWidth := mcpFormLabelWidth(c, L("Response"), L("Your version"), L("Note"), L("Applies to"))
		ui.Column(c).Gap(10).Children(func() {
			providerFormRow(c, labelWidth, L("Response"), func() {
				options := make([]popUpOption, 0, len(feedbackKinds))
				for _, kind := range feedbackKinds {
					options = append(options, popUpOption{Value: kind, Label: feedbackKindChoice(kind)})
				}
				ui.Row(c).Children(func() {
					if picked, changed, _ := popUpButton(c.Key("response"), popUp{Value: st.kind, Options: options, Disabled: st.saving, Label: L("Response")}); changed {
						st.kind = picked
					}
				})
			})
			if st.kind == model.FeedbackEdited {
				providerFormRow(c, labelWidth, L("Your version"), func() {
					textArea(c.Key("version"), &st.corrected, 6, fieldOptions{Label: L("Your version"), Disabled: st.saving})
				}).AlignItems(ui.Start)
			}
			providerFormRow(c, labelWidth, L("Note"), func() {
				placeholder := L("What should it do next time?")
				switch st.kind {
				case model.FeedbackAccepted:
					placeholder = L("What worked? (optional)")
				case model.FeedbackRejected:
					placeholder = L("What was wrong? (optional)")
				case model.FeedbackEdited:
					placeholder = L("What did you change? (optional)")
				}
				submits(textField(c.Key("note"), &st.note, fieldOptions{Placeholder: placeholder, Label: L("Note"), Disabled: st.saving, AutoFocus: true}))
			})
			providerFormRow(c, labelWidth, L("Applies to"), func() {
				options := []popUpOption{{Value: "", Label: L("Any")}}
				for i, target := range st.targets {
					options = append(options, popUpOption{Value: strconv.Itoa(i), Label: target.Name})
				}
				ui.Row(c).Children(func() {
					if picked, changed, _ := popUpButton(c.Key("applies"), popUp{Value: st.target, Options: options, Disabled: st.saving || len(st.targets) == 0, Label: L("Applies to")}); changed {
						st.target = picked
					}
				})
			})
		})
	})
	if result.Cancelled {
		s.dismiss()
		return
	}
	if !result.Confirmed || st.saving || !st.canSave() {
		return
	}
	// What is sent is copied now; the fields may change while the Runner answers.
	input := model.FeedbackInput{Kind: st.kind, ChatID: st.chatID, MessageID: st.messageID, Note: strings.TrimSpace(st.note), Before: st.original, After: st.corrected}
	if index, err := strconv.Atoi(st.target); err == nil && index < len(st.targets) {
		target := st.targets[index].Target
		input.Target = &target
	}
	st.saving = true
	store.RecordFeedback(st.botID, input, func(err error) {
		if st.closed {
			return
		}
		st.saving = false
		if err != nil {
			m.feedbackAlert(L("Couldn't save your feedback"), model.ErrorText(err))
			return
		}
		s.dismiss()
	})
}

// MARK: - A bot's feedback

type feedbackListState struct {
	botID                 string
	looking, foundNothing bool
	closed                bool
}

// presentFeedbackList is everything a bot keeps from the user's feedback: the changes it suggests,
// how often it looks for them, the changes accepted, and the notes. Opened from the inspector; it
// shows what the inspector last heard from the Runner.
func (m *mainWindow) presentFeedbackList(botID string) {
	st := &feedbackListState{botID: botID}
	m.present(func(c *ui.Context, s *sheet) { m.feedbackListView(c, s, st) }, func() { st.closed = true })
}

func (m *mainWindow) feedbackListView(c *ui.Context, s *sheet, st *feedbackListState) {
	bot := store.Bot(st.botID)
	if bot == nil {
		s.dismiss()
		return
	}
	f := m.inspector.feedback[st.botID]
	result := sheetFrame(c, sheetOptions{
		Title:    L("Feedback"),
		Subtitle: L("%@ suggests changes to its routines and skills from your feedback. Nothing changes until you accept one.", bot.Name),
		Width:    480,
		Confirm:  L("Done"),
		NoCancel: true,
		Leading: func() {
			title := L("Look Now")
			if st.looking {
				title = L("Looking…")
			}
			if pushButton(c.Key("look"), title, pushOptions{Disabled: st.looking, Tooltip: L("Look through new feedback for changes to suggest")}).Clicked() {
				st.looking, st.foundNothing = true, false
				store.SuggestChanges(st.botID, func(found int, err error) {
					if st.closed {
						return
					}
					st.looking = false
					if err != nil {
						m.feedbackAlert(L("Couldn't look for changes"), model.ErrorText(err))
						return
					}
					st.foundNothing = found == 0
				})
			}
		},
	}, func() {
		ui.Column(c).Gap(18).Children(func() {
			section(c, L("Suggestions"), sectionCaption, nil, func(k *card) {
				for _, suggestion := range f.Suggestions {
					if feedbackSuggestionRow(c, k, f, suggestion).Clicked() {
						m.presentFeedbackSuggestion(st.botID, f, suggestion, s)
					}
				}
				if st.foundNothing && len(f.Suggestions) == 0 {
					noteRow(c, k, L("Nothing to change right now."), nil)
				}
				current := ""
				if f.ReviewEvery != nil {
					current = strconv.FormatInt(*f.ReviewEvery, 10)
				}
				options := []popUpOption{{Value: "", Label: L("When asked")}, {Value: "86400", Label: L("Daily")}, {Value: "604800", Label: L("Weekly")}}
				if picked, changed := popUpRow(c.Key("interval"), k, L("Look for changes"), popUp{Value: current, Options: options, Label: L("Look for changes")}, false); changed {
					var every *int64
					if seconds, err := strconv.ParseInt(picked, 10, 64); err == nil {
						every = &seconds
					}
					store.SetFeedbackReview(st.botID, every, func(err error) {
						if err != nil && !st.closed {
							m.feedbackAlert(L("Couldn't change it"), model.ErrorText(err))
						}
					})
				}
			})
			if len(f.Changes) > 0 {
				section(c, L("Changes"), sectionCaption, nil, func(k *card) {
					for _, change := range f.Changes {
						symbolName, state := "pencil.line", L("Changed %@", model.Stamp(change.CreatedAt))
						if change.IsUndo {
							symbolName, state = "arrow.uturn.backward", L("Undone %@", model.Stamp(change.CreatedAt))
						}
						if feedbackRow(c.Key(change.ID), k, symbolName, store.FeedbackTargetName(f, change.Target), state, "").Clicked() {
							m.presentFeedbackChange(st.botID, f, change)
						}
					}
				})
			}
			if len(f.Notes) > 0 {
				section(c, Lc("Notes", "feedback"), sectionCaption, nil, func(k *card) {
					for _, note := range f.Notes {
						row := feedbackNoteRow(c, k, f, note)
						if row.Clicked() {
							s.dismiss()
							m.showFeedbackMessage(note.ChatID, note.MessageID)
						}
						row.ContextMenu(func(menu *ui.Menu) {
							if menu.Item(L("Show in Chat")).Chosen() {
								s.dismiss()
								m.showFeedbackMessage(note.ChatID, note.MessageID)
							}
							menu.Separator()
							wholeChat := false
							chosen := menu.Item(L("Exclude")).Chosen()
							if menu.Item(L("Exclude Everything from “%@”", chatTitle(note.ChatID))).Chosen() {
								chosen, wholeChat = true, true
							}
							if chosen {
								store.ExcludeFeedback(st.botID, note, wholeChat, func(err error) {
									if err != nil && !st.closed {
										m.feedbackAlert(L("Couldn't exclude it"), model.ErrorText(err))
									}
								})
							}
						})
					}
				})
			}
		})
	})
	if result.Confirmed || result.Cancelled {
		s.dismiss()
	}
}

func chatTitle(chatID string) string {
	if chat := store.Chat(chatID); chat != nil {
		return store.Title(chat)
	}
	return ""
}

// MARK: - A suggestion or a change

type feedbackItemState struct {
	busy, closed bool
}

func feedbackTargetKind(kind string) string {
	if kind == "playbook" {
		return L("Skill")
	}
	return L("Task")
}

// presentFeedbackSuggestion is one suggestion, with why and the notes it is based on, to accept
// or reject. `under` is the sheet it opened from, which Show in Chat closes too.
func (m *mainWindow) presentFeedbackSuggestion(botID string, f model.BotFeedback, suggestion model.FeedbackSuggestion, under *sheet) {
	st := &feedbackItemState{}
	name := store.FeedbackTargetName(f, suggestion.Target)
	m.present(func(c *ui.Context, s *sheet) {
		decide := func(accept bool) {
			st.busy = true
			store.DecideSuggestion(botID, suggestion, accept, func(err error) {
				if st.closed {
					return
				}
				st.busy = false
				if err != nil {
					title := L("Couldn't reject it")
					if accept {
						title = L("Couldn't make the change")
					}
					m.feedbackAlert(title, feedbackProblem(err, name))
					return
				}
				s.dismiss()
			})
		}
		result := sheetFrame(c, sheetOptions{
			Title:           name,
			Subtitle:        L("Suggested change"),
			Width:           480,
			Confirm:         L("Accept"),
			ConfirmDisabled: st.busy,
			Leading: func() {
				if pushButton(c.Key("reject"), L("Reject"), pushOptions{Disabled: st.busy}).Clicked() {
					decide(false)
				}
			},
		}, func() {
			ui.Text(c, suggestion.Explanation).FontSize(13).LineHeight(1.4).Selectable()
			section(c, feedbackTargetKind(suggestion.Target.Kind), sectionCaption, nil, func(k *card) { feedbackDiff(c, k, suggestion.Diff) })
			var notes []model.FeedbackNote
			for _, id := range suggestion.Evidence {
				if note := f.Note(id); note != nil {
					notes = append(notes, *note)
				}
			}
			if len(notes) > 0 {
				section(c, L("Based on"), sectionCaption, nil, func(k *card) {
					for _, note := range notes {
						if feedbackNoteRow(c, k, f, note).Clicked() {
							s.dismiss()
							if under != nil {
								under.dismiss()
							}
							m.showFeedbackMessage(note.ChatID, note.MessageID)
						}
					}
				})
			}
		})
		if result.Cancelled {
			s.dismiss()
		} else if result.Confirmed && !st.busy {
			decide(true)
		}
	}, func() { st.closed = true })
}

// presentFeedbackChange is one change accepted before, to undo while the routine or skill still
// reads as the change left it.
func (m *mainWindow) presentFeedbackChange(botID string, f model.BotFeedback, change model.FeedbackChange) {
	st := &feedbackItemState{}
	name := store.FeedbackTargetName(f, change.Target)
	subtitle := L("Changed %@", model.DaySeparator(change.CreatedAt))
	if change.IsUndo {
		subtitle = L("Undone %@", model.DaySeparator(change.CreatedAt))
	}
	m.present(func(c *ui.Context, s *sheet) {
		o := sheetOptions{Title: name, Subtitle: subtitle, Width: 480, Confirm: L("Done"), NoCancel: true}
		if change.CanUndo {
			o.Leading = func() {
				if !pushButton(c.Key("undo"), L("Undo Change"), pushOptions{Disabled: st.busy}).Clicked() {
					return
				}
				st.busy = true
				store.UndoChange(botID, change, func(err error) {
					if st.closed {
						return
					}
					st.busy = false
					if err != nil {
						m.feedbackAlert(L("Couldn't undo the change"), feedbackProblem(err, name))
						return
					}
					s.dismiss()
				})
			}
		}
		result := sheetFrame(c, o, func() {
			section(c, feedbackTargetKind(change.Target.Kind), sectionCaption, nil, func(k *card) { feedbackDiff(c, k, change.Diff) })
			if !change.CanUndo {
				caption(c, L("“%@” has changed since, so this can't be undone.", name))
			}
		})
		if result.Confirmed || result.Cancelled {
			s.dismiss()
		}
	}, func() { st.closed = true })
}
