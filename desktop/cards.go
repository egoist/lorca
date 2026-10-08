package main

import (
	"net/url"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// Cards in the transcript, after the macOS app's PermissionCellView and CommandCellView: a bot
// asking before a plugin action, an install, a sign-in, or a shell command runs, and a command the
// bot handed over, with its last lines, an answer field, and Stop. In a group a card sits in the
// bubbles' column with the bot's avatar beside its bottom edge.

// presentCommandSheet shows the whole command a card asks about, to read or copy before answering.
func (w *appWindow) presentCommandSheet(title, command string) {
	w.present(func(c *ui.Context, s *sheet) {
		p := colors(c)
		result := sheetFrame(c, sheetOptions{
			Title:    title,
			Width:    600,
			Confirm:  L("Done"),
			NoCancel: true,
			Leading: func() {
				copyButton(c, command, copyOptions{Title: L("Copy"), Bordered: true})
			},
		}, func() {
			ui.Scroll(c).MaxHeight(360).Padding(10, 12).Radius(8).Background(p.Code).Children(func() {
				ui.Text(c, command).Font(monoFont).FontSize(12).LineHeight(1.45).Selectable()
			})
		})
		if result.Confirmed || result.Cancelled {
			s.dismiss()
		}
	}, nil)
}

// commandBlock is a shell command's first lines in a code block that shows the whole command on
// click, highlighting under the pointer like a button. It reports the click.
func commandBlock(c *ui.Context, text string, lines int) bool {
	p := colors(c)
	b := ui.ButtonBase(c).FillWidth().Margin(6, 0, 0, 0).Padding(5, 8).Radius(6).Background(p.Code).TextColor(p.Label).
		Justify(ui.Start).Cursor(ui.CursorPointer).Label(L("Show the full command")).Tooltip(L("Show the full command"))
	if b.Hovered() {
		b.Background(p.CodeHover)
	}
	b.Children(func() {
		ui.Text(c, text).Font(monoFont).FontSize(11).FixedLineHeight(15).MaxLines(lines).Shrink(1).MinWidth(0)
	})
	return b.Clicked()
}

// commandOutput is a command's last lines: a code block that grows to `lines` lines and then
// scrolls, the newest line in view.
func commandOutput(c *ui.Context, text string, lines int) ui.Element {
	p := colors(c)
	box := ui.Scroll(c).Margin(6, 0, 0, 0).MaxHeight(float32(lines)*15+10).Padding(5, 8).Radius(6).Background(p.Code).TextColor(p.Label)
	state := ui.Local(box, "scroll", func() outputScroll { return outputScroll{} })
	if state.text != text {
		// New output: the newest line comes into view.
		state.text = text
		state.scroll.Y = 1 << 30
	}
	box.TrackScroll(&state.scroll)
	box.Children(func() {
		ui.Text(c, text).Font(monoFont).FontSize(11).FixedLineHeight(15).Selectable()
	})
	return box
}

type outputScroll struct {
	text   string
	scroll ui.ScrollState
}

// MARK: - The permission card

func hostOf(link string) string {
	u, err := url.Parse(link)
	if err != nil || u.Host == "" {
		return ""
	}
	return u.Host
}

// permissionSummary is the line under the title: the call while it waits, the answer and the call
// once answered, or where to enter a sign-in code.
func permissionSummary(request *model.PermissionRequest) string {
	if request.IsPending() {
		return request.ShownSummary()
	}
	if request.Decision == model.DecisionAllowed && request.Code != "" {
		return L("Enter this code at %@, then come back.", firstNonEmpty(hostOf(request.Link), L("the link")))
	}
	return request.DecisionText() + " · " + request.ShownSummary()
}

// ruleNote is what a card says about its rule: the one Always allow would add, or the one it added.
func ruleNote(request *model.PermissionRequest) string {
	if !request.HasRule {
		return ""
	}
	if request.IsPending() {
		return L("Always allow adds the rule “%@”.", request.Rule)
	}
	if request.Decision == model.DecisionAlways {
		return L("Added the rule “%@” to Auto-review.", request.Rule)
	}
	return ""
}

// cardFrame is a card's row: indented as a bubble is, the bot's avatar at its bottom edge in a
// group.
func cardFrame(c *ui.Context, cardAvatar *avatarContent, groupStart bool, label string, content func()) {
	p := colors(c)
	top := float32(groupTopPadding)
	if !groupStart {
		top = tightTopPadding
	}
	left := float32(horizontalInset)
	if cardAvatar != nil {
		left = bubbleIndent
	}
	ui.Row(c).Padding(top, horizontalInset, 0, left).Children(func() {
		ui.Row(c).AlignItems(ui.Start).Gap(8).Width(440).MaxWidthPercent(100).MinWidth(0).Padding(11, 12, 12, 12).Radius(12).
			Background(p.BotBubble).Border(1, p.BotBubbleBorder).Label(label).Children(content)
		if cardAvatar != nil {
			ui.Box(c).Absolute().Left(horizontalInset).Bottom(0).Children(func() {
				avatar(c, *cardAvatar, chatAvatarSize, false)
			})
		}
	})
}

func (m *mainWindow) permissionCard(c *ui.Context, chat *model.Chat, message *model.Message, cardAvatar *avatarContent, groupStart bool) {
	p := colors(c)
	request := message.Body.Request
	name := botName(message)
	icon := "hand.raised"
	switch {
	case request.IsConnect():
		icon = "person.crop.circle.badge.checkmark"
	case request.IsInstall():
		icon = "puzzlepiece.extension"
	}
	tint := p.Accent
	switch request.Decision {
	case model.DecisionFailed:
		tint = p.Red
	case model.DecisionConnected:
		tint = p.Green
	}
	title := name + " " + request.VerbPhrase()
	chatID, messageID := chat.ID, message.ID
	cardFrame(c, cardAvatar, groupStart, title+": "+permissionSummary(request), func() {
		ui.Row(c).Size(18, 18).Center().TextColor(tint).Children(func() { symbol(c, icon, 17, 1.9) })
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
			ui.Text(c, title).FontSize(12.5).FontWeight(600).FixedLineHeight(17).SingleLine().Tooltip(title)
			if request.IsPending() && request.IsShell() {
				if commandBlock(c, request.FullCommand(), 2) {
					m.presentCommandSheet(title, request.FullCommand())
				}
			} else {
				lines := 3
				if request.IsPending() {
					lines = 2
				}
				ui.Text(c, permissionSummary(request)).Margin(2, 0, 0, 0).FontSize(textCaption).LineHeight(1.35).TextColor(p.Label2).MaxLines(lines).Tooltip(request.ShownSummary())
			}
			if request.Decision == model.DecisionAllowed && request.Code != "" {
				ui.Row(c).Gap(10).Margin(10, 0, 0, 0).Children(func() {
					ui.Text(c, request.Code).Font(monoFont).FontSize(15).FontWeight(600).LetterSpacing(0.6).Selectable()
					open := copyButton(c, request.Code, copyOptions{Title: L("Copy code and open %@", firstNonEmpty(hostOf(request.Link), L("link"))), Bordered: true})
					if open.Clicked() && request.Link != "" {
						_ = openExternal(request.Link)
					}
				})
			}
			if reason := request.ShownReason(); request.IsPending() && reason != "" {
				ui.Text(c, reason).Margin(7, 0, 0, 0).FontSize(11.5).LineHeight(1.35).TextColor(p.Label2)
			}
			if request.IsPending() {
				ui.Row(c).Wrap().Gap(6).Margin(10, 0, 0, 0).Children(func() {
					for _, choice := range request.Choices() {
						if pushButton(c.Key(choice.Decision), choice.Title, pushOptions{Small: true}).Clicked() {
							if choice.Decision == "access" && message.Author.Kind == model.AuthorBot {
								m.presentBotAccess(message.Author.BotID)
							} else {
								store.AnswerPermission(chatID, messageID, choice.Decision)
							}
						}
					}
				})
			}
			if note := ruleNote(request); note != "" {
				ui.Text(c, note).Margin(8, 0, 0, 0).FontSize(11).LineHeight(1.35).TextColor(p.Label2)
			}
		})
	})
}

// MARK: - The command card

// commandTitle is "Chef wants to run a command on Workbench", "Chef's command is running", or
// "Chef's command is waiting for input".
func commandTitle(run *model.CommandRun, botName string) string {
	switch run.State {
	case model.CommandWaiting:
		return L("%@'s command is waiting for input", botName)
	case model.CommandRunning:
		return L("%@'s command is running", botName)
	}
	if run.Device != "" {
		return botName + " " + L("wants to run a command on %@", run.Device)
	}
	return L("%@'s command", botName)
}

// outputText is the terminal's last lines with something on them.
func outputText(output string) string {
	var lines []string
	for _, line := range strings.Split(output, "\n") {
		if line != "" {
			lines = append(lines, line)
		}
	}
	return strings.Join(lines, "\n")
}

func commandNote(run *model.CommandRun) string {
	switch {
	case run.State == model.CommandAsking && run.HasRule:
		return L("Always allow adds the rule “%@”.", run.Rule)
	case run.State == model.CommandWaiting && run.SessionID != "":
		return L("What you type goes straight to the command, not into the chat.")
	}
	return ""
}

func commandSpokenText(run *model.CommandRun, botName string) string {
	parts := []string{commandTitle(run, botName), "$ " + model.FirstLine(run.Command)}
	switch {
	case run.IsLive():
		if text := outputText(run.Output); text != "" {
			parts = append(parts, text)
		}
	case run.State == model.CommandAsking && run.Reason != "":
		parts = append(parts, run.Reason)
	}
	return strings.Join(parts, ": ")
}

// commandCardState is a command card's own: the answer being typed, a request under way, and
// what went wrong, until the command's state or output changes.
type commandCardState struct {
	answer   string
	busy     bool
	err      string
	seen     model.CommandState
	seenText string
}

func (m *mainWindow) commandCard(c *ui.Context, chat *model.Chat, message *model.Message, cardAvatar *avatarContent, groupStart bool) {
	p := colors(c)
	run := message.Body.Tool.Run
	name := botName(message)
	chatID, messageID := chat.ID, message.ID
	holder := ui.Box(c.Key("card:" + message.ID))
	st := ui.Local(holder, "card", func() commandCardState { return commandCardState{seen: run.State, seenText: run.Output} })
	// Another state or new output clears what went wrong; ending takes back what was typed.
	if st.seen != run.State || st.seenText != run.Output {
		st.seen, st.seenText, st.err = run.State, run.Output, ""
		if !run.TakesInput() {
			st.answer = ""
		}
	}
	answering := run.State == model.CommandWaiting && run.SessionID != ""
	send := func() {
		if st.busy || !run.TakesInput() {
			return
		}
		st.busy, st.err = true, ""
		text := st.answer
		store.AnswerCommand(chatID, messageID, text, func(err error) {
			st.busy = false
			if err != nil {
				st.err = model.ErrorText(err)
			} else {
				st.answer = ""
			}
		})
	}
	stop := func() {
		if st.busy {
			return
		}
		st.busy = true
		store.StopCommand(chatID, messageID, func(err error) {
			st.busy = false
			if err == nil {
				return
			}
			// Under the answer field when there is one, else in an alert.
			if commandNote(run) != "" {
				st.err = model.ErrorText(err)
			} else {
				m.showAlert(alertOptions{Message: L("Could not stop the command"), Informative: model.ErrorText(err)}, nil)
			}
		})
	}
	holder.Children(func() {
		cardFrame(c, cardAvatar, groupStart, commandSpokenText(run, name), func() {
			ui.Row(c).Size(18, 18).Center().TextColor(p.Accent).Children(func() { symbol(c, "terminal", 17, 1.9) })
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
				ui.Row(c).Gap(8).MinWidth(0).Children(func() {
					ui.Text(c, commandTitle(run, name)).Grow(1).Shrink(1).MinWidth(0).FontSize(12.5).FontWeight(600).FixedLineHeight(17).SingleLine().Tooltip(run.Command)
					if run.TakesInput() && pushButton(c, L("Stop"), pushOptions{Small: true, Disabled: st.busy}).Clicked() {
						stop()
					}
				})
				if commandBlock(c, "$ "+model.FirstLine(run.Command), 1) {
					m.presentCommandSheet(L("%@'s command", name), run.Command)
				}
				if run.State == model.CommandAsking && run.Reason != "" {
					ui.Text(c, run.Reason).Margin(7, 0, 0, 0).FontSize(11.5).LineHeight(1.35).TextColor(p.Label2).MaxLines(4)
				}
				if text := outputText(run.Output); run.IsLive() && text != "" {
					commandOutput(c, text, 6)
				}
				if run.State == model.CommandAsking {
					ui.Row(c).Wrap().Gap(6).Margin(10, 0, 0, 0).Children(func() {
						for _, choice := range run.Choices() {
							if pushButton(c.Key(choice.Decision), choice.Title, pushOptions{Small: true}).Clicked() {
								store.AnswerPermission(chatID, messageID, choice.Decision)
							}
						}
					})
				}
				if answering {
					ui.Row(c).Gap(6).Margin(10, 0, 0, 0).Children(func() {
						field := textField(c, &st.answer, fieldOptions{
							Secure:      !run.AsksYesOrNo(),
							Placeholder: L("Type your answer"),
							Label:       firstNonEmpty(run.Prompt, L("Type your answer")),
							Disabled:    st.busy,
						}).Grow(1).Shrink(1).MinWidth(0)
						if field.Submitted() {
							send()
						}
						if pushButton(c, L("Send"), pushOptions{Small: true, Disabled: st.busy}).Clicked() {
							send()
						}
					})
				}
				note := firstNonEmpty(st.err, commandNote(run))
				if note != "" {
					t := ui.Text(c, note).Margin(8, 0, 0, 0).FontSize(11).LineHeight(1.35).TextColor(p.Label2)
					if st.err != "" {
						t.TextColor(p.Red).Tooltip(st.err)
					}
				}
			})
		})
	})
}

// beep is the system's alert sound, for a command that cannot run.
func beep() { go mygo.Shell.Beep() }
