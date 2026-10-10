package main

import (
	"fmt"
	"net/url"
	"slices"
	"strings"
	"unicode/utf8"

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

// MARK: - The secret card

// secretCaption is the line under a secret card's title: why, while it asks; the answer and what
// was asked for, once answered.
func secretCaption(request *model.PermissionRequest) string {
	if request.IsPending() {
		return request.Reason
	}
	return request.DecisionText() + " · " + request.Summary
}

// runnerName is where a bot's message came from: its Runner's name.
func runnerName(message *model.Message) string {
	if bot := store.Bot(message.Author.BotID); bot != nil {
		if device := store.Device(bot.RunnerID); device != nil {
			return device.Name
		}
	}
	return L("its Runner")
}

// secretCardState is a secret card's own: what is typed in each field, a save under way, and why
// one failed, until the card is answered.
type secretCardState struct {
	values []string
	busy   bool
	err    string
}

// secretCard is a bot asking for a secret, after the macOS app's SecretCellView: who asks and where
// it goes, why, a field for each value that hides what is typed, Save and Not now, and a note that
// the value stays on the Runner and the bot never sees it, which a failed save turns into why.
// Once answered, the answer and what was asked for: "Saved · npm token".
func (m *mainWindow) secretCard(c *ui.Context, chat *model.Chat, message *model.Message, cardAvatar *avatarContent, groupStart bool) {
	p := colors(c)
	request := message.Body.Request
	name := botName(message)
	chatID, messageID := chat.ID, message.ID
	fields := request.Secret.Fields
	holder := ui.Box(c.Key("secret:" + message.ID))
	st := ui.Local(holder, "card", func() secretCardState { return secretCardState{} })
	if len(st.values) != len(fields) {
		st.values = make([]string, len(fields))
	}
	filled := len(fields) > 0
	for _, value := range st.values {
		filled = filled && strings.TrimSpace(value) != ""
	}
	save := func() {
		if st.busy || !filled {
			return
		}
		values := map[string]string{}
		for i, field := range fields {
			values[field.Name] = st.values[i]
		}
		st.busy, st.err = true, ""
		store.AnswerSecret(chatID, messageID, values, func(err error) {
			st.busy = false
			if err != nil {
				st.err = model.ErrorText(err)
			}
		})
	}
	title := name + " " + request.VerbPhrase()
	caption := secretCaption(request)
	tint := p.Accent
	if !request.IsPending() {
		tint = p.Label2
	}
	holder.Children(func() {
		cardFrame(c, cardAvatar, groupStart, title+": "+caption, func() {
			ui.Row(c).Size(18, 18).Center().TextColor(tint).Children(func() { symbol(c, "key", 17, 1.9) })
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
				ui.Text(c, title).FontSize(12.5).FontWeight(600).FixedLineHeight(17).SingleLine().Tooltip(title)
				if caption != "" {
					ui.Text(c, caption).Margin(2, 0, 0, 0).FontSize(11.5).LineHeight(1.35).TextColor(p.Label2).MaxLines(4)
				}
				if !request.IsPending() {
					return
				}
				ui.Column(c).Gap(6).Margin(9, 0, 0, 0).Children(func() {
					for i, field := range fields {
						ui.Row(c.Key(field.Name)).Children(func() {
							input := textField(c, &st.values[i], fieldOptions{Secure: true, Placeholder: field.Label, Label: field.Label, Disabled: st.busy}).Grow(1).Shrink(1).MinWidth(0)
							if input.Changed() {
								st.err = ""
							}
							if input.Submitted() {
								save()
							}
						})
					}
				})
				ui.Row(c).Gap(6).Margin(9, 0, 0, 0).Children(func() {
					if pushButton(c.Key("save"), L("Save"), pushOptions{Small: true, Disabled: st.busy || !filled}).Clicked() {
						save()
					}
					if pushButton(c.Key("deny"), L("Not now"), pushOptions{Small: true, Disabled: st.busy}).Clicked() {
						store.AnswerPermission(chatID, messageID, "deny")
					}
				})
				note := ui.Text(c, firstNonEmpty(st.err, L("Saved on %@. %@ never sees it.", runnerName(message), name))).Margin(7, 0, 0, 0).FontSize(11).LineHeight(1.35).TextColor(p.Label2).SingleLine()
				if st.err != "" {
					note.TextColor(p.Red).Tooltip(st.err)
				}
			})
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

// MARK: - The draft card

// An email or Slack message a bot wrote, after the macOS app's DraftCellView: a card the user
// edits in place, then sends or discards. While it waits: who drafted it and the account it goes
// out from, the header rows as Mail's compose window has them (To, Cc, Bcc, Subject), the text,
// the attachments, and Discard apart on the left from Send. A Slack card also offers Always Send,
// which sends it and has the bot send its next messages directly. Once sent or discarded the card
// keeps the message as it went, with how it ended on the title's line.

// draftRow is one header row: its label and the field it edits.
type draftRow struct {
	label string
	value *string
}

// draftCardState is a draft card's own: what the user typed, the files they kept, and a request
// under way, until the card's version or state changes.
type draftCardState struct {
	version                    uint64
	state                      string
	to, cc, bcc, subject, body string
	files                      []model.DraftFile
	busy                       bool
}

func newDraftCardState(card *model.DraftCard) draftCardState {
	f := card.Fields
	return draftCardState{
		version: card.Version, state: card.State, to: strings.Join(f.To, ", "), cc: strings.Join(f.Cc, ", "), bcc: strings.Join(f.Bcc, ", "),
		subject: f.Subject, body: f.Body, files: slices.Clone(f.Attachments),
	}
}

// fields is the message as the card shows it, with the user's changes.
func (st *draftCardState) fields(card *model.DraftCard) model.DraftFields {
	list := func(text string) []string {
		var out []string
		for _, part := range strings.Split(text, ",") {
			if part = strings.TrimSpace(part); part != "" {
				out = append(out, part)
			}
		}
		return out
	}
	f := card.Fields.Clone()
	f.To, f.Body, f.Attachments = list(st.to), st.body, slices.Clone(st.files)
	if f.IsEmail() {
		f.Cc, f.Bcc, f.Subject = list(st.cc), list(st.bcc), st.subject
	}
	return f
}

// draftFootnote is what Always Send changes, under a Slack card's buttons.
func draftFootnote(card *model.DraftCard) string {
	if card.IsPending() && card.Direct {
		return L("Always Send also sends this bot's next Slack messages directly.")
	}
	return ""
}

func draftSpokenText(card *model.DraftCard, botName string) string {
	parts := []string{card.Title(botName)}
	if state := card.StateText(); state != "" {
		parts = append(parts, state)
	}
	if len(card.Fields.To) > 0 {
		parts = append(parts, L("To %@", strings.Join(card.Fields.To, ", ")))
	}
	if card.Fields.Subject != "" {
		parts = append(parts, card.Fields.Subject)
	}
	return strings.Join(append(parts, card.Fields.Body), ": ")
}

// fileSize is "186 KB", "1.5 MB", as Finder counts.
func fileSize(bytes int64) string {
	if bytes >= 1_000_000 {
		return fmt.Sprintf("%.1f MB", float64(bytes)/1_000_000)
	}
	return model.Kilobytes(int(bytes))
}

func (m *mainWindow) draftCard(c *ui.Context, chat *model.Chat, message *model.Message, cardAvatar *avatarContent, groupStart bool) {
	p := colors(c)
	card := message.Body.Draft
	name := botName(message)
	chatID, messageID, botID := chat.ID, message.ID, message.Author.BotID
	holder := ui.Box(c.Key("draft:" + message.ID))
	st := ui.Local(holder, "draft", func() draftCardState { return newDraftCardState(card) })
	// The same card keeps what the user is typing; a new version or state shows what the Runner has.
	if st.version != card.Version || st.state != card.State {
		*st = newDraftCardState(card)
	}
	pending := card.IsPending()
	shown := st.fields(card)
	sendable := len(shown.To) > 0 && strings.TrimSpace(shown.Body) != ""
	send := func(always bool) {
		if st.busy || !sendable {
			return
		}
		st.busy = true
		store.SendDraft(chatID, messageID, botID, *card, shown, always, func(err error) {
			st.busy = false
			if err != nil {
				m.showAlert(alertOptions{Message: L("Couldn't send this draft"), Informative: model.ErrorText(err)}, nil)
			}
		})
	}
	discard := func() {
		if st.busy {
			return
		}
		st.busy = true
		store.DiscardDraft(chatID, messageID, *card, func(err error) {
			st.busy = false
			if err != nil {
				m.showAlert(alertOptions{Message: L("Couldn't discard this draft"), Informative: model.ErrorText(err)}, nil)
			}
		})
	}
	rows := []draftRow{{L("To"), &st.to}}
	if card.Fields.IsEmail() {
		if len(card.Fields.Cc) > 0 {
			rows = append(rows, draftRow{L("Cc"), &st.cc})
		}
		if len(card.Fields.Bcc) > 0 {
			rows = append(rows, draftRow{L("Bcc"), &st.bcc})
		}
		rows = append(rows, draftRow{L("Subject"), &st.subject})
	}
	// The text's room comes from the message as the bot wrote it, so typing never moves what is
	// under the card; a longer message scrolls.
	lines := 0
	for _, paragraph := range strings.Split(card.Fields.Body, "\n") {
		lines += max(1, (utf8.RuneCountInString(paragraph)+63)/64)
	}
	if pending {
		lines = max(lines, 3)
	}
	lines = min(lines, 14)
	icon := "bubble.left"
	if card.Fields.IsEmail() {
		icon = "envelope"
	}
	holder.Children(func() {
		cardFrame(c, cardAvatar, groupStart, draftSpokenText(card, name), func() {
			ui.Row(c).Size(18, 18).Center().TextColor(p.Accent).Children(func() { symbol(c, icon, 17, 1.9) })
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
				ui.Row(c).Gap(8).MinWidth(0).Children(func() {
					ui.Text(c, card.Title(name)).Grow(1).Shrink(1).MinWidth(0).FontSize(12.5).FontWeight(600).FixedLineHeight(17).SingleLine()
					if state := card.StateText(); state != "" {
						color := p.Label2
						switch card.State {
						case "failed":
							color = p.Red
						case "uncertain":
							color = p.Orange
						}
						ui.Text(c, state).FontSize(textCaption).FixedLineHeight(17).TextColor(color).SingleLine()
					}
				})
				ui.Text(c, card.Account).Margin(2, 0, 0, 0).FontSize(textCaption).LineHeight(1.35).TextColor(p.Label2).SingleLine()
				ui.Column(c).Margin(6, 0, 0, 0).Children(func() {
					for _, row := range rows {
						line := p.Separator
						ui.Row(c.Key("row-" + row.label)).Height(26).AlignItems(ui.Center).Gap(6).DrawOver(func(painter *ui.Painter, r ui.Rect) {
							painter.Fill(ui.Rect{X: r.X, Y: r.Y + r.H - 1, W: r.W, H: 1}, line, 0)
						}).Children(func() {
							ui.Text(c, row.label).Width(draftLabelWidth(card)).FontSize(12).TextColor(p.Label2).SingleLine()
							textField(c, row.value, fieldOptions{Plain: true, ReadOnly: !pending, Disabled: st.busy, Label: row.label}).
								Grow(1).Shrink(1).MinWidth(0).MinHeight(22).Padding(0, 0).FontSize(12)
						})
					}
				})
				if pending {
					textArea(c, &st.body, lines, fieldOptions{Plain: true, Disabled: st.busy, Label: L("Message")}).Margin(8, 0, 0, 0).Padding(0, 0)
				} else {
					// As it went: the text at its own height.
					ui.Text(c, card.Fields.Body).Margin(8, 0, 0, 0).FontSize(13).LineHeight(1.4).Selectable()
				}
				for i, file := range st.files {
					ui.Row(c.Key(fmt.Sprintf("file-%d", i))).Height(20).Gap(6).AlignItems(ui.Center).Children(func() {
						ui.Row(c).TextColor(p.Label2).Children(func() { symbol(c, "paperclip", 12, 1.75) })
						ui.Text(c, file.Name).FontSize(12).SingleLine().Shrink(1).MinWidth(0).Tooltip(file.Name)
						if file.Size > 0 {
							ui.Text(c, fileSize(file.Size)).FontSize(11).TextColor(p.Label2).SingleLine()
						}
						if pending {
							remove := ui.ButtonBase(c).TextColor(p.Label3).Cursor(ui.CursorPointer).Label(L("Remove %@", file.Name)).Tooltip(L("Remove %@", file.Name))
							remove.Children(func() { symbol(c, "xmark.circle.fill", 12, 1.75) })
							if remove.Clicked() && !st.busy {
								st.files = slices.Delete(slices.Clone(st.files), i, i+1)
							}
						}
					})
				}
				if card.Note != "" {
					color := p.Orange
					if card.State == "failed" {
						color = p.Red
					}
					ui.Text(c, card.Note).Margin(8, 0, 0, 0).FontSize(11).LineHeight(1.35).TextColor(color).MaxLines(3)
				}
				if pending {
					ui.Row(c).Gap(8).Margin(12, 0, 0, 0).Children(func() {
						if pushButton(c, L("Discard"), pushOptions{Small: true, Disabled: st.busy}).Clicked() {
							discard()
						}
						ui.Box(c).Grow(1)
						if card.Direct && pushButton(c, L("Always Send"), pushOptions{Small: true, Disabled: st.busy || !sendable}).Clicked() {
							send(true)
						}
						if pushButton(c, L("Send"), pushOptions{Small: true, Kind: buttonPrimary, Disabled: st.busy || !sendable}).Clicked() {
							send(false)
						}
					})
				}
				if note := draftFootnote(card); note != "" {
					ui.Text(c, note).Margin(7, 0, 0, 0).FontSize(11).LineHeight(1.35).TextColor(p.Label2)
				}
			})
		})
	})
}

// draftLabelWidth is the header labels' column: as wide as the longest label the card shows.
func draftLabelWidth(card *model.DraftCard) float32 {
	if card.Fields.IsEmail() {
		return 52
	}
	return 24
}
