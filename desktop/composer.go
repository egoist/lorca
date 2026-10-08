package main

import (
	"runtime"
	"slices"
	"strings"
	"unicode"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/plugins/glass"
	"github.com/egoist/mygo/ui"
)

// The composer, after the macOS app's ComposerView: a pill floating over the transcript with the
// + button, the text, and Stop and Send. One line of text sits beside the controls; longer text
// or attachments open it up, the chips above the text and the controls in a row under it.
// Return sends (Shift-Return breaks the line) unless Settings reserves sending for Ctrl-Return.
// `@` offers the bots, and a name picked from the menu goes out with the message by id.

const (
	maxAttachmentBytes = 100 * 1024 * 1024
	maxAttachments     = 10
	composerMaxLines   = 8
	mentionRowHeight   = 38
	mentionRows        = 5
)

// composerReply is the message a draft answers: its id, who wrote it, and how it opens.
type composerReply struct {
	messageID string
	name      string
	text      string
}

type composerState struct {
	draft       string
	attachments []model.OutgoingAttachment
	// picked are the bots chosen from the `@` menu, which count while their `@Name` is in the text.
	picked []string
	reply  *composerReply
	// mentionIndex is the highlighted bot of the `@` menu; mentionClosed is the menu closed with
	// Escape until the caret leaves the `@name`.
	mentionIndex  int
	mentionClosed int
	// focus asks for the keyboard in the field, once; caret puts the caret there, once (-1 none).
	focus bool
	caret int
	// caretAt is where the caret was as the last frame built the field.
	caretAt int
}

func composerOutgoing(file fileInfo) model.OutgoingAttachment {
	mime := file.Mime
	if mime == "" {
		mime = "application/octet-stream"
	}
	return model.OutgoingAttachment{
		Attachment: model.Attachment{ID: model.NewAttachmentID(), Name: file.Name, Mime: mime, Size: file.Size, Width: file.Width, Height: file.Height},
		Path:       file.Path,
	}
}

// mentionRange is the `@token` the caret sits in, in runes: an `@` at the start or after
// whitespace, and no space before the caret.
func mentionRange(text []rune, caret int) (start, end int, ok bool) {
	if caret <= 0 || caret > len(text) {
		return 0, 0, false
	}
	for i := caret - 1; i >= 0; i-- {
		switch ch := text[i]; {
		case ch == '@':
			if i == 0 || unicode.IsSpace(text[i-1]) {
				return i, caret, true
			}
			return 0, 0, false
		case ch == ' ' || ch == '\n':
			return 0, 0, false
		}
		if caret-i > 24 {
			return 0, 0, false
		}
	}
	return 0, 0, false
}

// addFiles adds the files it can; the rest get one alert, with `problems` found before.
func (m *mainWindow) addFiles(s *composerState, files []fileInfo, problems []string) {
	for _, info := range files {
		if len(s.attachments) >= maxAttachments {
			problems = append(problems, L("At most %d files per message.", maxAttachments))
			break
		}
		switch {
		case !info.IsFile:
			problems = append(problems, L("%@ is not a file.", info.Name))
		case info.Size > maxAttachmentBytes:
			problems = append(problems, L("%@ is larger than %d MB.", info.Name, maxAttachmentBytes/1024/1024))
		default:
			s.attachments = append(s.attachments, composerOutgoing(info))
		}
	}
	if len(problems) > 0 {
		m.showAlert(alertOptions{Message: L("Some files were not attached"), Informative: strings.Join(problems, "\n")}, nil)
	}
	s.focus = true
	m.invalidate()
}

// pasteImage takes a picture on the clipboard as an attachment, as a screenshot pasted into the
// composer is. It reports whether there was one with no text beside it.
func (m *mainWindow) pasteImage(s *composerState) bool {
	if mygo.Clipboard.ReadText() != "" {
		return false
	}
	data := mygo.Clipboard.ReadImage()
	if len(data) == 0 {
		return false
	}
	if len(data) > maxAttachmentBytes {
		m.addFiles(s, nil, []string{L("%@ is larger than %d MB.", "image.png", maxAttachmentBytes/1024/1024)})
		return true
	}
	if info, err := savePasted("image.png", data); err == nil {
		m.addFiles(s, []fileInfo{info}, nil)
	}
	return true
}

// frostedGlass is the composer's material: Liquid Glass over a blur of what scrolls under it, so
// the messages behind it frost to faint lines, as under the Mac's NSGlassEffectView.
type frostedGlass struct{}

func (frostedGlass) PaintMaterial(p *ui.Painter, box ui.Rect, radii [4]float32) {
	glass.Blur{Radius: 5}.PaintMaterial(p, box, radii)
	glass.Glass{}.PaintMaterial(p, box, radii)
}

func (m *mainWindow) composerView(c *ui.Context, chat *model.Chat, members []*model.Bot, chatS *chatState) ui.Element {
	p := colors(c)
	s := &chatS.composer
	if m.focusComposer {
		s.focus = true
		m.focusComposer = false
	}
	chatID := chat.ID
	responding := store.IsResponding(chatID) && !chatS.stopping
	hasContent := strings.TrimSpace(s.draft) != "" || len(s.attachments) > 0
	sendOnReturn := prefs.get().SendOnReturn

	// Who `@` can address: members first, then every other bot.
	mentionable := slices.Clone(members)
	for _, bot := range store.Bots {
		if !slices.ContainsFunc(members, func(member *model.Bot) bool { return member.ID == bot.ID }) {
			mentionable = append(mentionable, bot)
		}
	}
	runes := []rune(s.draft)
	var mentions []*model.Bot
	start, end, inMention := mentionRange(runes, s.caretAt)
	if inMention && start != s.mentionClosed-1 {
		query := strings.ToLower(string(runes[start+1 : end]))
		for _, bot := range mentionable {
			if query == "" || strings.HasPrefix(strings.ToLower(bot.Name), query) {
				mentions = append(mentions, bot)
			}
		}
	}
	if len(mentions) == 0 {
		s.mentionIndex = 0
	} else {
		s.mentionIndex = min(s.mentionIndex, len(mentions)-1)
	}
	if !inMention {
		s.mentionClosed = 0
	}
	insertMention := func(bot *model.Bot) {
		insertion := []rune("@" + bot.Name + " ")
		next := append(append(slices.Clone(runes[:start]), insertion...), runes[end:]...)
		s.draft = string(next)
		s.caret = start + len(insertion)
		s.picked = append(s.picked, bot.ID)
		s.focus = true
	}

	send := func() {
		if strings.TrimSpace(s.draft) == "" && len(s.attachments) == 0 {
			return
		}
		value := strings.TrimSpace(s.draft)
		// A pick counts while its `@Name` is still in the text.
		lowered := strings.ToLower(value)
		var ids []string
		for _, id := range s.picked {
			if bot := store.Bot(id); bot != nil && strings.Contains(lowered, "@"+strings.ToLower(bot.Name)) && !slices.Contains(ids, id) {
				ids = append(ids, id)
			}
		}
		attachments := s.attachments
		replyTo := ""
		if s.reply != nil {
			replyTo = s.reply.messageID
		}
		s.draft, s.attachments, s.picked, s.reply, s.caret = "", nil, nil, nil, 0
		chatS.list.ScrollToEnd()
		store.Send(value, attachments, ids, chatID, replyTo)
	}

	// One line beside the controls unless the text needs more room: a newline, a wrap, or files.
	expanded := len(s.attachments) > 0 || s.reply != nil || strings.Contains(s.draft, "\n")
	// Keyed, so its state and size stay its own as the button to the latest comes and goes before it.
	outer := ui.Column(c.Key("composer")).Absolute().Left(0).Right(0).Bottom(0).Padding(8, 20, 14, 20).PassThrough()
	var field ui.Element
	outer.Children(func() {
		field = ui.Grid(c).ColumnTracks(ui.Fixed(28), ui.Fr(1), ui.FitContent()).GapX(8).MinHeight(46).Padding(8).
			Radius(22).Border(0.5, p.ComposerBorder).Material(frostedGlass{}).Cursor(ui.CursorText)
		if !expanded {
			width := field.Bounds().W - 16 - 28 - 16 - 28 - 12
			if responding {
				width -= 32
			}
			if w, _ := c.MeasureText(0, ui.Span{Text: s.draft, Size: textMessage}); width > 0 && w > width {
				expanded = true
			}
		}
		if expanded {
			field.AlignItems(ui.End).GapY(6).Padding(12, 8, 8, 8)
		} else {
			field.AlignItems(ui.Center)
		}
		if dropped := field.DroppedFiles(); dropped != nil {
			var files []fileInfo
			for _, path := range dropped {
				files = append(files, inspect(path))
			}
			m.addFiles(s, files, nil)
		}
		if field.FileDragOver() {
			field.Border(2, p.Accent)
		}
		row := 1
		field.Children(func() {
			if s.reply != nil {
				reply := s.reply
				ui.Row(c.Key("reply")).ColumnSpan(-1).RowStart(row).Gap(6).MinWidth(0).Padding(0, 4, 2, 6).FontSize(12).TextColor(p.Label2).Children(func() {
					symbol(c, "arrowshape.turn.up.left.fill", 13, 2.4)
					ui.RichText(c, ui.Span{Text: L("Replying to %@", reply.name), Weight: 600, Color: p.Label}, ui.Span{Text: " " + reply.text}).Grow(1).Shrink(1).MinWidth(0).SingleLine()
					cancel := ui.ButtonBase(c).TextColor(p.Label3).Label(L("Cancel reply")).Tooltip(L("Cancel reply")).Cursor(ui.CursorPointer)
					cancel.Children(func() { symbol(c, "xmark.circle.fill", 15, 2) })
					if cancel.Clicked() {
						s.reply = nil
						s.focus = true
					}
				})
				row++
			}
			if len(s.attachments) > 0 {
				removed := -1
				// Chips 14 apart side by side (8, and the remove button's overhang), rows 8 apart.
				ui.Row(c.Key("strip")).ColumnSpan(-1).RowStart(row).Wrap().GapX(14).GapY(8).Padding(6, 4).Children(func() {
					for i, item := range s.attachments {
						chip := ui.Box(c.Key(item.Attachment.ID)).Tooltip(item.Attachment.Name)
						chip.Children(func() {
							if item.Attachment.IsImage() {
								ui.Box(c).Size(56, 56).Radius(10).Clip().Children(func() {
									if bitmap := loadBitmap(item.Path); bitmap != nil {
										ui.Image(c, bitmap).Size(56, 56).Fit(ui.Contain)
									}
								})
							} else {
								ui.Row(c).Gap(8).Width(176).Height(56).Padding(0, 10).Radius(10).Background(p.ComposerControl).Children(func() {
									symbol(c, "doc.fill", 18, 1.8)
									ui.Column(c).MinWidth(0).Shrink(1).FontSize(12).Children(func() {
										ui.Text(c, item.Attachment.Name).SingleLine()
										ui.Text(c, model.SizeText(item.Attachment.Size)).FontSize(11).TextColor(p.Label2)
									})
								})
							}
							remove := ui.ButtonBase(c).Absolute().Top(-6).Right(-6).Size(18, 18).Radius(9).Center().Background(ui.RGBA(0, 0, 0, 0.7)).TextColor(p.White).
								Label(L("Remove")).Tooltip(L("Remove"))
							remove.Children(func() { symbol(c, "xmark", 9, 3.2) })
							if remove.Clicked() {
								removed = i
							}
						})
					}
				})
				if removed >= 0 {
					s.attachments = slices.Delete(slices.Clone(s.attachments), removed, removed+1)
					s.focus = true
				}
				row++
			}
			textRow, controlsRow := row, row
			if expanded {
				controlsRow = row + 1
			}
			attach := ui.ButtonBase(c.Key("attach")).RowStart(controlsRow).ColumnStart(1).Size(28, 28).Radius(14).Center().
				Background(p.ComposerControl).TextColor(p.Label).Label(L("Attach files")).Tooltip(L("Attach files"))
			attach.Children(func() { symbol(c, "plus", 16, 2.2) })
			if attach.Clicked() {
				chooseFiles(m.win, "", L("Attach files to your message"), L("Attach"), true, false, func(files []fileInfo) {
					if len(files) > 0 {
						m.addFiles(s, files, nil)
					}
					s.focus = true
				})
			}
			holder := ui.Box(c.Key("text")).RowStart(textRow).MinWidth(0)
			if expanded {
				holder.ColumnStart(1).ColumnSpan(-1).Padding(0, 4)
			} else {
				holder.ColumnStart(2)
			}
			holder.Children(func() {
				placeholder := chatPlaceholder(chat, members)
				if s.reply != nil {
					placeholder = L("Reply…")
				}
				input := ui.TextAreaBase(c, &s.draft).Lines(1, composerMaxLines).Padding(4, 0).FontSize(textMessage).FixedLineHeight(19).
					TextColor(p.Label).Placeholder(placeholder).Label(placeholder).FocusRing(false)
				// Mentions read as addressed, not as typed punctuation: in the bot's color, bold.
				for _, mention := range mentionRuns(runes, mentionable) {
					input.TextRanges(ui.TextRange{Start: mention.start, End: mention.end, Color: p.accentColor(mention.bot.Accent), Weight: 600})
				}
				composing := input.Composing()
				input.HandleInput(func(ev ui.InputEvent) bool {
					if ev.Kind != ui.InputKeyDown || composing {
						return false
					}
					if len(mentions) > 0 {
						switch {
						case ev.Key == ui.KeyUp && ev.Mods == 0:
							s.mentionIndex = (s.mentionIndex - 1 + len(mentions)) % len(mentions)
							return true
						case ev.Key == ui.KeyDown && ev.Mods == 0:
							s.mentionIndex = (s.mentionIndex + 1) % len(mentions)
							return true
						case (ev.Key == ui.KeyTab || ev.Key == ui.KeyEnter) && ev.Mods == 0:
							insertMention(mentions[s.mentionIndex])
							return true
						case ev.Key == ui.KeyEscape:
							s.mentionClosed = start + 1
							return true
						}
					}
					if ev.Key == ui.KeyEscape && s.reply != nil {
						s.reply = nil
						return true
					}
					if ev.Key == ui.KeyV && ev.Mods == ui.Cmd {
						return m.pasteImage(s)
					}
					if ev.Key != ui.KeyEnter {
						return false
					}
					switch {
					case ev.Mods == ui.Alt:
						// Alt-Enter starts a line, as Option-Return does on the Mac.
						return false
					case sendOnReturn && ev.Mods == 0:
						send()
						return true
					case !sendOnReturn && ev.Mods == ui.Cmd:
						send()
						return true
					}
					return false
				})
				if s.focus {
					input.Focus()
					s.focus = false
				}
				if s.caret >= 0 && s.caret <= len([]rune(s.draft)) && s.caret != s.caretAt {
					input.SetTextSelection(s.caret, s.caret)
				}
				s.caret = -1
				caret, _ := input.TextSelection()
				if caret != s.caretAt {
					s.caretAt = caret
					c.AnimationFrame()
				}
				if input.Changed() {
					// The text changed by hand: picks whose name went keep no hold.
					c.AnimationFrame()
				}
			})
			trailing := ui.Row(c.Key("trailing")).RowStart(controlsRow).ColumnStart(3).Gap(4)
			trailing.Children(func() {
				if responding {
					stop := ui.ButtonBase(c).Size(28, 28).Radius(14).Center().Label(L("Stop responding")).Tooltip(L("Stop responding (%@)", shortcutText("CmdOrCtrl+.")))
					if hasContent {
						stop.Background(p.ComposerControl).TextColor(p.Label)
					} else {
						stop.Background(p.ComposerPrimary).TextColor(p.ComposerPrimaryContent)
					}
					stop.Children(func() { symbol(c, "stop.fill", 11, 2.4) })
					if stop.Clicked() {
						chatS.stopping = true
						store.StopResponding(chatID)
					}
				}
				if hasContent || !responding {
					tooltip := L("Send (%@)", shortcutText("CmdOrCtrl+Enter"))
					if sendOnReturn {
						tooltip = L("Send (Enter) · Shift-Enter for a new line")
						if runtime.GOOS == "darwin" {
							tooltip = L("Send (Return) · Shift-Return for a new line")
						}
					}
					if len(members) > 1 {
						tooltip += L(" · @ to mention")
					}
					button := ui.ButtonBase(c).Size(28, 28).Radius(14).Center().Background(p.ComposerPrimary).TextColor(p.ComposerPrimaryContent).
						Label(L("Send")).Tooltip(tooltip)
					if !hasContent {
						// Faded as one piece: a pale disc around an arrow as crisp as the enabled one's,
						// where an element's opacity would fade the arrow into the disc.
						button.Background(p.ComposerPrimary.Alpha(0.25)).TextColor(p.ComposerField.Alpha(1))
					}
					button.Children(func() { symbol(c, "arrow.up", 15, 2.6) })
					if button.Clicked() && hasContent {
						send()
					}
				}
			})
		})
		// A click on the pill around the text puts the keyboard in it.
		if field.Clicked() {
			s.focus = true
		}
	})
	if len(mentions) > 0 {
		m.mentionPanel(c, field, mentions, s, insertMention)
	}
	return outer
}

// mentionPanel is the `@` menu: above the composer, five rows showing and the rest scrolling.
func (m *mainWindow) mentionPanel(c *ui.Context, field ui.Element, bots []*model.Bot, s *composerState, pick func(*model.Bot)) {
	p := colors(c)
	ui.Overlay(c, func() {
		height := float32(min(len(bots), mentionRows)*mentionRowHeight + 10)
		panel := ui.Scroll(c).Width(268).Height(height).Padding(5).Radius(10).Background(p.Popover).Shadow(0, 10, 40, 0, p.Shadow).Border(0.5, p.ShadowEdge).
			AttachTo(field, ui.AnchorTopLeft, ui.AnchorBottomLeft).Margin(0, 0, 8, 40)
		panel.Children(func() {
			for i, bot := range bots {
				row := ui.Row(c.Key(bot.ID)).Height(mentionRowHeight).Gap(8).Padding(0, 8).Radius(7).Cursor(ui.CursorPointer)
				highlighted := i == s.mentionIndex
				if row.Hovered() {
					s.mentionIndex, highlighted = i, true
				}
				if highlighted {
					row.Background(p.Accent).TextColor(p.White)
					row.ScrollIntoView()
				}
				detail := model.ProviderName(bot.Provider, store.Providers)
				if device := store.Device(bot.RunnerID); device != nil {
					detail = L("on %@", device.Name)
				}
				row.Children(func() {
					avatar(c, botAvatar(bot), 22, false)
					ui.Column(c).MinWidth(0).Shrink(1).Children(func() {
						ui.Text(c, bot.Name).FontSize(13).FontWeight(500).SingleLine()
						d := ui.Text(c, detail).FontSize(11).TextColor(p.Label2).SingleLine()
						if highlighted {
							d.TextColor(ui.RGBA(255, 255, 255, 0.8))
						}
					})
				})
				if row.Pressed() {
					pick(bot)
				}
			}
		})
	})
}

type mentionRun struct {
	start, end int
	bot        *model.Bot
}

// mentionRuns are the `@Name` runs of a draft for the bots it can address, in runes, whatever
// their case, as the Mac's composer tints them; where two names start alike, the longer wins.
func mentionRuns(runes []rune, bots []*model.Bot) []mentionRun {
	lower := func(rs []rune) []rune {
		out := make([]rune, len(rs))
		for i, r := range rs {
			out[i] = unicode.ToLower(r)
		}
		return out
	}
	text := lower(runes)
	names := make([][]rune, len(bots))
	for i, bot := range bots {
		names[i] = lower([]rune(bot.Name))
	}
	var runs []mentionRun
	for i := 0; i < len(text); i++ {
		if text[i] != '@' {
			continue
		}
		best := -1
		for j, name := range names {
			n := len(name)
			if n > 0 && i+1+n <= len(text) && slices.Equal(text[i+1:i+1+n], name) && (best < 0 || n > len(names[best])) {
				best = j
			}
		}
		if best >= 0 {
			end := i + 1 + len(names[best])
			runs = append(runs, mentionRun{start: i, end: end, bot: bots[best]})
			i = end - 1
		}
	}
	return runs
}
