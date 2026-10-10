package main

import (
	"fmt"
	"runtime"
	"slices"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The sidebars, after the macOS app's: the chats (a search field that opens the command palette,
// a row per chat, and a footer with Settings, this computer, and the marketplace) and, while
// Settings is up, the settings panes (a search over the panes and their settings, and Back).

// MARK: - Chats

type chatsSidebarState struct {
	list ui.ListState
	// typed is what was typed to pick a chat by its title's first letters, until a pause.
	typed   string
	typedAt time.Time
	// revealSelection scrolls the selected row into view once.
	revealSelection bool
	// shownChat is the chat last selected, so a chat opened some other way unfolds its group once.
	shownChat string
}

// letterKeys are the keys that type the letters a type-select reads.
var letterKeys = func() map[ui.Key]rune {
	keys := map[ui.Key]rune{}
	letters := []ui.Key{ui.KeyA, ui.KeyB, ui.KeyC, ui.KeyD, ui.KeyE, ui.KeyF, ui.KeyG, ui.KeyH, ui.KeyI, ui.KeyJ, ui.KeyK, ui.KeyL, ui.KeyM,
		ui.KeyN, ui.KeyO, ui.KeyP, ui.KeyQ, ui.KeyR, ui.KeyS, ui.KeyT, ui.KeyU, ui.KeyV, ui.KeyW, ui.KeyX, ui.KeyY, ui.KeyZ}
	for i, key := range letters {
		keys[key] = rune('a' + i)
	}
	digits := []ui.Key{ui.Key0, ui.Key1, ui.Key2, ui.Key3, ui.Key4, ui.Key5, ui.Key6, ui.Key7, ui.Key8, ui.Key9}
	for i, key := range digits {
		keys[key] = rune('0' + i)
	}
	return keys
}()

// sidebarGroup is a folding group of the chats sidebar: one of the account's sections, the chats
// in no section, or the hidden chats.
type sidebarGroup struct {
	kind      sidebarGroupKind
	sectionID string
}

type sidebarGroupKind int

const (
	groupSection sidebarGroupKind = iota
	groupOthers
	groupHidden
)

// sidebarRow is a row of the chats sidebar: a chat, or a group's header.
type sidebarRow struct {
	chat      *model.Chat
	group     *sidebarGroup
	collapsed bool
}

func (r sidebarRow) key() any {
	if r.chat != nil {
		return r.chat.ID
	}
	return *r.group
}

// sidebarRows are what the chats sidebar lists, after the Mac's SidebarLayout: the pinned chats;
// each section with its chats, then the chats in no section under Chats; and the hidden chats
// under Hidden. Without sections the chats are one list with no header, and without hidden chats
// there is no Hidden. A folded group lists its header alone. `chats` are in the store's order.
func sidebarRows(chats []*model.Chat, sections []*model.Section, showsHidden, collapsesOthers bool) []sidebarRow {
	var rows []sidebarRow
	var listed, hidden []*model.Chat
	for _, chat := range chats {
		if chat.IsHidden {
			hidden = append(hidden, chat)
		} else {
			listed = append(listed, chat)
		}
	}
	group := func(g sidebarGroup, collapsed bool, members []*model.Chat) {
		rows = append(rows, sidebarRow{group: &g, collapsed: collapsed})
		if !collapsed {
			for _, chat := range members {
				rows = append(rows, sidebarRow{chat: chat})
			}
		}
	}
	if len(sections) == 0 {
		for _, chat := range listed {
			rows = append(rows, sidebarRow{chat: chat})
		}
	} else {
		known := map[string]bool{}
		for _, section := range sections {
			known[section.ID] = true
		}
		var others []*model.Chat
		for _, chat := range listed {
			switch {
			case chat.IsPinned:
				rows = append(rows, sidebarRow{chat: chat})
			case !known[chat.SectionID]:
				others = append(others, chat)
			}
		}
		for _, section := range sections {
			var members []*model.Chat
			for _, chat := range listed {
				if !chat.IsPinned && chat.SectionID == section.ID {
					members = append(members, chat)
				}
			}
			group(sidebarGroup{kind: groupSection, sectionID: section.ID}, section.Collapsed, members)
		}
		if len(others) > 0 {
			group(sidebarGroup{kind: groupOthers}, collapsesOthers, others)
		}
	}
	if len(hidden) > 0 {
		group(sidebarGroup{kind: groupHidden}, !showsHidden, hidden)
	}
	return rows
}

// groupOf is the group a chat is listed under, when the sidebar has groups for it.
func groupOf(chat *model.Chat) (sidebarGroup, bool) {
	switch {
	case chat.IsHidden:
		return sidebarGroup{kind: groupHidden}, true
	case len(store.Sections) == 0 || chat.IsPinned:
		return sidebarGroup{}, false
	case store.Section(chat.SectionID) != nil:
		return sidebarGroup{kind: groupSection, sectionID: chat.SectionID}, true
	default:
		return sidebarGroup{kind: groupOthers}, true
	}
}

func groupTitle(group sidebarGroup) string {
	switch group.kind {
	case groupSection:
		if section := store.Section(group.sectionID); section != nil {
			return section.Name
		}
		return ""
	case groupOthers:
		return Lc("Chats", "no section")
	default:
		return L("Hidden")
	}
}

// foldGroup folds or unfolds a group: a section on every Device, the others on this computer.
func foldGroup(group sidebarGroup, collapsed bool) {
	switch group.kind {
	case groupSection:
		store.SetSectionCollapsed(group.sectionID, collapsed)
	case groupOthers:
		setPrefs(PreferencesPatch{CollapsesOtherChats: &collapsed})
	case groupHidden:
		shows := !collapsed
		setPrefs(PreferencesPatch{ShowsHiddenChats: &shows})
	}
}

func groupCollapsed(group sidebarGroup) bool {
	switch group.kind {
	case groupSection:
		section := store.Section(group.sectionID)
		return section != nil && section.Collapsed
	case groupOthers:
		return prefs.get().CollapsesOtherChats
	default:
		return !prefs.get().ShowsHiddenChats
	}
}

func (m *mainWindow) chatsSidebar(c *ui.Context) {
	p := colors(c)
	s := &m.chatsList
	ui.Column(c).Padding(2, 10, 6, 10).Children(func() {
		// The field is the way into the palette: a click opens it, and the field never takes the
		// keyboard.
		field := ui.ButtonBase(c).Height(30).Gap(6).Justify(ui.Start).Padding(0, 8, 0, 10).Radius(15).Background(p.SearchBG).TextColor(p.Label2).Cursor(ui.CursorText).Label(L("Search")).FocusRing(false)
		field.Children(func() {
			symbol(c, "magnifyingglass", 13, 2)
			ui.Text(c, L("Search")).FontSize(13).TextColor(p.Placeholder).SingleLine()
		})
		if field.Clicked() {
			runCommand("palette")
		}
	})

	// A chat opened some other way (the palette, a notification, a hidden chat found by search)
	// shows its row: its group unfolds.
	if id := m.selection.ChatID; id != s.shownChat {
		s.shownChat = id
		if chat := store.Chat(id); chat != nil {
			if group, ok := groupOf(chat); ok && groupCollapsed(group) {
				foldGroup(group, false)
			}
		}
	}
	pr := prefs.get()
	rows := sidebarRows(store.Chats, store.Sections, pr.ShowsHiddenChats, pr.CollapsesOtherChats)
	var chatRows []int
	selected := -1
	for i, row := range rows {
		if row.chat == nil {
			continue
		}
		chatRows = append(chatRows, i)
		if row.chat.ID == m.selection.ChatID {
			selected = i
		}
	}
	place := slices.Index(chatRows, selected)
	// selectAt chooses the chat at a place among the chat rows; headers are passed over.
	selectAt := func(at int) {
		if len(chatRows) == 0 {
			return
		}
		at = max(0, min(len(chatRows)-1, at))
		m.selectChat(rows[chatRows[at]].chat.ID)
		s.list.ScrollIntoView(chatRows[at])
	}

	holder := ui.Column(c).Grow(1).MinHeight(0).Focusable().FocusRing(false).Label(L("Chats")).Role(ui.RoleList)
	focused := holder.FocusWithin()
	if s.revealSelection && selected >= 0 {
		s.list.ScrollIntoView(selected)
		s.revealSelection = false
	}
	// A page of rows is the list's height in 54-DIP rows, less one kept in view.
	page := max(1, int(holder.Bounds().H/54)-1)
	switch {
	case holder.Shortcut(0, ui.KeyDown):
		selectAt(place + 1)
	case holder.Shortcut(0, ui.KeyUp):
		if place < 0 {
			place = len(chatRows)
		}
		selectAt(place - 1)
	case holder.Shortcut(0, ui.KeyPageDown):
		selectAt(place + page)
	case holder.Shortcut(0, ui.KeyPageUp):
		selectAt(place - page)
	case holder.Shortcut(0, ui.KeyHome):
		selectAt(0)
	case holder.Shortcut(0, ui.KeyEnd):
		selectAt(len(chatRows) - 1)
	case holder.Shortcut(0, ui.KeyEnter):
		if m.selection.IsChat() {
			m.open(m.selection.ChatID)
		}
	}
	// Typing a chat's first letters selects it, as an outline view's type-select does.
	holder.HandleInput(func(ev ui.InputEvent) bool {
		if ev.Kind != ui.InputKeyDown || ev.Mods&^ui.Shift != 0 {
			return false
		}
		letter, ok := letterKeys[ev.Key]
		if !ok {
			return false
		}
		if time.Since(s.typedAt) > time.Second {
			s.typed = ""
		}
		s.typed += string(letter)
		s.typedAt = time.Now()
		for _, i := range chatRows {
			if strings.HasPrefix(strings.ToLower(store.Title(rows[i].chat)), s.typed) {
				m.selectChat(rows[i].chat.ID)
				s.list.ScrollIntoView(i)
				break
			}
		}
		return true
	})
	hints := m.shortcutHints(c)
	holder.Children(func() {
		s.list.Key = func(i int) any { return rows[i].key() }
		s.list.Label = func(i int) string {
			if rows[i].chat != nil {
				return store.Title(rows[i].chat)
			}
			return groupTitle(*rows[i].group)
		}
		// Chats drag into sections, and sections into another order.
		s.list.Reorder = nil
		if len(store.Sections) > 0 {
			s.list.Reorder = func(dragged []int, to int) { dropSidebarRows(rows, dragged, to) }
		}
		ui.List(c, &s.list, len(rows), func(i int) {
			row := rows[i]
			if row.group != nil {
				m.groupHeader(c, *row.group, row.collapsed)
				return
			}
			hint := ""
			if at := slices.Index(chatRows, i); hints && at >= 0 && at < len(chatNumberKeys) {
				hint = shortcutText(fmt.Sprintf("CmdOrCtrl+%d", at+1))
			}
			m.chatRow(c, row.chat, i == selected, focused, hint)
		}).Grow(1).Padding(2, 8, 8, 8)
	})
	m.sidebarFooter(c)
}

// visibleChats are the chats as their rows show, top down: Ctrl+1 to Ctrl+9 open the first nine.
func visibleChats() []*model.Chat {
	pr := prefs.get()
	var chats []*model.Chat
	for _, row := range sidebarRows(store.Chats, store.Sections, pr.ShowsHiddenChats, pr.CollapsesOtherChats) {
		if row.chat != nil {
			chats = append(chats, row.chat)
		}
	}
	return chats
}

// dropSidebarRows puts a dragged row where it was let go, `to` being the row it goes before: a
// chat into the group it lands in (not Hidden), a section among the sections.
func dropSidebarRows(rows []sidebarRow, dragged []int, to int) {
	if len(dragged) != 1 {
		return
	}
	row := rows[dragged[0]]
	to = min(to, len(rows))
	switch {
	case row.chat != nil:
		for i := to - 1; i >= 0; i-- {
			if group := rows[i].group; group != nil {
				if group.kind != groupHidden {
					store.MoveChat(row.chat.ID, group.sectionID)
				}
				return
			}
		}
	case row.group != nil && row.group.kind == groupSection:
		place := 0
		for _, before := range rows[:to] {
			if group := before.group; group != nil && group.kind == groupSection && group.sectionID != row.group.sectionID {
				place++
			}
		}
		store.MoveSection(row.group.sectionID, place)
	}
}

// groupHeader is a group's header, after the Mac's source-list group row: its name in small bold
// secondary text, a chevron that folds it while the pointer is over the row, and its menu.
func (m *mainWindow) groupHeader(c *ui.Context, group sidebarGroup, collapsed bool) {
	p := colors(c)
	title := groupTitle(group)
	row := ui.Row(c).Height(28).Padding(0, 6, 5, 6).AlignItems(ui.End).Gap(4).Label(title).Role(ui.RoleDisclosure).Expanded(!collapsed)
	hovered := row.Hovered()
	switch group.kind {
	case groupSection:
		id := group.sectionID
		row.ContextMenu(func(menu *ui.Menu) {
			at := slices.IndexFunc(store.Sections, func(section *model.Section) bool { return section.ID == id })
			if menu.Item(L("Rename Section…")).Chosen() {
				m.renameSection(id)
			}
			if menu.Item(L("Move Up")).Disabled(at <= 0).Chosen() {
				store.MoveSection(id, at-1)
			}
			if menu.Item(L("Move Down")).Disabled(at < 0 || at >= len(store.Sections)-1).Chosen() {
				store.MoveSection(id, at+1)
			}
			menu.Separator()
			if menu.Item(L("Delete Section…")).Chosen() {
				m.deleteSection(id)
			}
		})
	case groupOthers:
		row.ContextMenu(func(menu *ui.Menu) {
			if menu.Item(L("New Section…")).Chosen() {
				runCommand("newSection")
			}
		})
	}
	row.Children(func() {
		ui.Text(c, title).Grow(1).Shrink(1).MinWidth(0).FontSize(11).FontWeight(700).TextColor(p.Label2).SingleLine()
		name := "chevron.down"
		if collapsed {
			name = "chevron.right"
		}
		label := L("Hide")
		if collapsed {
			label = L("Show")
		}
		fold := ui.ButtonBase(c).Size(18, 18).Radius(4).Justify(ui.Center).TextColor(p.Label2).Label(label).FocusRing(false)
		if !hovered && !fold.Focused() {
			fold.Opacity(0)
		}
		fold.Children(func() { symbol(c, name, 11, 2.25) })
		if fold.Clicked() {
			foldGroup(group, !collapsed)
		}
	})
}

// shortcutHintDelay is how long Cmd (Ctrl on Windows and Linux) is held alone before the chats
// show their shortcuts, so a quick Cmd+C leaves the stamps alone.
const shortcutHintDelay = 250 * time.Millisecond

// shortcutHints is whether the first nine chats show their shortcuts in place of their stamps:
// Cmd held alone for a moment while the window takes keys and no sheet is up, as on the Mac.
func (m *mainWindow) shortcutHints(c *ui.Context) bool {
	if c.Modifiers() != ui.Cmd || m.hasSheet() {
		m.cmdHeldAt = time.Time{}
		return false
	}
	if m.cmdHeldAt.IsZero() {
		m.cmdHeldAt = c.Now()
	}
	if wait := shortcutHintDelay - c.Now().Sub(m.cmdHeldAt); wait > 0 {
		c.After(wait)
		return false
	}
	return true
}

// chatRow is one chat in the sidebar: its avatars, title, last message, and unread count, or its
// shortcut, `hint`, while Cmd is held. A press selects it, and its menu acts on it.
func (m *mainWindow) chatRow(c *ui.Context, chat *model.Chat, selected, listFocused bool, hint string) {
	p := colors(c)
	backdrop := p.Sidebar
	row := ui.Row(c.Key(chat.ID)).Height(54).Padding(0, 8, 0, 4).Radius(8).Gap(10)
	working := false
	for _, id := range chat.BotIDs {
		if store.IsWorking(id) {
			working = true
		}
	}
	title := store.Title(chat)
	muted := chat.IsMuted(c.Now())
	label := []string{title}
	if chat.UnreadCount > 0 {
		label = append(label, L("%d unread", chat.UnreadCount))
	}
	if muted {
		label = append(label, L("Muted"))
	}
	if working {
		label = append(label, L("Working"))
	}
	row.Label(strings.Join(label, ", ")).Role(ui.RoleListItem)
	secondary, tertiary := p.Label2, p.Label3
	pill := p.Label3
	if selected {
		if listFocused {
			row.Background(p.Selection).TextColor(p.SelectionText)
			backdrop = p.Selection
			secondary, tertiary = ui.RGBA(255, 255, 255, 0.8), ui.RGBA(255, 255, 255, 0.8)
			pill = ui.RGBA(255, 255, 255, 0.25)
		} else {
			row.Background(p.SelectionInactive)
			backdrop = p.SelectionInactive.Over(p.Sidebar)
		}
	}
	// A row is chosen as the pointer goes down, as the Mac's are; assistive technology presses
	// it instead.
	if (row.Pressed() || row.Clicked()) && !selected {
		m.selectChat(chat.ID)
	}
	if row.DoubleClicked() && chat.IsGroup() {
		m.renameChat()
	}
	chatID := chat.ID
	row.ContextMenu(func(menu *ui.Menu) {
		// Acting on the clicked row means selecting it first; the commands read the selection.
		m.selectChat(chatID)
		pin := L("Pin")
		if chat.IsPinned {
			pin = L("Unpin")
		}
		if menu.Item(pin).Chosen() {
			store.TogglePin(chatID)
		}
		m.chatSidebarItems(menu, chat)
		menu.Separator()
		if chat.IsGroup() {
			if menu.Item(L("Rename…")).Chosen() {
				m.renameChat()
			}
			if menu.Item(L("Add Bot…")).Disabled(len(botsAvailableToAdd(chatID)) == 0).Chosen() {
				m.addBotToChat(chatID)
			}
		} else if menu.Item(L("Share as Template…")).Chosen() {
			runCommand("shareBotTemplate")
		}
		menu.Separator()
		remove := L("Delete Chat")
		if chat.IsBotDM() {
			remove = L("Delete Bot")
		}
		if menu.Item(remove).Chosen() {
			m.deleteChat()
		}
	})
	row.Children(func() {
		members := store.BotsIn(chat)
		avatars := make([]avatarContent, 0, 4)
		for _, bot := range members[:min(4, len(members))] {
			avatars = append(avatars, botAvatar(bot))
		}
		avatarCluster(c, avatars, 38, working, backdrop)
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(2).Children(func() {
			ui.Row(c).Gap(6).MinWidth(0).Children(func() {
				ui.Text(c, title).Grow(1).Shrink(1).MinWidth(0).FontSize(13).FontWeight(500).SingleLine()
				if muted {
					symbol(c, "bell.slash.fill", 10, 2).TextColor(tertiary).Label(L("Muted"))
				}
				if chat.IsPinned {
					symbol(c, "pin.fill", 10, 2).TextColor(tertiary).Label(L("Pinned"))
				}
				stamp := model.Stamp(chat.LastActivity())
				if hint != "" {
					stamp = hint
				}
				ui.Text(c, stamp).FontSize(textCaption).TextColor(tertiary).FontFeatures("tnum").SingleLine()
			})
			ui.Row(c).Gap(6).MinWidth(0).Children(func() {
				ui.Text(c, store.Preview(chat)).Grow(1).Shrink(1).MinWidth(0).FontSize(11.5).TextColor(secondary).SingleLine()
				if chat.UnreadCount > 0 {
					count := fmt.Sprint(chat.UnreadCount)
					if chat.UnreadCount > 999 {
						count = "999+"
					}
					ui.Row(c).MinWidth(16).Height(16).Padding(0, 2.5).Radius(8).Justify(ui.Center).Background(pill).Children(func() {
						ui.Text(c, count).FontSize(textCaption).FontWeight(600).TextColor(p.White).FontFeatures("tnum")
					}).Label(L("%d unread", chat.UnreadCount))
				}
			})
		})
	})
}

// chatSidebarItems are a chat's items that quiet and file it, after the Mac's: Mute and its spans
// or Unmute, Move to Section and the sections (Move to New Section… while there are none), and
// Hide or Show in Sidebar.
func (m *mainWindow) chatSidebarItems(menu *ui.Menu, chat *model.Chat) {
	chatID := chat.ID
	if chat.IsMuted(time.Now()) {
		if menu.Item(L("Unmute")).Chosen() {
			store.Unmute(chatID)
		}
	} else {
		menu.Submenu(L("Mute"), func(menu *ui.Menu) {
			for _, span := range muteSpans() {
				if menu.Item(span.title).Chosen() {
					muteChat(chatID, span.seconds)
				}
			}
		})
	}
	if len(store.Sections) == 0 {
		if menu.Item(L("Move to New Section…")).Chosen() {
			m.moveChatToNewSection(chatID)
		}
	} else {
		menu.Submenu(L("Move to Section"), func(menu *ui.Menu) { m.sectionItems(menu, chat) })
	}
	hide := L("Hide")
	if chat.IsHidden {
		hide = L("Show in Sidebar")
	}
	if menu.Item(hide).Chosen() {
		store.SetHidden(chatID, !chat.IsHidden)
	}
}

// sectionItems are each section, checked where the chat is, then Chats for no section, then New
// Section….
func (m *mainWindow) sectionItems(menu *ui.Menu, chat *model.Chat) {
	current := ""
	if chat != nil && store.Section(chat.SectionID) != nil {
		current = chat.SectionID
	}
	for _, section := range store.Sections {
		if menu.Item(section.Name).Checked(chat != nil && current == section.ID).Disabled(chat == nil).Chosen() {
			store.MoveChat(chat.ID, section.ID)
		}
	}
	if menu.Item(Lc("Chats", "no section")).Checked(chat != nil && current == "").Disabled(chat == nil).Chosen() {
		store.MoveChat(chat.ID, "")
	}
	menu.Separator()
	if menu.Item(L("New Section…")).Disabled(chat == nil).Chosen() {
		m.moveChatToNewSection(chat.ID)
	}
}

type muteSpan struct {
	title   string
	seconds int
}

// muteSpans are how long Mute keeps a chat quiet; zero seconds is until unmuted.
func muteSpans() []muteSpan {
	return []muteSpan{{L("For 1 Hour"), 3600}, {L("For 8 Hours"), 8 * 3600}, {L("For 1 Week"), 7 * 24 * 3600}, {L("Always"), 0}}
}

func muteChat(chatID string, seconds int) {
	var until time.Time
	if seconds > 0 {
		until = time.Now().Add(time.Duration(seconds) * time.Second)
	}
	store.Mute(chatID, until)
}

// sidebarFooter is Settings, and this computer, whose icon turns red while the CLI is not
// answering and orange while the CLI cannot reach the relay, with why in its tooltip; the
// marketplace at the other end.
func (m *mainWindow) sidebarFooter(c *ui.Context) {
	p := colors(c)
	ui.Row(c).Gap(2).Padding(6, 8, 8, 8).Children(func() {
		if hoverButton(c, hoverButtonOptions{Symbol: "gearshape", Tooltip: L("Settings (%@)", shortcutText("CmdOrCtrl+,")), Label: L("Settings")}).Clicked() {
			m.selectSettings(model.PaneGeneral)
		}
		connected := store.IsConnected
		device := store.ThisDevice()
		name := L("This computer")
		symbolName := "laptopcomputer"
		if runtime.GOOS == "windows" {
			symbolName = "pc"
		}
		if device != nil {
			name, symbolName = device.Name, device.Symbol()
		}
		if symbolName == "desktopcomputer" {
			symbolName = "display"
		}
		relayError := ""
		if connected {
			relayError = store.RelayError
		}
		var status string
		switch {
		case !connected:
			status = strings.Replace(L("CLI not running · start it with: lorca serve"), "lorca serve", cliCommand(), 1)
		case relayError != "":
			status = L("Can’t connect to the relay: %@", relayError)
		default:
			status = L("CLI on 127.0.0.1:%@", fmt.Sprint(prefs.cliPort()))
		}
		this := hoverButton(c, hoverButtonOptions{Symbol: symbolName, Tooltip: name + " · " + status, Label: name})
		if !connected {
			this.TextColor(p.Red)
		} else if relayError != "" {
			this.TextColor(p.Orange)
		}
		if this.Clicked() && device != nil {
			m.openDevice(device.ID)
		}
		ui.Spacer(c)
		if hoverButton(c, hoverButtonOptions{Symbol: "circle.grid.2x2", Tooltip: L("Marketplace (%@)", shortcutText("CmdOrCtrl+Shift+M")), Label: L("Marketplace")}).Clicked() {
			runCommand("marketplace")
		}
	})
}

// MARK: - Settings

type settingsSidebarState struct {
	query string
	// picked is the result picked, by its pane and row, which outlast the rows rebuilt as the query
	// or the roster changes.
	pickedPane model.SettingsPane
	pickedRow  string
	// focusSearch and focusList ask for the keyboard in the field or the list, once.
	focusSearch bool
	focusList   bool
}

type settingsRow struct {
	pane  model.SettingsPane
	entry *settingsEntry
}

func (r settingsRow) key() string {
	if r.entry == nil {
		return "pane:" + string(r.pane)
	}
	return "setting:" + string(r.entry.pane) + ":" + r.entry.row
}

func (m *mainWindow) settingsSidebar(c *ui.Context) {
	p := colors(c)
	s := &m.settingsBar
	if s.pickedPane != "" && s.pickedPane != m.selection.Pane {
		// The pane changed some other way (Back, Forward, the palette): its row is the one selected.
		s.pickedPane, s.pickedRow = "", ""
	}
	var rows []settingsRow
	text := strings.TrimSpace(s.query)
	if text == "" {
		for _, pane := range model.SettingsPanes {
			rows = append(rows, settingsRow{pane: pane})
		}
	} else {
		for _, match := range panesMatching(text, store.Device(m.settingsDeviceID)) {
			rows = append(rows, settingsRow{pane: match.pane})
			for i := range match.entries {
				rows = append(rows, settingsRow{pane: match.pane, entry: &match.entries[i]})
			}
		}
	}
	selectedKey := ""
	if s.pickedPane != "" && s.pickedPane == m.selection.Pane {
		selectedKey = "setting:" + string(s.pickedPane) + ":" + s.pickedRow
	} else if m.selection.IsSettings() {
		selectedKey = "pane:" + string(m.selection.Pane)
	}
	pick := func(row settingsRow) {
		if row.entry == nil {
			s.pickedPane, s.pickedRow = "", ""
			m.selectSettings(row.pane)
			return
		}
		s.pickedPane, s.pickedRow = row.entry.pane, row.entry.row
		m.reveal(*row.entry)
	}
	// Opens the best search result: the first matching setting, or else the first page listed.
	pickFirst := func() bool {
		if text == "" || len(rows) == 0 {
			return false
		}
		first := rows[0]
		for _, row := range rows {
			if row.entry != nil {
				first = row
				break
			}
		}
		pick(first)
		return true
	}
	move := func(delta int) {
		index := -1
		for i, row := range rows {
			if row.key() == selectedKey {
				index = i
			}
		}
		next := max(0, min(len(rows)-1, index+delta))
		if next < len(rows) {
			pick(rows[next])
		}
	}

	var list ui.Element
	ui.Column(c).Padding(2, 10, 6, 10).Children(func() {
		field := searchField(c, &s.query, "")
		if s.focusSearch {
			field.Focus()
			s.focusSearch = false
		}
		if field.Submitted() {
			pickFirst()
		}
		if field.Focused() && field.Shortcut(0, ui.KeyDown) {
			// A result is picked only when the list has no row selected; otherwise Down only moves
			// the keyboard to the list.
			has := false
			for _, row := range rows {
				if row.key() == selectedKey {
					has = true
				}
			}
			if !has {
				pickFirst()
			}
			s.focusList = true
		}
	})
	list = ui.Scroll(c).Grow(1).MinHeight(0).Padding(2, 8, 8, 8).Focusable().FocusRing(false).Role(ui.RoleList).Label(L("Settings"))
	if s.focusList {
		list.Focus()
		s.focusList = false
	}
	focused := list.FocusWithin()
	if list.Shortcut(0, ui.KeyDown) {
		move(1)
	}
	if list.Shortcut(0, ui.KeyUp) {
		move(-1)
	}
	list.Children(func() {
		for _, row := range rows {
			on := row.key() == selectedKey
			var e ui.Element
			if row.entry == nil {
				e = ui.Row(c.Key(row.key())).Height(32).Padding(0, 8, 0, 4).Radius(7).FontSize(13)
				e.Children(func() {
					icon := ui.Row(c).Width(38).Margin(0, 10, 0, 0).Justify(ui.Center).TextColor(p.Label2)
					if on && focused {
						icon.TextColor(p.SelectionText)
					}
					icon.Children(func() { symbol(c, row.pane.Symbol(), 16, 1.7) })
					ui.Text(c, row.pane.Title()).SingleLine()
				})
			} else {
				e = ui.Row(c.Key(row.key())).Height(26).Padding(0, 8, 0, 52).Radius(6).FontSize(12).Tooltip(row.entry.title)
				e.Children(func() { ui.Text(c, row.entry.title).SingleLine() })
			}
			e.Role(ui.RoleListItem).Label(firstNonEmpty(func() string {
				if row.entry != nil {
					return row.entry.title
				}
				return row.pane.Title()
			}()))
			if on {
				if focused {
					e.Background(p.Selection).TextColor(p.SelectionText)
				} else {
					e.Background(p.SelectionInactive)
				}
			}
			if (e.Pressed() || e.Clicked()) && !on {
				pick(row)
			}
		}
		if len(rows) == 0 {
			ui.Text(c, L("No Results for “%@”", text)).Padding(18, 8).FontSize(12).TextColor(p.Label2).TextAlign(ui.Center)
		}
	})
	ui.Row(c).Gap(2).Padding(6, 8, 8, 8).Children(func() {
		if hoverButton(c, hoverButtonOptions{Symbol: "chevron.left", Size: 13, Title: L("Back"), Tooltip: L("Back to Chats (esc)"), Label: L("Back to Chats")}).Clicked() {
			m.closeSettings()
		}
	})
}

// reveal opens a setting's pane, scrolls to its row, and flashes it.
func (m *mainWindow) reveal(entry settingsEntry) {
	m.showSettings(entry.pane)
	m.revealed = &revealedSetting{entry: entry}
}
