package main

import (
	"slices"
	"sort"
	"strings"
	"time"
	"unicode"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
	"golang.org/x/text/runes"
	"golang.org/x/text/transform"
	"golang.org/x/text/unicode/norm"
)

// The command palette, after the macOS app's CommandPalette and PaletteIndex: a search field over
// the menu bar's commands, the chats, and the settings, floating over the main window. The field
// keeps the keyboard; the arrows move the list's selection, Return runs it, and Escape or a click
// elsewhere closes it. A query also searches the chats' messages through the CLI.

type paletteItem struct {
	symbol string
	// bots is a chat's members, whose avatars stand in for a symbol.
	bots     []*model.Bot
	title    string
	subtitle string
	// shortcut is the menu item's, as the menu spells it.
	shortcut string
	// keywords find the item besides its title.
	keywords []string
	// searchOnly is left out of the list until a query finds it.
	searchOnly bool
	chatID     string
	run        func()
}

type paletteSection struct {
	title string
	items []paletteItem
}

type paletteRow struct {
	header string
	item   *paletteItem
}

// paletteCommands are the menu bar's commands worth offering, in the macOS app's order. Each takes
// its title, shortcut, and enabled state from the command table, so it reads and behaves as it does
// in the menu.
var paletteCommands = []struct{ id, symbol, keywords string }{
	{"newBot", "plus.message", "create add"},
	{"newGroupChat", "person.2", "create room"},
	{"pairDevice", "qrcode", "phone link runner"},
	{"addBot", "person.badge.plus", "invite member group"},
	{"renameChat", "pencil", "title name"},
	{"pinChat", "pin", "unpin favorite"},
	{"newSkill", "book.closed", "playbook instructions workflow"},
	{"stopResponding", "stop.circle", "cancel interrupt"},
	{"runInBackground", "terminal", "detach server task"},
	{"scrollToLatest", "arrow.down.to.line", "bottom newest jump"},
	{"toggleSidebar", "sidebar.leading", "show hide"},
	{"toggleInspector", "sidebar.trailing", "show hide details"},
	{"fullScreen", "arrow.up.left.and.arrow.down.right", "fullscreen"},
	{"settings", "gearshape", "preferences"},
	{"checkForUpdates", "arrow.triangle.2.circlepath", "upgrade version"},
	{"deleteChat", "trash", "remove"},
	{"help", "questionmark.circle", "about"},
}

type paletteState struct {
	open     bool
	query    string
	sections []paletteSection
	results  model.WireSearchResults
	// searching is a message search on its way for the query.
	searching  bool
	generation int
	// picked is the row the arrows or the pointer picked; -1 follows the list's first item.
	picked int
	// reveal scrolls the picked row into view, once.
	reveal bool
	scroll ui.ScrollState
	// pointerX and pointerY are where the pointer was over the list in the last frame.
	pointerX, pointerY float32
	m                  *mainWindow
}

func (s *paletteState) toggle() {
	if s.open {
		s.close()
	} else {
		s.show()
	}
}

func (s *paletteState) show() {
	m := app.main
	if m == nil || m.hasSheet() {
		beep()
		return
	}
	// Read when the palette opens, as the Mac palette reads the menu before it takes the keyboard.
	*s = paletteState{open: true, picked: -1, m: m, generation: s.generation + 1}
	s.sections = []paletteSection{
		{L("Actions"), paletteActions(m)},
		{L("Chats"), paletteChats(m)},
		{L("Settings"), paletteSettings(m)},
	}
	m.invalidate()
}

func (s *paletteState) close() {
	if !s.open {
		return
	}
	s.open = false
	s.generation++
	if s.m != nil {
		s.m.invalidate()
	}
}

func paletteActions(m *mainWindow) []paletteItem {
	var items []paletteItem
	for _, entry := range paletteCommands {
		cmd := commandByID(entry.id)
		if cmd == nil || (entry.id == "checkForUpdates" && !updatesEnabled()) || !cmd.isEnabled() {
			continue
		}
		id := entry.id
		shortcut := ""
		if cmd.accelerator != "" {
			shortcut = shortcutText(cmd.accelerator)
		}
		items = append(items, paletteItem{
			symbol:   entry.symbol,
			title:    cmd.title(),
			shortcut: shortcut,
			keywords: []string{entry.keywords},
			run: func() {
				// Full screen is the system's own menu item; the palette asks the window for it.
				if id == "fullScreen" {
					m.win.ToggleFullScreen()
					return
				}
				runCommand(id)
			},
		})
	}
	return items
}

func paletteChats(m *mainWindow) []paletteItem {
	if !store.IsConnected {
		return nil
	}
	var items []paletteItem
	for _, chat := range store.Chats {
		bots := store.BotsIn(chat)
		keywords := []string{store.Preview(chat)}
		for _, bot := range bots {
			keywords = append(keywords, bot.Name)
		}
		id := chat.ID
		items = append(items, paletteItem{bots: bots, title: store.Title(chat), subtitle: store.Subtitle(chat), keywords: keywords, chatID: id, run: func() { m.open(id) }})
	}
	return items
}

// paletteSettings are the panes, then the settings on them, which only a query brings up.
func paletteSettings(m *mainWindow) []paletteItem {
	device := store.Device(m.settingsDeviceID)
	var panes, entries []paletteItem
	for _, pane := range model.SettingsPanes {
		pane := pane
		panes = append(panes, paletteItem{symbol: pane.Symbol(), title: pane.Title(), keywords: []string{L("Settings")}, run: func() { m.showSettings(pane) }})
		for _, setting := range entriesIn(pane, device) {
			setting := setting
			entries = append(entries, paletteItem{symbol: pane.Symbol(), title: setting.title, subtitle: pane.Title(), keywords: setting.keywords, searchOnly: true, run: func() { m.reveal(setting) }})
		}
	}
	return append(panes, entries...)
}

func paletteSearchSections(m *mainWindow, results model.WireSearchResults) []paletteSection {
	hit := func(chatID, snippet string) *paletteItem {
		chat := store.Chat(chatID)
		if chat == nil {
			return nil
		}
		id := chat.ID
		return &paletteItem{bots: store.BotsIn(chat), title: store.Title(chat), subtitle: snippet, searchOnly: true, chatID: id, run: func() { m.open(id) }}
	}
	var chats, messages []paletteItem
	for _, each := range results.Chats {
		if item := hit(each.ChatID, each.Snippet); item != nil {
			chats = append(chats, *item)
		}
	}
	for _, each := range results.Messages {
		if item := hit(each.ChatID, each.Snippet); item != nil {
			messages = append(messages, *item)
		}
	}
	var out []paletteSection
	if len(chats) > 0 {
		out = append(out, paletteSection{L("Chats"), chats})
	}
	if len(messages) > 0 {
		out = append(out, paletteSection{L("Messages"), messages})
	}
	return out
}

// MARK: - Search

var paletteFolding = transform.Chain(norm.NFKC, norm.NFD, runes.Remove(runes.In(unicode.Mn)), norm.NFC)

// paletteFold folds case, accents, and full-width forms away, as the Mac palette compares.
func paletteFold(text string) string {
	folded, _, err := transform.String(paletteFolding, text)
	if err != nil {
		folded = text
	}
	return strings.ToLower(folded)
}

func isSubsequence(needle, haystack string) bool {
	at := 0
	for _, r := range needle {
		found := strings.IndexRune(haystack[at:], r)
		if found < 0 {
			return false
		}
		at += found + len(string(r))
	}
	return true
}

func splitWords(text string) []string {
	return strings.FieldsFunc(text, func(r rune) bool { return !unicode.IsLetter(r) && !unicode.IsNumber(r) })
}

// paletteScore ranks an item for a query: a title that starts with it beats a word that does, then
// a substring, then the query's letters as the words' initials ("ngc" finds New Group Chat).
// Keywords and subtitles rank last. -1 is no match.
func paletteScore(item paletteItem, query string) int {
	title := paletteFold(item.title)
	if strings.HasPrefix(title, query) {
		return 100
	}
	words := splitWords(title)
	for _, word := range words {
		if strings.HasPrefix(word, query) {
			return 80
		}
	}
	if strings.Contains(title, query) {
		return 60
	}
	var initials strings.Builder
	for _, word := range words {
		r := []rune(word)
		initials.WriteRune(r[0])
	}
	if isSubsequence(query, initials.String()) {
		return 50
	}
	for _, text := range append([]string{item.subtitle}, item.keywords...) {
		if strings.Contains(paletteFold(text), query) {
			return 40
		}
	}
	if len([]rune(query)) > 1 && isSubsequence(query, title) {
		return 20
	}
	return -1
}

// paletteFilter narrows the sections to a query, best match first. With no query, everything but
// the items only a search brings up.
func paletteFilter(sections []paletteSection, raw string) []paletteSection {
	query := paletteFold(strings.TrimSpace(raw))
	var out []paletteSection
	for _, section := range sections {
		var items []paletteItem
		if query == "" {
			for _, item := range section.items {
				if !item.searchOnly {
					items = append(items, item)
				}
			}
		} else {
			type scored struct {
				item  paletteItem
				value int
			}
			var found []scored
			for _, item := range section.items {
				if value := paletteScore(item, query); value >= 0 {
					found = append(found, scored{item, value})
				}
			}
			sort.SliceStable(found, func(a, b int) bool { return found[a].value > found[b].value })
			for _, each := range found {
				items = append(items, each.item)
			}
		}
		if len(items) > 0 {
			out = append(out, paletteSection{section.title, items})
		}
	}
	return out
}

// paletteMerge adds the message search's sections: chats it found join the Chats section unless
// listed already.
func paletteMerge(incoming, sections []paletteSection) []paletteSection {
	merged := slices.Clone(sections)
	for _, section := range incoming {
		index := -1
		if section.title == L("Chats") {
			index = slices.IndexFunc(merged, func(s paletteSection) bool { return s.title == section.title })
		}
		if index < 0 {
			merged = append(merged, section)
			continue
		}
		existing := merged[index]
		items := slices.Clone(existing.items)
		for _, item := range section.items {
			if item.chatID == "" || !slices.ContainsFunc(items, func(known paletteItem) bool { return known.chatID == item.chatID }) {
				items = append(items, item)
			}
		}
		merged[index] = paletteSection{existing.title, items}
	}
	return merged
}

// paletteHighlighted is `text` as spans, the query's words semibold on an accent tint, found
// ignoring case and accents.
func paletteHighlighted(text, query string, mark ui.Color) []ui.Span {
	terms := splitWords(paletteFold(query))
	if len(terms) == 0 || text == "" {
		return []ui.Span{{Text: text}}
	}
	characters := []rune(text)
	// Folded one character at a time, so a match maps back onto the characters it came from.
	folded := make([]string, len(characters))
	for i, r := range characters {
		one := paletteFold(string(r))
		if len([]rune(one)) != 1 {
			one = strings.ToLower(string(r))
		}
		folded[i] = one
	}
	marked := make([]bool, len(characters))
	for _, term := range terms {
		needle := []rune(term)
		for start := 0; start+len(needle) <= len(folded); start++ {
			match := true
			for offset, r := range needle {
				if folded[start+offset] != string(r) {
					match = false
					break
				}
			}
			if match {
				for offset := range needle {
					marked[start+offset] = true
				}
				start += len(needle) - 1
			}
		}
	}
	var spans []ui.Span
	run, inMark := []rune{}, false
	flush := func() {
		if len(run) == 0 {
			return
		}
		span := ui.Span{Text: string(run)}
		if inMark {
			span.Weight, span.Background = 600, mark
		}
		spans = append(spans, span)
		run = run[:0]
	}
	for i, r := range characters {
		if marked[i] != inMark {
			flush()
			inMark = marked[i]
		}
		run = append(run, r)
	}
	flush()
	return spans
}

func (s *paletteState) rows() []paletteRow {
	matched := paletteFilter(s.sections, s.query)
	if strings.TrimSpace(s.query) != "" {
		matched = paletteMerge(paletteSearchSections(s.m, s.results), matched)
	}
	var rows []paletteRow
	for _, section := range matched {
		rows = append(rows, paletteRow{header: section.title})
		for i := range section.items {
			rows = append(rows, paletteRow{item: &section.items[i]})
		}
	}
	return rows
}

// searchMessages asks the CLI for the query's messages a moment after the typing stops.
func (s *paletteState) searchMessages() {
	s.generation++
	generation := s.generation
	text := strings.TrimSpace(s.query)
	if text == "" {
		s.searching = false
		return
	}
	time.AfterFunc(140*time.Millisecond, func() {
		post(func() {
			if generation != s.generation {
				return
			}
			store.SearchChats(text, func(found model.WireSearchResults, err error) {
				if generation != s.generation {
					return
				}
				if err != nil {
					found = model.WireSearchResults{}
				}
				s.results, s.searching, s.picked = found, false, -1
				s.scroll.Y = 0
			})
		})
	})
}

const (
	paletteWidth        = 620
	paletteFieldHeight  = 52
	paletteRowHeight    = 36
	paletteHeaderHeight = 26
	paletteListHeight   = 360
	paletteEmptyHeight  = 64
)

func (s *paletteState) view(c *ui.Context, m *mainWindow) {
	if !s.open {
		return
	}
	// The window going to the background takes the palette with it.
	if !m.focused && m.win != nil {
		s.close()
		return
	}
	p := colors(c)
	width, height := c.Size()
	rows := s.rows()
	firstItem := slices.IndexFunc(rows, func(row paletteRow) bool { return row.item != nil })
	selected := s.picked
	if selected < 0 || selected >= len(rows) || rows[selected].item == nil {
		selected = firstItem
	}
	run := func(item *paletteItem) {
		s.close()
		item.run()
	}
	move := func(delta int) {
		var items []int
		for i, row := range rows {
			if row.item != nil {
				items = append(items, i)
			}
		}
		if len(items) == 0 {
			return
		}
		current := slices.Index(items, selected)
		var next int
		switch {
		case current >= 0:
			next = items[(current+delta+len(items))%len(items)]
		case delta > 0:
			next = items[0]
		default:
			next = items[len(items)-1]
		}
		s.picked, s.reveal = next, true
	}
	ui.Overlay(c, func() {
		scrim := ui.Box(c).Absolute().Left(0).Top(0).Right(0).Bottom(0)
		scrim.Children(func() {
			panelWidth := min(paletteWidth, width-40)
			top := max(90, float32(int(height*0.2)))
			panel := ui.Column(c).Absolute().Top(top).Left((width-panelWidth)/2).Width(panelWidth).Radius(18).Clip().
				Background(p.Popover).Shadow(0, 10, 40, 0, p.Shadow).Border(0.5, p.ShadowEdge).Label(L("Command Palette")).Role(ui.RoleDialog)
			panel.Children(func() {
				ui.Row(c).Height(paletteFieldHeight).Gap(8).Padding(0, 18).Children(func() {
					symbol(c, "magnifyingglass", 20, 1.8).TextColor(p.Label2)
					placeholder := L("Search actions, chats, messages, and settings")
					field := ui.TextInputBase(c, &s.query).Grow(1).MinWidth(0).FontSize(19).TextColor(p.Label).Placeholder(placeholder).Label(placeholder).FocusRing(false).AutoFocus()
					if field.Changed() {
						s.results = model.WireSearchResults{}
						s.searching = strings.TrimSpace(s.query) != ""
						s.picked = -1
						s.scroll.Y = 0
						s.searchMessages()
					}
					switch {
					case field.Shortcut(0, ui.KeyDown):
						move(1)
					case field.Shortcut(0, ui.KeyUp):
						move(-1)
					case field.Shortcut(0, ui.KeyEscape):
						s.close()
					case field.Submitted():
						if selected >= 0 && rows[selected].item != nil {
							run(rows[selected].item)
						}
					}
				})
				ui.Box(c).Height(1).Background(p.Separator)
				if len(rows) == 0 {
					text := L("No Results")
					if s.searching {
						text = L("Searching…")
					}
					ui.Row(c).Height(paletteEmptyHeight).Center().Children(func() {
						ui.Text(c, text).FontSize(13).TextColor(p.Label2)
					})
					return
				}
				list := ui.Scroll(c).TrackScroll(&s.scroll).MaxHeight(paletteListHeight).Padding(4, 10, 10, 10).Role(ui.RoleList)
				// Only a real move of the pointer selects a row, never rows that appear or scroll
				// beneath a resting pointer.
				x, y, over := list.PointerPosition()
				moved := over && (x != s.pointerX || y != s.pointerY)
				s.pointerX, s.pointerY = x, y
				list.Children(func() {
					for i, row := range rows {
						if row.item == nil {
							ui.Row(c.Key(i)).Height(paletteHeaderHeight).AlignItems(ui.End).Padding(0, 8, 4, 8).Children(func() {
								ui.Text(c, row.header).FontSize(11).FontWeight(600).TextColor(p.Label3)
							})
							continue
						}
						item := row.item
						on := i == selected
						r := ui.Row(c.Key(i)).Height(paletteRowHeight).Gap(8).Padding(0, 8).Radius(8).Role(ui.RoleListItem).Label(item.title)
						secondary := p.Label2
						mark := p.Accent.Alpha(0.2)
						if on {
							r.Background(p.Accent).TextColor(p.White)
							secondary, mark = ui.RGBA(255, 255, 255, 0.8), ui.RGBA(255, 255, 255, 0.25)
							if s.reveal {
								r.ScrollIntoView()
								s.reveal = false
							}
						}
						if moved && r.Hovered() && !on {
							s.picked = i
						}
						if r.Clicked() {
							run(item)
						}
						r.Children(func() {
							icon := ui.Row(c).Size(22, 22).Center().TextColor(secondary)
							if on {
								icon.TextColor(p.White)
							}
							icon.Children(func() {
								if item.bots != nil {
									avatars := make([]avatarContent, 0, 4)
									for _, bot := range item.bots[:min(4, len(item.bots))] {
										avatars = append(avatars, botAvatar(bot))
									}
									avatarCluster(c, avatars, 22, false, p.Popover)
								} else {
									symbol(c, item.symbol, 16, 1.8)
								}
							})
							ui.RichText(c, paletteHighlighted(item.title, s.query, mark)...).FontSize(13).MaxWidthPercent(60).SingleLine().Shrink(0)
							if item.subtitle != "" {
								ui.RichText(c, paletteHighlighted(item.subtitle, s.query, mark)...).FontSize(12).TextColor(secondary).Shrink(1).MinWidth(0).SingleLine()
							}
							ui.Spacer(c)
							if item.shortcut != "" {
								ui.Text(c, item.shortcut).FontSize(13).TextColor(secondary).NoWrap()
							}
						})
					}
				})
			})
			// A press outside the palette closes it.
			if panel.PressedOutside() {
				s.close()
			}
		})
	})
}
