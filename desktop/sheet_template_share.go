package main

import (
	"path/filepath"
	"regexp"
	"slices"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Bot templates, after the macOS app's TemplateShareViewController and TemplateItemViews.
// Sharing: the user picks what goes in a bot's template besides its profile (routines, plugins,
// memories), sees each as the template will hold it, and shares a link, which the CLI puts on the
// relay encrypted with a key only the link carries, or saves a file. A bot shared before updates
// its link, which keeps its address.

// templateItem is a piece of a template as the sheets list it.
type templateItem struct {
	id, title, detail string
	// flags are what to look at before sharing it: "email", "phone", "path", "link", "credential".
	flags []string
	// look is the profile's avatar; plugin is a plugin's id, for its logo.
	look   *avatarContent
	plugin string
	// lines is how many lines the title may take: a memory's three, a name's one.
	lines int
}

var markdownMark = regexp.MustCompile(`^(#+|[-*+]|\d+\.)\s+`)

// templateMemory reads as its first line, without Markdown's list or heading marks, over the rest.
func templateMemory(id, text string, flags []string) templateItem {
	var lines []string
	for _, line := range strings.Split(text, "\n") {
		if line = markdownMark.ReplaceAllString(strings.TrimSpace(line), ""); line != "" {
			lines = append(lines, line)
		}
	}
	item := templateItem{id: id, title: text, flags: flags, lines: 3}
	if len(lines) > 0 {
		item.title, item.detail = lines[0], strings.Join(lines[1:], " ")
	}
	return item
}

func templateProfile(profile model.TemplateProfile, flags []string) templateItem {
	look := avatarContent{Kind: avatarBot, SymbolName: firstNonEmpty(profile.SymbolName, "sparkles"), Accent: model.Accent(firstNonEmpty(profile.Accent, "indigo"))}
	return templateItem{id: "profile", title: profile.Name, detail: profile.Description, flags: flags, look: &look, lines: 1}
}

func templateRoutine(id string, routine model.PortableRoutine, scheduleText string, flags []string) templateItem {
	schedule := routine.Schedule
	if scheduleText != "" {
		schedule = model.Schedule(scheduleText)
	}
	detail := schedule
	if routine.Prompt != "" {
		detail += " · " + routine.Prompt
	}
	return templateItem{id: id, title: routine.Name, detail: detail, flags: flags, lines: 1}
}

// templateFlags is an item's flags in a word or two, and whether one is personal, which tints it.
func templateFlags(flags []string) (string, bool) {
	var words []string
	personal := false
	for _, flag := range flags {
		switch flag {
		case "email":
			words = append(words, L("Email address"))
		case "phone":
			words = append(words, L("Phone number"))
		case "path":
			words = append(words, L("File path"))
		case "link":
			words = append(words, L("Link"))
		case "credential":
			words = append(words, L("Key removed"))
			continue
		default:
			continue
		}
		personal = true
	}
	return strings.Join(words[:min(2, len(words))], ", "), personal
}

// templateItemRow is a piece of a template, after pickerBotRow: a check when it can be picked, the
// bot's avatar or a plugin's logo, its text, and what to look at before sharing it at the end. It
// reports a click while it can be picked.
func templateItemRow(c *ui.Context, k *card, item templateItem, selectable, selected bool) (ui.Element, bool) {
	p := colors(c)
	label := strings.TrimSpace(item.title + "\n" + item.detail)
	r := k.row(rowBox(c.Key(item.id)).MinHeight(36).Padding(9, 12).AlignItems(ui.Start).Label(label).Tooltip(label))
	clicked := false
	if selectable {
		r.Role(ui.RoleCheckBox).Checked(selected).Cursor(ui.CursorPointer)
		if r.Hovered() {
			r.Background(p.RowHover)
		}
		clicked = r.Clicked()
	}
	r.Children(func() {
		// The check sits on the image's middle, or the first line's.
		if selectable || item.look != nil || item.plugin != "" {
			lead := ui.Row(c).Gap(10).Height(16)
			if item.look != nil {
				lead.Height(28)
			} else if item.plugin != "" {
				lead.Height(18)
			}
			lead.Children(func() {
				if selectable {
					check, tint := "circle", p.Label3
					if selected {
						check, tint = "checkmark.circle.fill", p.Accent
					}
					ui.Row(c).TextColor(tint).Children(func() { symbol(c, check, 16, 1.8) })
				}
				switch {
				case item.look != nil:
					avatar(c, *item.look, 28, false)
				case item.plugin != "":
					ui.Row(c).TextColor(p.Label2).Children(func() { pluginTile(c, item.plugin, "puzzlepiece.extension", 18) })
				}
			})
		}
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(2).Children(func() {
			title := ui.Text(c, item.title).FontSize(13).LineHeight(1.25)
			if item.lines > 1 {
				title.MaxLines(item.lines)
			} else {
				title.FontWeight(500).SingleLine()
			}
			if item.detail != "" {
				ui.Text(c, item.detail).FontSize(textCaption).LineHeight(1.35).TextColor(p.Label2).SingleLine()
			}
		})
		if text, personal := templateFlags(item.flags); text != "" {
			tint := p.Label2
			if personal {
				tint = p.Orange
			}
			ui.Text(c, text).Margin(2, 0, 0, 0).FontSize(10).FontWeight(500).TextColor(tint).SingleLine()
		}
	})
	return r, clicked
}

type templateSection struct {
	title string
	items []templateItem
	// fixed is a section that is always in the file, so its rows have no check.
	fixed     bool
	accessory func()
}

// templateLongList is how many items a section has before its header offers Select All.
const templateLongList = 6

// templateList is a card per kind that has items, scrolling past `height`. It reports the item
// clicked, as "<section>:<id>". Taller than `height`, it ends where a row or a card ends rather than
// through one, and a hairline marks the edge while there is more below.
func templateList(c *ui.Context, height float32, sections []templateSection, selectable bool, selected func(section, id string) bool) string {
	p := colors(c)
	clicked := ""
	box := ui.Scroll(c)
	scroll := ui.Local(box, "scroll", func() ui.ScrollState { return ui.ScrollState{} })
	box.TrackScroll(scroll)
	// The previous frame's boxes: where each row and card ends below the cards' top.
	var top, full float32
	var ends []float32
	box.Children(func() {
		cards := ui.Column(c.Key("cards")).Gap(16)
		top, full = cards.Bounds().Y, cards.Bounds().H
		cards.Children(func() {
			for _, sec := range sections {
				if len(sec.items) == 0 {
					continue
				}
				var last ui.Element
				el := section(c.Key(sec.title), sec.title, sectionCaption, sec.accessory, func(k *card) {
					for _, item := range sec.items {
						can := selectable && !sec.fixed
						row, picked := templateItemRow(c, k, item, can, can && selected(sec.title, item.id))
						if picked {
							clicked = sec.title + ":" + item.id
						}
						if last.Bounds().H > 0 {
							ends = append(ends, last.Bounds().Y+last.Bounds().H-top)
						}
						last = row
					}
				})
				// A card's end leaves half the gap to the next, so the edge clears its corners.
				ends = append(ends, el.Bounds().Y+el.Bounds().H-top+8)
			}
		})
	})
	end := float32(0)
	for _, bottom := range ends {
		if bottom <= height && bottom > end {
			end = bottom
		}
	}
	if full > height && end > 0 {
		box.Height(end)
	} else {
		box.MaxHeight(height)
	}
	box.DrawOver(func(painter *ui.Painter, r ui.Rect) {
		if scroll.Y < scroll.MaxY-0.5 {
			painter.Fill(ui.Rect{X: r.X, Y: r.Y + r.H - 1, W: r.W, H: 1}, p.Separator, 0)
		}
	})
	return clicked
}

// selectAll is a section header's Select All, or Deselect All once every item is picked.
func selectAll(c *ui.Context, items []templateItem, picked *[]string, disabled bool) {
	p := colors(c)
	all := len(*picked) == len(items)
	title := L("Select All")
	if all {
		title = L("Deselect All")
	}
	b := ui.ButtonBase(c).Shrink(0).Padding(0, 6).FontSize(11).TextColor(p.Label2).Label(title).Cursor(ui.CursorPointer)
	b.Children(func() { ui.Text(c, title).SingleLine() })
	if b.Clicked() && !disabled {
		*picked = nil
		if !all {
			for _, item := range items {
				*picked = append(*picked, item.id)
			}
		}
	}
}

type templateShareState struct {
	botID, name string
	contents    *model.TemplateContents
	// link is the link the bot was shared as, which Update Link replaces what is behind.
	link *model.SharedLink
	// picked is what goes in the template, by section title, in the bot's order.
	picked map[string][]string
	status string
	failed bool
	busy   bool
	// shared is the link once it is out, which the sheet shows; copiedAt is when it was copied.
	shared   string
	copiedAt time.Time
}

func (w *appWindow) presentTemplateShare(botID string) {
	bot := store.Bot(botID)
	if bot == nil {
		return
	}
	st := &templateShareState{botID: botID, name: bot.Name, status: L("Loading…"), picked: map[string][]string{}}
	if link := store.SharedLinkFor(botID); link != nil {
		copied := *link
		st.link = &copied
	}
	s := w.present(func(c *ui.Context, s *sheet) { st.view(c, w, s) }, nil)
	store.TemplateContents(botID, func(contents model.TemplateContents, err error) {
		if s.window == nil {
			return
		}
		if err != nil {
			st.status, st.failed = model.ErrorText(err), true
			return
		}
		st.contents, st.status = &contents, ""
		// An update starts from what the link holds, less what the bot no longer has.
		if st.link != nil {
			held := map[string][]string{L("Skills"): st.link.Selection.SkillIDs, L("Routines"): st.link.Selection.RoutineIDs,
				L("Plugins"): st.link.Selection.RequirementIDs, L("Memories"): st.link.Selection.MemoryIDs}
			for _, sec := range st.sections() {
				for _, item := range sec.items {
					if slices.Contains(held[sec.title], item.id) {
						st.picked[sec.title] = append(st.picked[sec.title], item.id)
					}
				}
			}
		}
	})
}

// sections are the bot's pieces by kind; memories come last, the longest list and most personal.
func (st *templateShareState) sections() []templateSection {
	contents := st.contents
	// The profile is what makes a template a bot, so it is always in the file.
	profile := templateSection{title: L("Profile"), items: []templateItem{templateProfile(contents.Profile.Content, contents.Profile.Flags)}, fixed: true}
	skills := templateSection{title: L("Skills")}
	for _, item := range contents.Skills {
		skills.items = append(skills.items, templateItem{id: item.ID, title: item.Content.Name, detail: item.Content.Description, flags: item.Flags, lines: 1})
	}
	routines := templateSection{title: L("Routines")}
	for _, item := range contents.Routines {
		text := ""
		if routine := store.Routine(item.ID); routine != nil {
			text = routine.ScheduleText
		}
		routines.items = append(routines.items, templateRoutine(item.ID, item.Content, text, item.Flags))
	}
	plugins := templateSection{title: L("Plugins")}
	for _, service := range contents.Requirements {
		plugins.items = append(plugins.items, templateItem{id: service.ServiceID, title: service.Name, plugin: service.ServiceID, lines: 1})
	}
	memories := templateSection{title: L("Memories")}
	for _, item := range contents.Memories {
		memories.items = append(memories.items, templateMemory(item.ID, item.Content, item.Flags))
	}
	return []templateSection{profile, skills, routines, plugins, memories}
}

func (st *templateShareState) toggle(sections []templateSection, title, id string) {
	ids := st.picked[title]
	if slices.Contains(ids, id) {
		ids = slices.DeleteFunc(slices.Clone(ids), func(each string) bool { return each == id })
	} else {
		ids = append(slices.Clone(ids), id)
	}
	// The file lists things in the bot's order, whatever order they were picked in.
	for _, sec := range sections {
		if sec.title == title {
			var ordered []string
			for _, item := range sec.items {
				if slices.Contains(ids, item.id) {
					ordered = append(ordered, item.id)
				}
			}
			st.picked[title] = ordered
		}
	}
	if st.failed {
		st.status, st.failed = "", false
	}
}

func (st *templateShareState) selection() model.TemplateSelection {
	return model.TemplateSelection{
		Profile:        true,
		SkillIDs:       st.picked[L("Skills")],
		MemoryIDs:      st.picked[L("Memories")],
		RoutineIDs:     st.picked[L("Routines")],
		RequirementIDs: st.picked[L("Plugins")],
	}
}

func (st *templateShareState) view(c *ui.Context, w *appWindow, s *sheet) {
	p := colors(c)
	options := sheetOptions{
		Title:           L("Share “%@”", st.name),
		Subtitle:        L("Others get a copy of what you pick. Keys, sign-ins, and chats stay."),
		Width:           480,
		Confirm:         L("Share Link"),
		ConfirmDisabled: st.busy || st.contents == nil,
		Leading: func() {
			if pushButton(c, L("Save as File…"), pushOptions{Disabled: st.busy || st.contents == nil}).Clicked() {
				st.export(w, s)
			}
		},
	}
	switch {
	case st.shared != "":
		options.Subtitle = L("Anyone with this link can add their own copy of %@. Revoke it in Settings › Shared Links.", st.name)
		if st.link != nil {
			options.Subtitle = L("The link now holds what you picked. Anyone who opens it gets this version.")
		}
		options.Confirm, options.NoCancel, options.ConfirmDisabled, options.Leading = L("Done"), true, false, nil
	case st.link != nil:
		options.Subtitle = L("Update the link with what you pick now. Its address stays the same.")
		options.Confirm = L("Update Link")
	}
	result := sheetFrame(c, options, func() {
		if st.shared != "" {
			linkBox(c.Key("shared-link"), st.shared, &st.copiedAt)
			return
		}
		if st.link != nil {
			linkBox(c.Key("shared-link"), st.link.URL, &st.copiedAt)
		}
		if st.contents != nil {
			sections := st.sections()
			// A long list picks all at once, whichever kind it is.
			for i := range sections {
				sec := sections[i]
				if len(sec.items) >= templateLongList && !sec.fixed {
					sections[i].accessory = func() {
						picked := st.picked[sec.title]
						selectAll(c.Key("all:"+sec.title), sec.items, &picked, st.busy)
						st.picked[sec.title] = picked
					}
				}
			}
			clicked := templateList(c.Key("template-list"), 380, sections, true, func(title, id string) bool { return slices.Contains(st.picked[title], id) })
			if title, id, ok := strings.Cut(clicked, ":"); ok && !st.busy {
				st.toggle(sections, title, id)
			}
		}
		if st.status != "" {
			tint := p.Label2
			if st.failed {
				tint = p.Red
			}
			ui.Text(c, st.status).FontSize(11.5).LineHeight(1.4).TextColor(tint)
		}
	})
	switch {
	case result.Confirmed && st.shared != "":
		s.dismiss()
	case result.Cancelled && !st.busy:
		s.dismiss()
	case result.Confirmed && !st.busy && st.contents != nil:
		st.share(s)
	}
}

// share builds the template first, so the CLI's checks speak before anything leaves, then shares
// what was shown: the CLI refuses it if the contents changed since. The link is copied at once.
func (st *templateShareState) share(s *sheet) {
	selection := st.selection().Clone()
	st.busy = true
	fail := func(err error) {
		st.busy, st.failed, st.status = false, true, model.ErrorText(err)
	}
	store.PreviewTemplateExport(st.botID, selection, func(preview model.TemplatePreview, err error) {
		if s.window == nil {
			return
		}
		if err != nil {
			fail(err)
			return
		}
		options := model.TemplateShareOptions{BotID: st.botID, Selection: selection, ExpectedDigest: preview.Digest, Reviewed: true}
		if st.link != nil {
			options.LinkID = st.link.ID
		}
		store.ShareTemplate(options, func(link model.SharedLink, err error) {
			if s.window == nil {
				return
			}
			if err != nil {
				fail(err)
				return
			}
			st.busy, st.shared = false, link.URL
			copyText(link.URL)
			st.copiedAt = time.Now()
		})
	})
}

// export builds the file's contents first, so the CLI's checks speak before the Save dialog opens,
// then writes what was shown: the CLI refuses the save if the contents changed since.
func (st *templateShareState) export(w *appWindow, s *sheet) {
	selection := st.selection().Clone()
	st.busy = true
	fail := func(err error) {
		st.busy, st.failed, st.status = false, true, model.ErrorText(err)
	}
	store.PreviewTemplateExport(st.botID, selection, func(preview model.TemplatePreview, err error) {
		if s.window == nil {
			return
		}
		if err != nil {
			fail(err)
			return
		}
		chooseTemplateDestination(w.win, st.name, func(path string, err error) {
			if s.window == nil {
				return
			}
			if err != nil || path == "" {
				st.busy = false
				if err != nil {
					fail(err)
				}
				return
			}
			write := func(path string, overwrite bool) {
				options := model.TemplateExportOptions{BotID: st.botID, Selection: selection, Path: path, ExpectedDigest: preview.Digest, Reviewed: true, Overwrite: overwrite}
				store.ExportTemplate(options, func(err error) {
					if s.window == nil {
						return
					}
					if err != nil {
						fail(err)
						return
					}
					s.dismiss()
				})
			}
			path, exists, confirm := templateDestination(path)
			if !confirm {
				write(path, exists)
				return
			}
			// The dialog asked about the name typed; the extension made it another file that exists.
			w.showAlert(alertOptions{Message: L("“%@” already exists. Do you want to replace it?", filepath.Base(path)), Buttons: []alertButton{{Title: L("Replace")}, {Title: L("Cancel")}}}, func(index int) {
				if s.window == nil {
					return
				}
				if index == 0 {
					write(path, true)
				} else {
					st.busy = false
				}
			})
		})
	})
}

// linkBox is a shared link's address on the code fill with a button that copies it, as the
// pairing sheet shows its code. `copiedAt` is when it was last copied, by the button or by a share
// that copied it at once; the button says Copied for a moment after.
func linkBox(c *ui.Context, url string, copiedAt *time.Time) {
	p := colors(c)
	ui.Row(c).Gap(8).Padding(4, 6, 4, 10).Radius(8).Background(p.Code).Children(func() {
		ui.Text(c, url).Grow(1).Shrink(1).MinWidth(0).FontSize(12).SingleLine().Selectable().Tooltip(url)
		copied := !copiedAt.IsZero() && c.Now().Sub(*copiedAt) < 1500*time.Millisecond
		if copied {
			c.After(1500*time.Millisecond - c.Now().Sub(*copiedAt))
		}
		b := ui.ButtonBase(c).Height(24).MinWidth(24).Gap(5).Padding(0, 4).Justify(ui.Center).Radius(5).TextColor(p.Label2).Label(L("Copy Link"))
		if b.Hovered() {
			b.Background(p.Hover)
		}
		if b.Clicked() {
			copyText(url)
			*copiedAt = c.Now()
		}
		if copied {
			b.TextColor(p.Green)
		}
		b.Children(func() {
			if copied {
				symbol(c, "checkmark", 13, 2)
				ui.Text(c, L("Copied")).SingleLine()
			} else {
				symbol(c, "doc.on.doc", 13, 2)
			}
		})
	})
}
