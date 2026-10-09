package main

import (
	"slices"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/plugins/glass"
	"github.com/egoist/mygo/ui"
)

// The main window, after the macOS app's MainWindowController and RootSplitViewController: the
// panes' headers (the sidebar and create buttons, back and forward in Settings, the title, the
// Device picker on the Device panes, the chat's running tasks, and the inspector's toggle) over a
// split of the sidebar, the content, and the inspector. The content is a chat, a settings pane, or
// with nothing selected, a state.

// contentMinWidth is the narrowest the content gets before the side panes give way.
const contentMinWidth = 460

const (
	sidebarMin, sidebarMax     = 232, 340
	inspectorMin, inspectorMax = 268, 320
	headerHeight               = 52
)

type mainWindow struct {
	appWindow

	// selection is what the window shows: a chat, or a settings pane, which puts the settings
	// sidebar in the chats' place.
	selection model.Selection
	// settingsDeviceID is the Device the Plugins, Bots, and Devices panes show: this computer until
	// another is picked.
	settingsDeviceID string
	// paneHistory is the panes visited since Settings opened, for the header's back and forward.
	paneHistory    []model.SettingsPane
	paneIndex      int
	walkingHistory bool
	lastChatID     string

	userWantsInspector bool
	sidebarCollapsed   bool
	// squeezesContent is the user having opened a side pane the window could not widen enough
	// for: the content narrows past its minimum rather than the inspector stepping aside, until the
	// window narrows.
	squeezesContent bool
	lastWidth       float32
	sidebarWidth    float32
	inspectorWidth  float32
	// shownChat is a chat having been on screen since the window opened.
	shownChat bool

	// revealed is a setting a search picked, for its pane to scroll to and flash.
	revealed *revealedSetting

	chat        *chatState
	chatsList   chatsSidebarState
	settingsBar settingsSidebarState
	settings    settingsPageState
	palette     *paletteState
	inspector   inspectorState
	tasks       runningTasksState
	attention   attentionState
	// focusComposer asks the chat on screen to put the keyboard in its composer.
	focusComposer bool
	// cmdHeldAt is when Cmd (Ctrl on Windows and Linux) went down alone, for the chats' shortcut
	// hints.
	cmdHeldAt time.Time
	// saidUpdateRequired is the relay having turned this build away, said once per launch.
	saidUpdateRequired bool
}

func newMainWindow() *mainWindow {
	p := prefs.get()
	m := &mainWindow{
		userWantsInspector: p.ShowsInspector,
		sidebarCollapsed:   p.SidebarCollapsed,
		sidebarWidth:       260,
		inspectorWidth:     280,
		palette:            &paletteState{picked: -1},
	}
	if p.SidebarWidth > 0 {
		m.sidebarWidth = clamp(float32(p.SidebarWidth), sidebarMin, sidebarMax)
	}
	if p.InspectorWidth > 0 {
		m.inspectorWidth = clamp(float32(p.InspectorWidth), inspectorMin, inspectorMax)
	}
	return m
}

func clamp(value, low, high float32) float32 { return min(high, max(low, value)) }

// MARK: - Selection

func (m *mainWindow) selectedChatID() string { return m.selection.ChatID }

func (m *mainWindow) isSettings() bool { return m.selection.IsSettings() }

func (m *mainWindow) recordHistory(current model.Selection) {
	if !current.IsSettings() {
		m.paneHistory, m.paneIndex = nil, 0
		return
	}
	if m.walkingHistory {
		return
	}
	kept := slices.Clone(m.paneHistory[:min(len(m.paneHistory), m.paneIndex+1)])
	if len(kept) == 0 || kept[len(kept)-1] != current.Pane {
		kept = append(kept, current.Pane)
	}
	m.paneHistory, m.paneIndex = kept, len(kept)-1
}

func (m *mainWindow) canGoBack() bool    { return m.paneIndex > 0 }
func (m *mainWindow) canGoForward() bool { return m.paneIndex < len(m.paneHistory)-1 }
func (m *mainWindow) goBack()            { m.walkHistory(m.paneIndex - 1) }
func (m *mainWindow) goForward()         { m.walkHistory(m.paneIndex + 1) }

func (m *mainWindow) walkHistory(index int) {
	if index < 0 || index >= len(m.paneHistory) {
		return
	}
	m.paneIndex = index
	m.walkingHistory = true
	m.selectSettings(m.paneHistory[index])
	m.walkingHistory = false
}

func (m *mainWindow) setSelection(next model.Selection) {
	old := m.selection
	if old == next {
		return
	}
	m.selection = next
	m.recordHistory(next)
	if old.IsChat() {
		m.lastChatID = old.ChatID
	}
	// A relaunch lands on the chat that showed last, never on a settings pane.
	if !next.IsSettings() {
		encoded := next.Encode()
		prefs.update(PreferencesPatch{Selection: &encoded})
	}
	app.notifier.watchingChanged()
	m.invalidate()
}

// selectChat shows a chat and marks it read.
func (m *mainWindow) selectChat(id string) {
	leaving := m.isSettings()
	m.setSelection(model.Selection{ChatID: id})
	store.MarkRead(id)
	// Leaving Settings, the keyboard goes back to the chat.
	if leaving {
		m.focusComposer = true
	}
}

func (m *mainWindow) selectSettings(pane model.SettingsPane) {
	entering := !m.isSettings()
	m.setSelection(model.Selection{Pane: pane})
	if entering {
		m.settingsBar.focusList = true
	}
}

func (m *mainWindow) selectNone() { m.setSelection(model.Selection{}) }

// restoreSelection brings the selection back after a snapshot: an open settings pane or a chat that
// still exists stays, else the saved chat when it still exists, else the first chat.
func (m *mainWindow) restoreSelection() {
	if m.isSettings() || (m.selection.IsChat() && store.Chat(m.selection.ChatID) != nil) {
		return
	}
	if m.selection.IsChat() && !store.IsConnected {
		return
	}
	saved := model.DecodeSelection(prefs.get().Selection)
	id := ""
	if saved.IsChat() && (store.Chat(saved.ChatID) != nil || !store.IsConnected) {
		id = saved.ChatID
	} else if len(store.Chats) > 0 {
		id = store.Chats[0].ID
	}
	if id != "" {
		m.setSelection(model.Selection{ChatID: id})
	} else {
		m.setSelection(model.Selection{})
	}
}

// showSettings is Settings in this window: its panes take the content area, and the sidebar lists
// them in place of the chats.
func (m *mainWindow) showSettings(pane model.SettingsPane) {
	if m.sidebarCollapsed {
		m.setSidebarCollapsed(false)
	}
	if pane != "" {
		m.selectSettings(pane)
		return
	}
	if m.isSettings() {
		return
	}
	m.selectSettings(model.PaneGeneral)
}

// openDevice is the Devices pane on that Device, with the other Device panes on it too.
func (m *mainWindow) openDevice(id string) {
	m.showSettingsDevice(id)
	if m.sidebarCollapsed {
		m.setSidebarCollapsed(false)
	}
	m.selectSettings(model.PaneDevice)
}

func (m *mainWindow) showSettingsDevice(id string) {
	if device := store.Device(id); device != nil {
		m.settingsDeviceID = device.ID
	} else if this := store.ThisDevice(); this != nil {
		m.settingsDeviceID = this.ID
	} else {
		m.settingsDeviceID = ""
	}
}

func (m *mainWindow) closeSettings() {
	if chat := store.Chat(m.lastChatID); chat != nil {
		m.selectChat(chat.ID)
	} else if len(store.Chats) > 0 {
		m.selectChat(store.Chats[0].ID)
	} else {
		m.selectNone()
		m.focusComposer = true
	}
}

func (m *mainWindow) setSidebarCollapsed(collapsed bool) {
	m.sidebarCollapsed = collapsed
	prefs.update(PreferencesPatch{SidebarCollapsed: &collapsed})
	m.invalidate()
}

func (m *mainWindow) toggleSidebar() {
	if m.sidebarCollapsed {
		m.openPane(true, func() { m.setSidebarCollapsed(false) })
	} else {
		m.setSidebarCollapsed(true)
	}
}

// toggleInspector goes by what is on screen, as the Mac app's split view's does: an inspector that
// stepped aside for room comes back.
func (m *mainWindow) toggleInspector() {
	if !m.selection.IsChat() || !store.IsConnected {
		mygo.Shell.Beep()
		return
	}
	show := func(wants bool) {
		m.userWantsInspector = wants
		prefs.update(PreferencesPatch{ShowsInspector: &wants})
		m.invalidate()
	}
	_, inspector := m.panes(m.lastWidth)
	if inspector > 0 {
		show(false)
	} else {
		m.openPane(false, func() { show(true) })
	}
}

// open shows a chat with the keyboard in its composer.
func (m *mainWindow) open(chatID string) {
	m.selectChat(chatID)
	m.focusComposer = true
}

// storeChanged is the window's reaction to the store: the Device picker falls back to this
// computer when its Device leaves, and a chat that goes away gives the selection to the first chat.
func (m *mainWindow) storeChanged(event model.Event) {
	m.chat.storeChanged(event)
	m.inspectorStoreChanged(event)
	switch event.Kind {
	case model.EventSnapshotReplaced:
		m.showSettingsDevice(m.settingsDeviceID)
		m.restoreSelection()
	case model.EventChatsChanged:
		if m.selection.IsChat() && store.Chat(m.selection.ChatID) == nil && store.IsConnected {
			if len(store.Chats) > 0 {
				m.selectChat(store.Chats[0].ID)
			} else {
				m.selectNone()
			}
		}
	case model.EventRosterChanged:
		m.showSettingsDevice(m.settingsDeviceID)
		m.relayStateChanged()
	case model.EventConnectionChanged:
		if store.IsConnected {
			m.restoreSelection()
		}
	}
}

// relayStateChanged says once per launch that the relay turned this build away: every sync
// attempt gets the same answer until Lorca is updated.
func (m *mainWindow) relayStateChanged() {
	if !store.RelayUpdateRequired || m.saidUpdateRequired {
		return
	}
	m.saidUpdateRequired = true
	updates := updatesEnabled()
	buttons := []alertButton{{Title: L("OK")}}
	if updates {
		buttons = []alertButton{{Title: L("Check for Updates…")}, {Title: L("Later")}}
	}
	m.showAlert(alertOptions{
		Message:     L("Update Lorca to keep syncing"),
		Informative: L("The relay no longer works with this version. Chats on this computer stay as they are, and nothing syncs with your other Devices until you update."),
		Buttons:     buttons,
	}, func(answer int) {
		if updates && answer == 0 {
			runCommand("checkForUpdates")
		}
	})
}

// MARK: - Commands

// run does what a command of this window does.
func (m *mainWindow) run(id string) {
	switch id {
	case "newBot":
		m.newBot()
	case "importBotTemplate":
		m.presentTemplateImport("", "", m.selectChat)
	case "shareBotTemplate":
		if bot := selectedDMBot(); bot != nil {
			m.presentTemplateShare(bot.ID)
		}
	case "newGroupChat":
		m.newGroupChat()
	case "newTask":
		m.presentDurableTask(m.selection.ChatID, nil)
	case "marketplace":
		m.presentMarketplace("")
	case "pairDevice":
		m.presentPairing()
	case "find":
		if m.isSettings() {
			m.settingsBar.focusSearch = true
		} else {
			m.palette.show()
		}
	case "palette":
		m.palette.toggle()
	case "attention":
		m.toggleAttention()
	case "toggleSidebar":
		m.toggleSidebar()
	case "toggleInspector":
		m.toggleInspector()
	case "scrollToLatest":
		if m.chat != nil && m.chat.chatID == m.selection.ChatID {
			m.chat.scrollToLatest = true
		}
	case "addBot":
		m.addBotToChat(m.selection.ChatID)
	case "renameChat":
		m.renameChat()
	case "pinChat":
		if id := m.selection.ChatID; id != "" {
			store.TogglePin(id)
		}
	case "newSkill":
		m.newSkill()
	case "stopResponding":
		if id := m.selection.ChatID; id != "" {
			store.StopResponding(id)
		}
	case "runInBackground":
		// Every command in the chat that a bot's call still waits on.
		if id := m.selection.ChatID; id != "" {
			for _, message := range store.ForegroundCommands(id) {
				store.SendCommandToBackground(id, message.ID, nil)
			}
		}
	case "deleteChat":
		m.deleteChat()
	}
	m.invalidate()
}

// botsAvailableToAdd are the bots that could still join a chat: none for a DM, a full group, or
// when every bot is in it.
func botsAvailableToAdd(chatID string) []*model.Bot {
	chat := store.Chat(chatID)
	if chat == nil || !chat.CanAddBot() {
		return nil
	}
	var out []*model.Bot
	for _, bot := range store.Bots {
		if !slices.Contains(chat.BotIDs, bot.ID) {
			out = append(out, bot)
		}
	}
	return out
}

func (m *mainWindow) renameChat() {
	chat := store.Chat(m.selection.ChatID)
	if chat == nil || !chat.IsGroup() {
		mygo.Shell.Beep()
		return
	}
	chatID := chat.ID
	value := store.Title(chat)
	m.showAlert(alertOptions{
		Message:     L("Rename Chat"),
		Informative: L("Chat names live inside the encrypted roster blob, never on the relay."),
		Buttons:     []alertButton{{Title: L("Rename")}, {Title: L("Cancel")}},
		Accessory: func(c *ui.Context) {
			textField(c, &value, fieldOptions{Placeholder: L("Chat name"), AutoFocus: true})
		},
	}, func(answer int) {
		if answer == 0 {
			store.Rename(chatID, value)
		}
	})
}

func (m *mainWindow) deleteChat() {
	chat := store.Chat(m.selection.ChatID)
	if chat == nil {
		mygo.Shell.Beep()
		return
	}
	chatID := chat.ID
	deletesBot := chat.IsDM() && len(chat.BotIDs) > 0 && store.Bot(chat.BotIDs[0]) != nil
	informative := L("The transcript is removed from this Device and from paired Devices.")
	confirm := L("Delete")
	if deletesBot {
		informative = L("The bot, its routines, and this direct chat are removed from this Device and from paired Devices.")
		confirm = L("Delete Bot")
	}
	m.showAlert(alertOptions{
		Message:     L("Delete \"%@\"?", store.Title(chat)),
		Informative: informative,
		Style:       alertWarning,
		Buttons:     []alertButton{{Title: confirm, Destructive: true}, {Title: L("Cancel")}},
	}, func(answer int) {
		if answer == 0 {
			store.DeleteChat(chatID)
		}
	})
}

// MARK: - Layout

// panes are the side panes' widths in a window `available` wide, each with its divider's DIP, as
// they give way to the content down to their own minimums, the inspector first; past that the
// inspector steps aside until there is room again, as AppKit's split view collapses it.
func (m *mainWindow) panes(available float32) (sidebar, inspector float32) {
	if !m.sidebarCollapsed {
		sidebar = m.sidebarWidth + 1
	}
	if m.showsInspector() {
		inspector = m.inspectorWidth + 1
	}
	s, i, fits := fitPanes(available, sidebar, inspector)
	// With the inspector aside, the sidebar has its own width back where there is room.
	if !fits && inspector > 0 && !m.squeezesContent {
		s, i, _ = fitPanes(available, sidebar, 0)
	}
	return max(0, s-1), max(0, i-1)
}

func fitPanes(available, sidebar, inspector float32) (float32, float32, bool) {
	short := contentMinWidth - (available - sidebar - inspector)
	giveWay := func(width, low float32) float32 {
		taken := max(0, min(short, width-low))
		short -= taken
		return width - taken
	}
	if inspector > 0 {
		inspector = giveWay(inspector, inspectorMin+1)
	}
	if sidebar > 0 {
		sidebar = giveWay(sidebar, sidebarMin+1)
	}
	return sidebar, inspector, short <= 0
}

// showsInspector is whether the inspector belongs beside what the window shows: a chat, or until a
// chat has shown (loading, or a first start that failed), the chat the window is about to show,
// so the transcript never widens and then narrows.
func (m *mainWindow) showsInspector() bool {
	if !m.userWantsInspector {
		return false
	}
	if m.visibleChatID() != "" {
		return true
	}
	return !m.shownChat && !store.IsConnected && m.selection.IsChat()
}

// visibleChatID is the chat on screen: the selected one, while the CLI answers and has it.
func (m *mainWindow) visibleChatID() string {
	id := m.selection.ChatID
	if id == "" || !store.IsConnected || store.Chat(id) == nil {
		return ""
	}
	return id
}

// openPane opens a side pane: where there is no room, the window widens by the pane, so the
// content keeps its width, as AppKit's split view does. Where the window cannot widen enough
// (maximized, full screen, the screen's edges), the content narrows past its minimum instead.
func (m *mainWindow) openPane(sidebar bool, show func()) {
	s, i := m.panes(m.lastWidth)
	wantSidebar, wantInspector := float32(0), float32(0)
	if sidebar || s > 0 {
		wantSidebar = m.sidebarWidth + 1
	}
	if !sidebar || i > 0 {
		wantInspector = m.inspectorWidth + 1
	}
	if _, _, fits := fitPanes(m.lastWidth, wantSidebar, wantInspector); fits || m.win.IsMaximized() || m.win.IsFullScreen() {
		if !fits {
			m.squeezesContent = true
		}
		show()
		return
	}
	width := wantInspector
	if sidebar {
		width = wantSidebar
	}
	bounds := m.win.Bounds()
	area := mygo.Screen.DisplayMatching(bounds).WorkArea
	next := widened(bounds, area, int(width+0.5))
	if next.Width-bounds.Width < int(width) {
		m.squeezesContent = true
	}
	if next != bounds {
		m.win.SetBounds(next)
	}
	show()
}

// widened is r made `by` wider within area: to the right where the area has room, then to the
// left, and past both by less. It never shrinks r or moves it back into the area.
func widened(r, area mygo.Rectangle, by int) mygo.Rectangle {
	right := min(by, max(0, area.X+area.Width-(r.X+r.Width)))
	left := min(by-right, max(0, r.X-area.X))
	return mygo.Rectangle{X: r.X - left, Y: r.Y, Width: r.Width + left + right, Height: r.Height}
}

// title is the window's title and subtitle: the chat's, the pane's, or the app's.
func (m *mainWindow) title() (string, string) {
	if m.selection.IsChat() {
		if chat := store.Chat(m.selection.ChatID); chat != nil {
			return store.Title(chat), store.Subtitle(chat)
		}
	}
	if m.selection.IsSettings() {
		return m.selection.Pane.Title(), ""
	}
	return appName(), ""
}

func (m *mainWindow) view(c *ui.Context) {
	p := colors(c)
	width, _ := c.Size()
	if width < m.lastWidth {
		m.squeezesContent = false
	}
	m.lastWidth = width
	chatID := m.visibleChatID()
	if chatID != "" {
		m.shownChat = true
	}
	title, subtitle := m.title()
	m.setTitle(title)
	sidebar, inspector := m.panes(width)

	m.shortcuts(c)
	// Where the window shows its material, the sidebar is transparent over it, as the Mac's.
	vibrant := c.Vibrancy()
	if vibrant {
		c.Root().Background(ui.Transparent)
	}
	root := ui.Box(c).Fill()
	if !vibrant {
		root.Background(p.Content)
	}
	root.Children(func() {
		ui.Row(c).Fill().AlignItems(ui.Stretch).Children(func() {
			if !m.sidebarCollapsed {
				pane := ui.Column(c).Width(sidebar)
				if !vibrant {
					pane.Background(p.Sidebar)
				}
				pane.Children(func() {
					m.paneHeader(c).Padding(0, 8).Gap(4).Children(func() { m.leadingButtons(c, false) })
					ui.Column(c).Grow(1).MinHeight(0).Children(func() {
						if m.isSettings() {
							m.settingsSidebar(c)
						} else {
							m.chatsSidebar(c)
						}
					})
				})
				ui.Box(c).Width(1).Background(p.ToolbarLine)
			}
			ui.Column(c).Grow(1).MinWidth(0).Background(p.Content).Children(func() {
				// A chat runs under the header, as the Mac's transcript runs under the titlebar; the
				// other panes start below it.
				underHeader := chatID != "" && !m.isSettings()
				body := ui.Column(c).Grow(1).MinHeight(0)
				if !underHeader {
					// What scrolls under the header fades into it.
					body.Margin(headerHeight, 0, 0, 0).DrawOver(func(painter *ui.Painter, r ui.Rect) {
						painter.FillGradient(ui.Rect{X: r.X, Y: r.Y, W: r.W, H: 10}, ui.LinearGradient{From: p.Content, To: p.Content.Alpha(0), Angle: 180}, 0)
					})
				}
				body.Children(func() { m.content(c, chatID) })
				var edge ui.Element
				if underHeader {
					edge = ui.Box(c).Absolute().Top(0).Left(0).Right(0).Height(headerHeight).PassThrough()
				}
				header := m.contentHeader(c, chatID, title, subtitle, inspector == 0)
				if underHeader && m.chat != nil {
					edge.Material(headerEdge{scroll: m.chat.transcriptScroll, hovered: header.Hovered(), background: p.Content})
				}
			})
			if inspector > 0 {
				ui.Box(c).Width(1).Background(p.ToolbarLine)
				ui.Column(c).Width(inspector).Background(p.Inspector).Children(func() {
					bar := c.TitleBar()
					m.paneHeader(c).Padding(0, max(10, bar.Right+8), 0, 12).Children(func() {
						ui.Spacer(c)
						m.inspectorToggle(c)
					})
					ui.Column(c).Grow(1).MinHeight(0).Children(func() {
						if chatID != "" {
							m.inspectorView(c, chatID)
						}
					})
				})
			}
		})
		if !m.sidebarCollapsed {
			m.divider(c, sidebar, true)
		}
		if inspector > 0 {
			m.divider(c, width-inspector-1, false)
		}
	})
	m.palette.view(c, m)
	refreshMenuBar()
}

// paneHeader is a pane's header: where the title bar would be, under the window controls, which
// drags the window; the buttons in it stay buttons.
func (m *mainWindow) paneHeader(c *ui.Context) ui.Element {
	return ui.Row(c).Height(headerHeight).Gap(8).Padding(0, 12).MinWidth(0).DragWindow()
}

// divider is a drag handle between two panes, at x. The sidebar keeps 232 to 340 DIPs, the
// inspector 268 to 320; dragging the sidebar's well past its narrowest collapses it, as a split
// view's collapsible pane.
func (m *mainWindow) divider(c *ui.Context, x float32, sidebar bool) {
	key := "inspector-divider"
	if sidebar {
		key = "sidebar-divider"
	}
	h := ui.Box(c.Key(key)).Absolute().Top(0).Bottom(0).Left(x - 3).Width(7).Cursor(ui.CursorResizeEW).Label(L("Divider"))
	dragging := ui.Local(h, "drag", func() dividerDrag { return dividerDrag{} })
	// Dragged registers capture and stays held outside the handle; Pressed only covers its bounds.
	dx, _, held := h.Dragged()
	if held {
		if !dragging.active {
			dragging.active = true
			dragging.offset = 0
			if sidebar {
				dragging.from = m.sidebarWidth
			} else {
				dragging.from = m.inspectorWidth
			}
		}
		dragging.offset += dx
		if sidebar {
			wanted := dragging.from + dragging.offset
			if wanted < sidebarMin/2 {
				dragging.active = false
				m.toggleSidebar()
				return
			}
			m.sidebarWidth = clamp(wanted, sidebarMin, sidebarMax)
		} else {
			m.inspectorWidth = clamp(dragging.from-dragging.offset, inspectorMin, inspectorMax)
		}
	} else if dragging.active {
		dragging.active = false
		if sidebar {
			width := int(m.sidebarWidth)
			prefs.update(PreferencesPatch{SidebarWidth: &width})
		} else {
			width := int(m.inspectorWidth)
			prefs.update(PreferencesPatch{InspectorWidth: &width})
		}
	}
}

type dividerDrag struct {
	active bool
	from   float32
	offset float32
}

// leadingButtons are the sidebar and Create buttons, after the traffic lights on macOS, or the
// window controls on Linux when the desktop puts them at the left. They sit in the sidebar's
// header, or in the content's while the sidebar is collapsed.
func (m *mainWindow) leadingButtons(c *ui.Context, inContent bool) {
	bar := c.TitleBar()
	row := ui.Row(c).Gap(4)
	if bar.Left > 0 {
		// The traffic lights end 79 in, and AppKit starts the accessory's first button at 96.
		inset := bar.Left + 4
		if inContent {
			inset = bar.Left
		}
		row.Margin(0, 0, 0, inset)
	}
	row.Children(func() {
		if hoverButton(c, hoverButtonOptions{Symbol: "sidebar.leading", Tooltip: L("Toggle Sidebar (%@)", shortcutText("CmdOrCtrl+B")), Label: L("Toggle Sidebar")}).Clicked() {
			m.toggleSidebar()
		}
		// Creating bots and chats belongs to the chats; Settings hides it. A press opens its
		// menu, as the Mac button's own does.
		if !m.isSettings() {
			create := hoverButton(c, hoverButtonOptions{Symbol: "plus", Tooltip: L("Create")})
			create.Menu(func(menu *ui.Menu) {
				if menu.Item(L("Create New Bot…")).Chosen() {
					runCommand("newBot")
				}
				if menu.Item(L("Create Group Chat…")).Chosen() {
					runCommand("newGroupChat")
				}
				menu.Separator()
				if menu.Item(L("Pair a Device…")).Chosen() {
					runCommand("pairDevice")
				}
			})
		}
	})
}

func (m *mainWindow) inspectorToggle(c *ui.Context) {
	if hoverButton(c, hoverButtonOptions{Symbol: "sidebar.trailing", Tooltip: L("Toggle Inspector (%@)", shortcutText("CmdOrCtrl+Shift+B")), Label: L("Toggle Inspector")}).Clicked() {
		m.toggleInspector()
	}
}

// contentHeader is the content's header: back and forward in Settings, the title, the Device
// picker on the Device panes, and the chat's running tasks. The inspector's toggle sits here while
// the inspector is closed, and in the inspector's header while it is open; the rightmost header
// keeps clear of the window controls on Windows and Linux.
func (m *mainWindow) contentHeader(c *ui.Context, chatID, title, subtitle string, rightmost bool) ui.Element {
	p := colors(c)
	header := m.paneHeader(c.Key("content-header")).Absolute().Top(0).Left(0).Right(0)
	if rightmost {
		header.Padding(0, max(12, c.TitleBar().Right+8), 0, 12)
	}
	header.Children(func() {
		if m.sidebarCollapsed {
			m.leadingButtons(c, true)
		}
		if m.isSettings() {
			ui.Row(c).Gap(2).Children(func() {
				if hoverButton(c, hoverButtonOptions{Symbol: "chevron.left", Tooltip: L("Back"), Disabled: !m.canGoBack()}).Clicked() {
					m.goBack()
				}
				if hoverButton(c, hoverButtonOptions{Symbol: "chevron.right", Tooltip: L("Forward"), Disabled: !m.canGoForward()}).Clicked() {
					m.goForward()
				}
			})
		}
		ui.Column(c).Shrink(1).MinWidth(0).Justify(ui.Center).Children(func() {
			ui.Text(c, title).FontSize(13).FontWeight(700).FixedLineHeight(16).SingleLine()
			if subtitle != "" {
				ui.Text(c, subtitle).FontSize(11).FixedLineHeight(14).TextColor(p.Label2).SingleLine()
			}
		})
		ui.Spacer(c)
		if m.isSettings() {
			if m.selection.Pane.IsDeviceScoped() {
				m.devicePicker(c)
			}
			return
		}
		m.runningTasksButton(c, chatID)
		m.attentionButton(c)
		if rightmost {
			m.inspectorToggle(c)
		}
	})
	return header
}

// headerEdge is the hard scroll edge under a chat's header, as AppKit's titlebar pocket draws it:
// none while the transcript rests at its top and the pointer is elsewhere, frosting what scrolled
// under the header, with a hairline below it, once the transcript scrolls or the pointer is over
// the header. It reads the offset as it paints, after the frame laid the transcript out.
type headerEdge struct {
	scroll     *ui.ScrollState
	hovered    bool
	background ui.Color
}

func (e headerEdge) PaintMaterial(p *ui.Painter, box ui.Rect, radii [4]float32) {
	if e.hovered || e.scroll != nil && e.scroll.Y > 0 {
		glass.ScrollEdge{Hard: true, Background: e.background}.PaintMaterial(p, box, radii)
	}
}

// devicePicker is the Device the Plugins, Bots, and Devices panes show; the pick holds across
// them. Pairing is a command, so the pop-up goes on showing the picked Device.
func (m *mainWindow) devicePicker(c *ui.Context) {
	var options []popUpOption
	for _, device := range store.Devices {
		label := device.Name
		if device.IsThisDevice {
			label = L("%@ (This computer)", device.Name)
		}
		options = append(options, popUpOption{Value: device.ID, Label: label, Symbol: device.Symbol()})
	}
	picked, changed, extra := popUpButton(c, popUp{
		Options: options,
		Value:   m.settingsDeviceID,
		Style:   popUpPlain,
		Label:   L("Device"),
		Tooltip: L("The Device this page shows"),
		Extras:  []popUpExtra{{ID: "pairDevice", Label: L("Pair a Device…")}},
	})
	if changed {
		m.showSettingsDevice(picked)
	}
	if extra != "" {
		runCommand(extra)
	}
}

// content is the content pane's body: a chat, a settings pane, or a state.
func (m *mainWindow) content(c *ui.Context, chatID string) {
	switch {
	case m.isSettings():
		// The panes show while the CLI is not answering, since the relay URL and the CLI port live
		// there.
		m.settingsPage(c, m.selection.Pane)
	case !store.IsConnected:
		if store.IsStarting {
			loadingState(c)
		} else {
			offlineState(c, m)
		}
	case chatID != "":
		m.chatView(c, chatID)
	default:
		placeholderState(c, m)
	}
}

// shortcuts are the window's own keys: Ctrl+1 to Ctrl+9 (⌘ on a Mac) for the chat at that place
// in the sidebar, ready for a reply, and Escape leaving Settings when nothing in front takes it.
func (m *mainWindow) shortcuts(c *ui.Context) {
	if m.hasSheet() {
		return
	}
	for i, key := range chatNumberKeys {
		if c.Shortcut(ui.Cmd, key) && i < len(store.Chats) {
			m.open(store.Chats[i].ID)
			m.chatsList.revealSelection = true
		}
	}
	if m.isSettings() && !m.palette.open && c.Shortcut(0, ui.KeyEscape) {
		m.closeSettings()
	}
}

// MARK: - Actions

// newBot makes a bot: every bot has a direct chat, so creating one lands in that chat right away.
func (m *mainWindow) newBot() {
	m.presentNewBot(func(botID string) { m.open(store.DM(botID)) })
}

// newGroupChat makes a group chat and lands in it.
func (m *mainWindow) newGroupChat() {
	m.presentNewGroupChat(func(botIDs []string, title string) { m.open(store.CreateChat(model.ChatGroup, botIDs, title)) })
}

// addBotToChat adds a bot to a chat, from those that can still join it.
func (m *mainWindow) addBotToChat(chatID string) {
	chat := store.Chat(chatID)
	available := botsAvailableToAdd(chatID)
	if chat == nil || len(available) == 0 {
		beep()
		return
	}
	m.presentBotPicker(L("Add a bot to %@", store.Title(chat)), available, func(botID string) { store.AddBot(botID, chatID) })
}
