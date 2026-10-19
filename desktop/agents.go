package main

import (
	"math"
	"reflect"
	"strconv"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Coding agents, after Grok Bot's: a coding agent is a peer the bot messages. Its start and each
// message the bot sends it are centered markers, "Messaged ⬢ Title · first line", what the user
// writes it on its page is "You messaged ⬢ Title · first line", and what it says back is "Message
// from ⬢ Title · first line"; a click opens its page. While it asks to run a command, the
// permission card sits under its start's marker. The page is its transcript, drawn as Cursor
// draws an agent's run, with a composer under it that writes to the agent itself.

// MARK: - The markers

// agentMarkerText is the marker's words, for assistive technology and the tests.
func agentMarkerText(lead, title, text string) string {
	return strings.TrimSpace(lead + " " + title + ": " + firstLineOf(text))
}

// agentMarker is "Messaged ⬢ Title · first line" or "Message from ⬢ Title · first line",
// centered as the bot-to-bot markers are, the agent's mark in place of a bot's avatar. It answers
// a click; with `full` set, a click shows that in a popover instead, as for an agent that never
// started. A tint colors the first line, as red does what an agent said as it failed.
func agentMarker(c *ui.Context, lead, kind, title, text, full string, tint *ui.Color, groupStart bool) bool {
	p := colors(c)
	top := float32(groupTopPadding)
	if !groupStart {
		top = tightTopPadding
	}
	first := firstLineOf(text)
	clicked := false
	ui.Row(c).Padding(top, horizontalInset, 0, horizontalInset).Justify(ui.Center).Children(func() {
		marker := ui.Row(c).Gap(5).MaxWidthPercent(100).MinWidth(0).Padding(3, 9).Radius(12).FontSize(11.5).TextColor(p.Label2).
			Label(agentMarkerText(lead, title, text)).Role(ui.RoleButton).Cursor(ui.CursorPointer)
		if full != "" {
			marker.Tooltip(firstLineOf(full))
			textPopover(c, marker, full)
		} else {
			clicked = marker.Clicked()
		}
		marker.Children(func() {
			ui.Text(c, lead).NoWrap()
			ui.Row(c).Size(16, 16).Center().TextColor(p.Label).Children(func() { agentLogo(c, kind, 14) })
			ui.Text(c, title).FontWeight(500).TextColor(p.Label).MaxWidth(320).Shrink(0.5).MinWidth(0).SingleLine()
			if first != "" {
				t := ui.Text(c, "· "+first).MaxWidth(280).Shrink(1).MinWidth(0).SingleLine()
				if tint != nil {
					t.TextColor(*tint)
				}
			}
		})
	})
	return clicked
}

// agentRow is a coding agent's start or a message the bot sent one: its marker. What the agent
// asks is a permission card of its own at the end of the chat. A start that failed has no page:
// its marker shows why in a popover.
func (m *mainWindow) agentRow(c *ui.Context, chat *model.Chat, message *model.Message, groupStart bool) {
	tool := message.Body.Tool
	agent := model.MessagedAgent(chat, tool)
	kind, rowID := "", message.ID
	if agent != nil {
		kind = agent.Kind
	}
	if tool.Agent == nil {
		rowID = ""
		if row := model.AgentRowOf(chat, tool.TargetAgentID); row != nil {
			rowID = row.ID
		}
	}
	text := tool.AgentText()
	full := ""
	switch {
	case agent != nil && !agent.Started():
		full = firstNonEmpty(agent.Outcome, text)
	case rowID == "":
		full = text
	}
	chatID := chat.ID
	ui.Column(c.Key("agent:" + message.ID)).Children(func() {
		if agentMarker(c, L("Messaged"), kind, model.AgentLabel(chat, tool), text, full, nil, groupStart) {
			m.presentAgentPage(chatID, rowID)
		}
	})
}

// agentMessageRow is what a coding agent said: "Message from ⬢ Title · first line", its first line
// in red when it ended on its own; or what the user wrote it on its page, "You messaged ⬢ Title ·
// first line". A click opens the agent's page, or once its start is gone, shows the whole message.
func (m *mainWindow) agentMessageRow(c *ui.Context, chat *model.Chat, message *model.Message, groupStart bool) {
	said := message.Body.AgentMessage
	row := model.AgentRowOf(chat, said.AgentID)
	title := firstNonEmpty(said.Title, model.CodingAgentName(said.AgentKind))
	full := ""
	if row == nil {
		full = said.Text
	}
	var tint *ui.Color
	if said.Failed {
		tint = &colors(c).Red
	}
	lead := L("Message from")
	if message.Author.Kind == model.AuthorYou {
		lead = L("You messaged")
	}
	ui.Column(c.Key("agent-message:" + message.ID)).Children(func() {
		if agentMarker(c, lead, said.AgentKind, title, said.Text, full, tint, groupStart) && row != nil {
			m.presentAgentPage(chat.ID, row.ID)
		}
	})
}

// MARK: - How it stands

// agentStatusMark is a coding agent's state as a mark: a spinner while it works, an exclamation
// mark when it needs the user (red when it failed), the pull request it is ready with, a check
// when it finished, a stop mark once stopped.
func agentStatusMark(c *ui.Context, agent *model.AgentRun) {
	p := colors(c)
	name, tint := "stop.circle", p.Label3
	switch agent.State {
	case model.AgentStarting, model.AgentWorking:
		spinner(c, 13)
		return
	case model.AgentAsking:
		name, tint = "exclamationmark.circle.fill", p.Orange
	case model.AgentIdle:
		name, tint = "checkmark.circle", p.Green
		if agent.PullRequest != "" {
			name, tint = "arrow.triangle.pull", p.Accent
		}
	case model.AgentFailed:
		name, tint = "exclamationmark.circle.fill", p.Red
	}
	ui.Row(c).TextColor(tint).Children(func() { symbol(c, name, 15, 1.8) })
}

// agentRowDetail is the line under an agent's title in Details: how it stands, and its pull
// request or its branch.
func agentRowDetail(agent *model.AgentRun) string {
	if pr := agent.PullNumber(); pr != "" {
		return agent.Status() + " · " + pr
	}
	if place := agent.Place(); place != "" {
		return agent.Status() + " · " + place
	}
	return agent.Status()
}

// MARK: - The page

// agentPage follows a coding agent's transcript while its page is up, asking its Runner again
// every two seconds, five through the relay, for as long as the agent has not ended.
type agentPage struct {
	chatID, messageID string
	remote            bool
	items             []model.AgentItem
	// queued are the messages it was sent and has yet to take up.
	queued []model.AgentQueued
	// reachable is whether a message can reach it, as the transcript last answered.
	reachable bool
	// draft is the composer's text; sending, a message on its way; sendErr, why the last did not go.
	draft   string
	sending bool
	sendErr string
	focus   bool
	// card is the agent as the transcript last answered it, for when its row is gone.
	card     *model.AgentRun
	err      string
	loaded   bool
	closed   bool
	stopping bool
	timer    *time.Timer
	scroll   ui.ScrollState
	// open are the steps shown in full: a fold of reads and searches, a thought, a command's
	// output, the long first prompt.
	open map[string]bool
}

// agent is the agent as its row says now, which events keep current.
func (t *agentPage) agent() *model.AgentRun {
	if chat := store.Chat(t.chatID); chat != nil {
		if agent := model.AgentOf(chat.Message(t.messageID)); agent != nil {
			return agent
		}
	}
	return t.card
}

func (t *agentPage) load() {
	if t.closed {
		return
	}
	store.AgentTranscript(t.chatID, t.messageID, func(transcript model.AgentTranscript, err error) {
		if t.closed {
			return
		}
		if err != nil {
			// What was shown stays; the next try may reach the Runner.
			if !t.loaded {
				t.err = model.ErrorText(err)
			}
		} else {
			// New steps scroll into view while the reader is at the end.
			atEnd := !t.loaded || t.scroll.Y >= t.scroll.MaxY-24
			if atEnd && (!reflect.DeepEqual(transcript.Items, t.items) || !reflect.DeepEqual(transcript.Queued, t.queued)) {
				t.scroll.Y = math.MaxFloat32
			}
			t.items, t.queued, t.reachable, t.loaded, t.err = transcript.Items, transcript.Queued, transcript.Open, true, ""
			if transcript.Card != nil {
				t.card = transcript.Card
			}
		}
		t.schedule()
	})
}

func (t *agentPage) schedule() {
	// One poll at a time: a send asks again at once.
	if t.timer != nil {
		t.timer.Stop()
	}
	if t.closed || store.IsMock {
		return
	}
	if agent := t.agent(); agent != nil && !agent.IsOpen() {
		return
	}
	every := 2 * time.Second
	if t.remote {
		every = 5 * time.Second
	}
	t.timer = time.AfterFunc(every, func() { post(t.load) })
}

func (t *agentPage) close() {
	t.closed = true
	if t.timer != nil {
		t.timer.Stop()
	}
}

// presentAgentPage shows a coding agent's page: how it stands, its transcript, what its work
// changed, and a composer that writes to the agent itself.
func (w *appWindow) presentAgentPage(chatID, messageID string) {
	chat := store.Chat(chatID)
	if chat == nil {
		return
	}
	row := chat.Message(messageID)
	if agent := model.AgentOf(row); agent == nil || !agent.Started() {
		return
	}
	t := &agentPage{chatID: chatID, messageID: messageID, remote: !store.RunsHere(row), open: map[string]bool{}}
	t.load()
	w.present(func(c *ui.Context, s *sheet) { t.view(c, s, w) }, t.close)
}

// subtitle is "Claude Code · Working · fix-login", and the Runner's name when it is another
// Device.
func (t *agentPage) subtitle(agent *model.AgentRun) string {
	parts := []string{agent.Name(), agent.Status()}
	if place := agent.Place(); place != "" {
		parts = append(parts, place)
	}
	if t.remote && agent.Device != "" {
		parts = append(parts, agent.Device)
	}
	return strings.Join(parts, " · ")
}

func (t *agentPage) stop(w *appWindow, agent *model.AgentRun) {
	if t.stopping {
		return
	}
	t.stopping = true
	name := agent.Name()
	store.StopAgent(t.chatID, t.messageID, func(err error) {
		t.stopping = false
		if err != nil && !t.closed {
			w.showAlert(alertOptions{Message: L("Could not stop %@", name), Informative: model.ErrorText(err)}, nil)
		}
	})
}

// send gives the agent what the composer holds, down the path the bot's messages take. The text
// leaves the field at once and comes back with why when it could not go; once it went, the
// transcript is asked again at once, for the message in the agent's queue and how it stands.
func (t *agentPage) send() {
	text := strings.TrimSpace(t.draft)
	if text == "" || t.sending {
		return
	}
	t.sending, t.sendErr, t.draft = true, "", ""
	t.scroll.Y = math.MaxFloat32
	store.SendToAgent(t.chatID, t.messageID, text, func(err error) {
		t.sending = false
		if t.closed {
			return
		}
		if err != nil {
			t.sendErr = model.ErrorText(err)
			if strings.TrimSpace(t.draft) == "" {
				t.draft = text
			}
			t.focus = true
			return
		}
		t.load()
	})
}

// composerState is what the composer says for the agent as it stands: its placeholder, the line
// under it, and whether it takes a message. One at work takes it up once it is done with what it
// does now; one whose process is gone starts again on its session; one still starting, or one
// that ended before its session began, takes none.
func (t *agentPage) composerState(agent *model.AgentRun) (placeholder, hint string, enabled bool) {
	name := agent.Name()
	placeholder = L("Message %@", name)
	switch {
	case agent.State == model.AgentStarting:
		return L("%@ is starting…", name), "", false
	case t.loaded && !t.reachable:
		return placeholder, L("%@ ended before its session began, so it cannot take a message. Ask the bot to start a new one.", name), false
	case agent.State == model.AgentWorking || agent.State == model.AgentAsking:
		return placeholder, L("%@ takes it up once it is done with what it is doing.", name), true
	case agent.State == model.AgentStopped || agent.State == model.AgentFailed:
		return placeholder, L("Sending starts %@ again on its session.", name), true
	}
	return placeholder, "", true
}

// composer is the page's field, which writes to the agent itself: Return sends as it does in the
// chat, and the line under it says when the agent takes the message, or why it did not go.
func (t *agentPage) composer(c *ui.Context, agent *model.AgentRun) {
	p := colors(c)
	placeholder, hint, enabled := t.composerState(agent)
	sendOnReturn := prefs.get().SendOnReturn
	ready := enabled && !t.sending && strings.TrimSpace(t.draft) != ""
	ui.Column(c.Key("composer")).Gap(6).Padding(12, 20, 0, 20).Children(func() {
		field := ui.Row(c).Gap(8).AlignItems(ui.End).MinHeight(38).Padding(5, 5, 5, 12).Radius(19).Border(0.5, p.ComposerBorder).
			Background(p.ComposerField).Cursor(ui.CursorText)
		field.Children(func() {
			ui.Box(c.Key("text")).Grow(1).Shrink(1).MinWidth(0).Padding(0, 0, 3, 0).Children(func() {
				input := ui.TextAreaBase(c, &t.draft).Lines(1, 6).Padding(4, 0).FontSize(textMessage).FixedLineHeight(19).
					TextColor(p.Label).Placeholder(placeholder).Label(placeholder).FocusRing(false).Disabled(!enabled)
				composing := input.Composing()
				input.HandleInput(func(ev ui.InputEvent) bool {
					if ev.Kind != ui.InputKeyDown || composing || ev.Key != ui.KeyEnter {
						return false
					}
					if (sendOnReturn && ev.Mods == 0) || (!sendOnReturn && ev.Mods == ui.Cmd) {
						t.send()
						return true
					}
					return false
				})
				if input.Changed() {
					t.sendErr = ""
				}
				if t.focus && enabled {
					input.Focus()
				}
				t.focus = false
			})
			button := ui.ButtonBase(c.Key("send")).Size(28, 28).Shrink(0).Radius(14).Center().Background(p.ComposerPrimary).
				TextColor(p.ComposerPrimaryContent).Label(L("Send")).Tooltip(L("Send to %@", agent.Name()))
			if !ready {
				// Faded as one piece, as the chat's send button.
				button.Background(p.ComposerPrimary.Alpha(0.25)).TextColor(p.ComposerField.Alpha(1))
			}
			button.Children(func() {
				if t.sending {
					spinner(c, 13)
				} else {
					symbol(c, "arrow.up", 15, 2.6)
				}
			})
			if button.Clicked() && ready {
				t.send()
			}
		})
		// A click on the pill around the text puts the keyboard in it.
		if field.Clicked() && enabled {
			t.focus = true
		}
		switch {
		case t.sendErr != "":
			ui.Text(c, L("Not sent: %@", t.sendErr)).FontSize(11.5).LineHeight(1.35).TextColor(p.Red).Padding(0, 12).Selectable()
		case hint != "":
			ui.Text(c, hint).FontSize(11.5).LineHeight(1.35).TextColor(p.Label3).Padding(0, 12)
		}
	})
}

func (t *agentPage) view(c *ui.Context, s *sheet, w *appWindow) {
	p := colors(c)
	agent := t.agent()
	if agent == nil {
		s.dismiss()
		return
	}
	done := false
	panel := sheetPanel(c, 680).Padding(0).Label(agent.Label())
	panel.Focusable().FocusRing(false).AutoFocus()
	panel.Children(func() {
		ui.Row(c).Gap(10).AlignItems(ui.Start).Padding(18, 20, 14, 20).Children(func() {
			ui.Row(c).Size(22, 22).Center().TextColor(p.Label).Margin(1, 0, 0, 0).Children(func() { agentLogo(c, agent.Kind, 20) })
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(2).Children(func() {
				ui.Text(c, agent.Label()).FontSize(15).FontWeight(600).LineHeight(1.3).MaxLines(2).Tooltip(agent.Label())
				ui.Text(c, t.subtitle(agent)).FontSize(12).TextColor(p.Label2).SingleLine()
			})
			ui.Row(c).Gap(8).Shrink(0).Children(func() {
				if pr := agent.PullNumber(); pr != "" {
					b := pushButton(c.Key("pull-request"), pr, pushOptions{}).Tooltip(agent.PullRequest)
					if b.Clicked() {
						_ = openExternal(agent.PullRequest)
					}
				}
				if agent.IsRunning() && pushButton(c.Key("stop"), L("Stop"), pushOptions{Disabled: t.stopping}).Clicked() {
					t.stop(w, agent)
				}
			})
		})
		ui.Box(c).Height(1).Background(p.Separator)
		body := ui.Scroll(c.Key("transcript")).Height(460).Shrink(1).MinHeight(120).Padding(14, 20, 18, 20).Label(L("Transcript"))
		body.TrackScroll(&t.scroll)
		body.Children(func() { t.transcript(c) })
		ui.Box(c).Height(1).Background(p.Separator)
		t.composer(c, agent)
		ui.Row(c).Gap(10).Padding(12, 20, 18, 20).Children(func() {
			ui.Row(c).Grow(1).Shrink(1).MinWidth(0).Gap(12).Children(func() { agentFooter(c, agent) })
			if pushButton(c.Key("done"), L("Done"), pushOptions{Kind: buttonPrimary}).Clicked() {
				done = true
			}
		})
	})
	if panel.OverlayShortcut(0, ui.KeyEscape) || panel.OverlayShortcut(0, ui.KeyEnter) {
		done = true
	}
	if done {
		s.dismiss()
	}
}

// agentFooter is what the agent's work changed, "+12 −3 in 4 files", and its branch.
func agentFooter(c *ui.Context, agent *model.AgentRun) {
	p := colors(c)
	if changes := agent.Changes; changes != nil && changes.Files > 0 {
		files := L("in %d files", changes.Files)
		if changes.Files == 1 {
			files = L("in 1 file")
		}
		ui.RichText(c,
			ui.Span{Text: "+" + strconv.Itoa(changes.Added), Color: p.Green, Weight: 500},
			ui.Span{Text: " −" + strconv.Itoa(changes.Removed), Color: p.Red, Weight: 500},
			ui.Span{Text: " " + files, Color: p.Label2},
		).FontSize(12).SingleLine().Shrink(0)
	}
	if agent.Branch != "" {
		ui.Row(c).Gap(4).Shrink(1).MinWidth(0).TextColor(p.Label2).Tooltip(agent.Folder).Children(func() {
			symbol(c, "arrow.triangle.branch", 12, 2)
			ui.Text(c, agent.Branch).Font(monoFont).FontSize(11.5).SingleLine().Shrink(1).MinWidth(0)
		})
	}
}

// MARK: - The transcript

// agentStep is a line of the page: one item, or a run of reads and searches folded into one.
type agentStep struct {
	item     model.AgentItem
	explored []model.AgentItem
}

func (s agentStep) isTool() bool { return s.item.Kind == "tool" && s.item.Tool != nil }

// agentSteps folds each run of reads and searches into one step.
func agentSteps(items []model.AgentItem) []agentStep {
	var steps []agentStep
	for _, item := range items {
		if item.Kind == "tool" && item.Tool != nil && item.Tool.Explores() {
			if n := len(steps); n > 0 && steps[n-1].explored != nil {
				steps[n-1].explored = append(steps[n-1].explored, item)
				continue
			}
			steps = append(steps, agentStep{item: item, explored: []model.AgentItem{item}})
			continue
		}
		steps = append(steps, agentStep{item: item})
	}
	return steps
}

// exploredText is a fold's line: "Explored 3 files 2 searches", "Exploring 1 file" while one runs.
func exploredText(items []model.AgentItem) string {
	files, searches, running := 0, 0, false
	for _, item := range items {
		if item.Tool.Kind == "read" {
			files++
		} else {
			searches++
		}
		running = running || item.Tool.Status == "running"
	}
	var parts []string
	switch {
	case files == 1:
		parts = append(parts, L("1 file"))
	case files > 1:
		parts = append(parts, L("%d files", files))
	}
	switch {
	case searches == 1:
		parts = append(parts, L("1 search"))
	case searches > 1:
		parts = append(parts, L("%d searches", searches))
	}
	switch {
	case len(parts) == 2 && running:
		return L("Exploring %@ %@", parts[0], parts[1])
	case len(parts) == 2:
		return L("Explored %@ %@", parts[0], parts[1])
	case running:
		return L("Exploring %@", parts[0])
	}
	return L("Explored %@", parts[0])
}

// agentToolText is a step's line, in the past once it is done and in the present while it runs:
// "Ran bun test", "Editing src/a.rs", "Not allowed: git push".
func agentToolText(step *model.AgentToolStep) string {
	running := step.Status == "running"
	if step.Status == "denied" {
		return L("Not allowed: %@", step.Title)
	}
	switch {
	case step.Kind == "read" && running:
		return L("Reading %@", step.Title)
	case step.Kind == "read":
		return L("Read %@", step.Title)
	case step.Kind == "search" && running:
		return L("Searching %@", step.Title)
	case step.Kind == "search":
		return L("Searched %@", step.Title)
	case step.Kind == "edit" && running:
		return L("Editing %@", step.Title)
	case step.Kind == "edit":
		return L("Edited %@", step.Title)
	case step.Kind == "execute" && running:
		return L("Running %@", step.Title)
	case step.Kind == "execute":
		return L("Ran %@", step.Title)
	case step.Kind == "fetch" && running:
		return L("Fetching %@", step.Title)
	case step.Kind == "fetch":
		return L("Fetched %@", step.Title)
	case running:
		return L("Using %@", step.Title)
	}
	return L("Used %@", step.Title)
}

// toolSpans are a step's line with its subject in the text color, a command in monospace, and an
// edit's counts in green and red; a failed step reads in red.
func toolSpans(p *palette, step *model.AgentToolStep) []ui.Span {
	text := agentToolText(step)
	quiet, subject := p.Label2, p.Label
	if step.Status == "failed" {
		quiet, subject = p.Red, p.Red
	}
	font := ""
	if step.Kind == "execute" {
		font = monoFont
	}
	var spans []ui.Span
	if i := strings.Index(text, step.Title); step.Title != "" && i >= 0 {
		spans = append(spans, ui.Span{Text: text[:i], Color: quiet}, ui.Span{Text: step.Title, Color: subject, Font: font},
			ui.Span{Text: text[i+len(step.Title):], Color: quiet})
	} else {
		spans = append(spans, ui.Span{Text: text, Color: quiet})
	}
	if step.Kind == "edit" && step.Status != "denied" && (step.Added != nil || step.Removed != nil) {
		if step.Added != nil {
			spans = append(spans, ui.Span{Text: "  +" + strconv.Itoa(*step.Added), Color: p.Green, Weight: 500})
		}
		if step.Removed != nil {
			spans = append(spans, ui.Span{Text: " −" + strconv.Itoa(*step.Removed), Color: p.Red, Weight: 500})
		}
	}
	return spans
}

// stepLine is a step's one line: a spinner while it runs, its words, and when it holds more, a
// chevron that shows it. It answers a click that opens or closes it.
func stepLine(c *ui.Context, spans []ui.Span, label string, running, expandable, open bool, size float32) bool {
	p := colors(c)
	var line ui.Element
	clicked := false
	if expandable {
		line = ui.ButtonBase(c).Cursor(ui.CursorPointer).FocusRing(false)
		if line.Hovered() {
			line.Background(p.RowHover)
		}
		clicked = line.Clicked()
	} else {
		line = ui.Row(c)
	}
	line.Gap(6).MinHeight(22).Padding(2, 6).Margin(0, -6).Radius(6).MinWidth(0).MaxWidthPercent(100).AlignSelf(ui.Start).Label(label).Tooltip(label)
	line.Children(func() {
		if running {
			spinner(c, 11)
		}
		ui.RichText(c, spans...).FontSize(size).SingleLine().Shrink(1).MinWidth(0)
		if expandable {
			name := "chevron.right"
			if open {
				name = "chevron.down"
			}
			ui.Row(c).Shrink(0).TextColor(p.Label3).Children(func() { symbol(c, name, 11, 2.2) })
		}
	})
	return clicked
}

// transcript is the page's body: each step in order, or what stands in for them.
func (t *agentPage) transcript(c *ui.Context) {
	p := colors(c)
	switch {
	case len(t.items) == 0 && len(t.queued) == 0 && t.err != "":
		ui.Text(c, t.err).FontSize(12).TextColor(p.Label2).Selectable()
		return
	case len(t.items) == 0 && len(t.queued) == 0:
		words := L("Loading…")
		if t.loaded {
			words = L("Nothing yet")
		}
		ui.Text(c, words).FontSize(12).TextColor(p.Label2)
		return
	}
	steps := agentSteps(t.items)
	ui.Column(c).Children(func() {
		for i, step := range steps {
			// Tool lines sit close together; the rest a paragraph apart.
			top := float32(12)
			switch {
			case i == 0:
				top = 0
			case step.isTool() && steps[i-1].isTool():
				top = 2
			}
			ui.Column(c.Key(step.item.ID)).Margin(top, 0, 0, 0).MinWidth(0).Children(func() { t.step(c, step, i == 0) })
		}
		// What it was sent and has yet to take up, under what it does now.
		for i, queued := range t.queued {
			top := float32(12)
			if i == 0 && len(steps) == 0 {
				top = 0
			}
			ui.Column(c.Key("queued:"+strconv.Itoa(i))).Margin(top, 0, 0, 0).MinWidth(0).Children(func() {
				agentPromptCard(c, t.promptAuthor(queued.From), queued.Text, queued.From == "user", true, false, nil)
			})
		}
	})
}

// promptAuthor is who wrote a message to the agent: "You" for the user on its page, else the bot
// that passed it on.
func (t *agentPage) promptAuthor(from string) string {
	if from == "user" {
		return L("You")
	}
	if chat := store.Chat(t.chatID); chat != nil {
		if row := chat.Message(t.messageID); row != nil {
			if bot := store.Bot(row.Author.BotID); bot != nil {
				return bot.Name
			}
		}
	}
	return L("Bot")
}

func (t *agentPage) step(c *ui.Context, step agentStep, first bool) {
	p := colors(c)
	item := step.item
	open := t.open[item.ID]
	toggle := func() { t.open[item.ID] = !open }
	switch {
	case step.explored != nil:
		text := exploredText(step.explored)
		running := false
		for _, each := range step.explored {
			running = running || each.Tool.Status == "running"
		}
		if stepLine(c, []ui.Span{{Text: text, Color: p.Label2}}, text, running, true, open, 12.5) {
			toggle()
		}
		if open {
			ui.Column(c).Padding(2, 0, 2, 14).Children(func() {
				for _, each := range step.explored {
					ui.Box(c.Key(each.ID)).Children(func() {
						stepLine(c, toolSpans(p, each.Tool), agentToolText(each.Tool), each.Tool.Status == "running", false, false, 12)
					})
				}
			})
		}
	case item.Kind == "tool" && item.Tool != nil:
		tool := item.Tool
		expandable := tool.Output != "" && tool.Status != "running"
		if stepLine(c, toolSpans(p, tool), agentToolText(tool), tool.Status == "running", expandable, open, 12.5) {
			toggle()
		}
		if expandable && open {
			commandOutput(c, tool.Output, 12).Margin(4, 0, 2, 0)
		}
	case item.Kind == "prompt":
		agentPromptCard(c, t.promptAuthor(item.From), item.Text, item.From == "user", false, open, toggle)
	case item.Kind == "status":
		ui.Text(c, item.Text).FontSize(11.5).TextColor(p.Label3).Selectable()
	case item.Kind == "thought":
		text := Lc("Thought", "coding agent")
		if item.Seconds != nil {
			text = L("Thought for %ds", max(1, int(math.Round(*item.Seconds))))
		}
		expandable := strings.TrimSpace(item.Text) != ""
		if stepLine(c, []ui.Span{{Text: text, Color: p.Label2}}, text, false, expandable, open, 12.5) {
			toggle()
		}
		if expandable && open {
			ui.Text(c, item.Text).Padding(2, 0, 0, 14).FontSize(12).LineHeight(1.45).TextColor(p.Label2).Selectable()
		}
	case item.Kind == "text":
		markdownView(c, item.Text, markdownOptions{Size: 13})
	case item.Kind == "plan":
		ui.Column(c).Gap(5).Padding(9, 12).Radius(9).Background(p.Code).Label(L("Plan")).Children(func() {
			for i, entry := range item.Plan {
				name, tint, text := "circle", p.Label3, p.Label
				switch entry.Status {
				case "completed":
					name, tint, text = "checkmark.circle.fill", p.Green, p.Label2
				case "in_progress":
					name, tint = "circle.dashed", p.Accent
				}
				ui.Row(c.Key(i)).Gap(8).AlignItems(ui.Start).Children(func() {
					ui.Row(c).Size(14, 17).Center().TextColor(tint).Children(func() { symbol(c, name, 14, 2) })
					ui.Text(c, entry.Text).Grow(1).Shrink(1).MinWidth(0).FontSize(12.5).LineHeight(1.35).TextColor(text)
				})
			}
		})
	case item.Kind == "error":
		ui.Row(c).Gap(6).AlignItems(ui.Start).TextColor(p.Red).Children(func() {
			ui.Row(c).Size(14, 17).Center().Children(func() { symbol(c, "exclamationmark.triangle", 13, 2) })
			ui.Text(c, item.Text).Grow(1).Shrink(1).MinWidth(0).FontSize(12.5).LineHeight(1.35).Selectable()
		})
	}
}

// agentPromptCard is what the agent was sent, under who wrote it: "You", in the accent's tint, for
// what the user wrote on its page, else the bot that passed it on. One it has yet to take up
// reads "Queued". A long one, as the first brief is, folds past six lines behind Show more.
func agentPromptCard(c *ui.Context, author, text string, mine, queued, open bool, toggle func()) {
	p := colors(c)
	long := toggle != nil && (strings.Count(text, "\n") >= 6 || len([]rune(text)) > 480)
	card := ui.Column(c).Gap(4).Padding(9, 12, 10, 12).Radius(10).Background(p.BotBubble).Border(1, p.BotBubbleBorder)
	if mine {
		card.Background(p.Accent.Alpha(0.09)).Border(1, p.Accent.Alpha(0.22))
	}
	card.Children(func() {
		ui.Row(c).Gap(6).Children(func() {
			ui.Text(c, author).FontSize(textCaption).FontWeight(600).TextColor(p.Label3)
			if queued {
				ui.Text(c, "· "+L("Queued")).FontSize(textCaption).TextColor(p.Label3).Tooltip(L("It takes this up once it is done with what it is doing"))
			}
		})
		body := ui.Text(c, text).FontSize(12.5).LineHeight(1.4).Selectable()
		if long && !open {
			body.MaxLines(6)
		}
		if long {
			words := L("Show more")
			if open {
				words = L("Show less")
			}
			if linkButton(c, words, false).AlignSelf(ui.Start).Margin(2, 0, 0, 0).Clicked() {
				toggle()
			}
		}
	})
}
