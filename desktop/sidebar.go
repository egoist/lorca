package main

import (
	"fmt"
	"runtime"
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

	chats := store.Chats
	selected := -1
	for i, chat := range chats {
		if chat.ID == m.selection.ChatID {
			selected = i
		}
	}
	selectAt := func(index int) {
		if len(chats) == 0 {
			return
		}
		index = max(0, min(len(chats)-1, index))
		m.selectChat(chats[index].ID)
		s.list.ScrollIntoView(index)
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
		selectAt(selected + 1)
	case holder.Shortcut(0, ui.KeyUp):
		selectAt(selected - 1)
	case holder.Shortcut(0, ui.KeyPageDown):
		selectAt(selected + page)
	case holder.Shortcut(0, ui.KeyPageUp):
		selectAt(selected - page)
	case holder.Shortcut(0, ui.KeyHome):
		selectAt(0)
	case holder.Shortcut(0, ui.KeyEnd):
		selectAt(len(chats) - 1)
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
		for i, chat := range store.Chats {
			if strings.HasPrefix(strings.ToLower(store.Title(chat)), s.typed) {
				m.selectChat(chat.ID)
				s.list.ScrollIntoView(i)
				break
			}
		}
		return true
	})
	hints := m.shortcutHints(c)
	holder.Children(func() {
		s.list.Key = func(i int) any { return chats[i].ID }
		s.list.Label = func(i int) string { return store.Title(chats[i]) }
		ui.List(c, &s.list, len(chats), func(i int) {
			hint := ""
			if hints && i < len(chatNumberKeys) {
				hint = shortcutText(fmt.Sprintf("CmdOrCtrl+%d", i+1))
			}
			m.chatRow(c, chats[i], i == selected, focused, hint)
		}).Grow(1).Padding(2, 8, 8, 8)
	})
	m.sidebarFooter(c)
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
	label := []string{title}
	if chat.UnreadCount > 0 {
		label = append(label, L("%d unread", chat.UnreadCount))
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
		if chat.IsDM() {
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
