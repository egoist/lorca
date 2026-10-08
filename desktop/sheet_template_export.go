package main

import (
	"path/filepath"
	"regexp"
	"slices"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Bot templates, after the macOS app's TemplateExportViewController and TemplateItemViews.
// Exporting: the user picks a bot's profile, routines, plugins, and memories, sees each as the file
// will hold it, and saves the file where they like. The CLI writes the file; saving publishes
// nothing.

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
func templateItemRow(c *ui.Context, k *card, item templateItem, selectable, selected bool) bool {
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
	return clicked
}

type templateSection struct {
	title     string
	items     []templateItem
	accessory func()
}

// templateList is a card per kind that has items, scrolling past `height`. It reports the item
// clicked, as "<section>:<id>".
func templateList(c *ui.Context, height float32, sections []templateSection, selectable bool, selected func(section, id string) bool) string {
	clicked := ""
	ui.Scroll(c).MaxHeight(height).Children(func() {
		ui.Column(c).Gap(16).Children(func() {
			for _, sec := range sections {
				if len(sec.items) == 0 {
					continue
				}
				section(c.Key(sec.title), sec.title, sectionCaption, sec.accessory, func(k *card) {
					for _, item := range sec.items {
						if templateItemRow(c, k, item, selectable, selectable && selected(sec.title, item.id)) {
							clicked = sec.title + ":" + item.id
						}
					}
				})
			}
		})
	})
	return clicked
}

type templateExportState struct {
	botID, name string
	contents    *model.TemplateContents
	// picked is what goes in the file, by section title, in the bot's order.
	picked map[string][]string
	status string
	failed bool
	busy   bool
}

func (w *appWindow) presentTemplateExport(botID string) {
	bot := store.Bot(botID)
	if bot == nil {
		return
	}
	st := &templateExportState{botID: botID, name: bot.Name, status: L("Loading…"), picked: map[string][]string{}}
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
		// The profile is what makes a template a bot; the rest waits to be picked.
		st.picked[L("Profile")] = []string{"profile"}
	})
}

// sections are the bot's pieces by kind; memories come last, the longest list and most personal.
func (st *templateExportState) sections() []templateSection {
	contents := st.contents
	profile := templateSection{title: L("Profile"), items: []templateItem{templateProfile(contents.Profile.Content, contents.Profile.Flags)}}
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

func (st *templateExportState) toggle(sections []templateSection, title, id string) {
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

func (st *templateExportState) selection() model.TemplateSelection {
	return model.TemplateSelection{
		Profile:        len(st.picked[L("Profile")]) > 0,
		SkillIDs:       st.picked[L("Skills")],
		MemoryIDs:      st.picked[L("Memories")],
		RoutineIDs:     st.picked[L("Routines")],
		RequirementIDs: st.picked[L("Plugins")],
	}
}

func (st *templateExportState) view(c *ui.Context, w *appWindow, s *sheet) {
	p := colors(c)
	result := sheetFrame(c, sheetOptions{
		Title:           L("Export “%@”", st.name),
		Subtitle:        L("Pick what goes in the template. Keys, sign-ins, and chats never do."),
		Width:           480,
		Confirm:         L("Export…"),
		ConfirmDisabled: st.busy || st.contents == nil || st.selection().Empty(),
	}, func() {
		if st.contents != nil {
			sections := st.sections()
			// Every memory at once, or none once all are picked: a bot's memory runs to dozens of lines.
			if memories := sections[4].items; len(memories) >= 3 {
				sections[4].accessory = func() {
					all := len(st.picked[L("Memories")]) == len(memories)
					title := L("Select All")
					if all {
						title = L("Deselect All")
					}
					b := ui.ButtonBase(c.Key("all-memories")).Shrink(0).Padding(0, 6).FontSize(11).TextColor(p.Label2).Label(title).Cursor(ui.CursorPointer)
					b.Children(func() { ui.Text(c, title).SingleLine() })
					if b.Clicked() && !st.busy {
						st.picked[L("Memories")] = nil
						if !all {
							for _, item := range memories {
								st.picked[L("Memories")] = append(st.picked[L("Memories")], item.id)
							}
						}
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
	case result.Cancelled && !st.busy:
		s.dismiss()
	case result.Confirmed && !st.busy && st.contents != nil && !st.selection().Empty():
		st.export(w, s)
	}
}

// export builds the file's contents first, so the CLI's checks speak before the Save dialog opens,
// then writes what was shown: the CLI refuses the save if the contents changed since.
func (st *templateExportState) export(w *appWindow, s *sheet) {
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
