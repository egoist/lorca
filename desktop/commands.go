package main

import (
	"runtime"
	"slices"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// The app's commands, after the macOS app's menu bar: one table that becomes the windows' native
// menu bars (whose items keep their titles and enabled state as the selection changes) and the
// words and enabled state of the command palette's actions.

type command struct {
	id    string
	title func() string
	// accelerator is "CmdOrCtrl+Shift+N": Ctrl on Windows and Linux.
	accelerator string
	// role is a standard item the system handles: Close, Quit, Full Screen.
	role mygo.MenuRole
	// opensMain brings the main window up first when chosen elsewhere.
	opensMain bool
	enabled   func() bool
	// checked makes it a check box; nil for a plain item.
	checked func() bool
}

// whileSheet is what a sheet leaves working: the app's own commands, not the window's, as a sheet
// takes the window's keys on the Mac.
var whileSheet = []string{"quit", "about", "help", "architecture", "checkForUpdates", "fullScreen", "simulateOffline", "replayMock", "showOnboarding"}

// appCommands are what any window answers, the small Settings window and onboarding's too.
var appCommands = []string{"settings", "help", "architecture", "about", "checkForUpdates", "showOnboarding", "simulateOffline", "replayMock"}

func chatSelected() bool {
	m := app.main
	return m != nil && m.selection.ChatID != "" && store.Chat(m.selection.ChatID) != nil
}

func selectedChatIsGroup() bool {
	if !chatSelected() {
		return false
	}
	return store.Chat(app.main.selection.ChatID).IsGroup()
}

// commandTable is every command, in menu order.
var commandTable = []command{
	{id: "newBot", title: func() string { return L("New Bot…") }, accelerator: "CmdOrCtrl+N", opensMain: true},
	{id: "newGroupChat", title: func() string { return L("New Group Chat…") }, accelerator: "CmdOrCtrl+Shift+N", opensMain: true},
	{id: "newTask", title: func() string { return L("New Task…") }, enabled: chatSelected},
	{id: "marketplace", title: func() string { return L("Marketplace…") }, accelerator: "CmdOrCtrl+Shift+M", opensMain: true},
	{id: "pairDevice", title: func() string { return L("Pair a Device…") }, accelerator: "CmdOrCtrl+Shift+P", opensMain: true},
	{id: "settings", title: func() string { return L("Settings…") }, accelerator: "CmdOrCtrl+,"},
	{id: "closeWindow", title: func() string { return L("Close Window") }, accelerator: "CmdOrCtrl+W", role: mygo.RoleClose},
	{id: "quit", title: func() string { return L("Quit %@", appName()) }, accelerator: "CmdOrCtrl+Q", role: mygo.RoleQuit},
	{id: "find", title: func() string { return L("Find…") }, accelerator: "CmdOrCtrl+F", opensMain: true},
	{id: "palette", title: func() string { return L("Command Palette…") }, accelerator: "CmdOrCtrl+K", opensMain: true},
	{id: "attention", title: func() string { return L("Attention") }, accelerator: "CmdOrCtrl+Shift+A", opensMain: true, enabled: func() bool { return app.main == nil || !app.main.isSettings() }},
	{id: "toggleSidebar", title: func() string { return L("Toggle Sidebar") }, accelerator: "CmdOrCtrl+B"},
	{id: "toggleInspector", title: func() string { return L("Toggle Inspector") }, accelerator: "CmdOrCtrl+Shift+B"},
	{id: "scrollToLatest", title: func() string { return L("Scroll to Latest") }, accelerator: "CmdOrCtrl+J", enabled: chatSelected},
	{id: "fullScreen", title: func() string {
		if app.main != nil && app.main.win != nil && app.main.win.IsFullScreen() {
			return L("Exit Full Screen")
		}
		return L("Enter Full Screen")
	}, accelerator: "F11", role: mygo.RoleToggleFullScreen},
	{id: "addBot", title: func() string { return L("Add Bot…") }, accelerator: "CmdOrCtrl+Alt+B", enabled: func() bool {
		return chatSelected() && len(botsAvailableToAdd(app.main.selection.ChatID)) > 0
	}},
	{id: "renameChat", title: func() string { return L("Rename Chat…") }, accelerator: "CmdOrCtrl+R", enabled: selectedChatIsGroup},
	{id: "pinChat", title: func() string { return L("Pin Chat") }, accelerator: "CmdOrCtrl+P", enabled: chatSelected},
	{id: "newSkill", title: func() string { return L("New Skill…") }, enabled: func() bool {
		if !chatSelected() {
			return false
		}
		chat := store.Chat(app.main.selection.ChatID)
		_, ok := model.SkillScope(chat, store.BotsIn(chat))
		return ok
	}},
	{id: "stopResponding", title: func() string { return L("Stop Responding") }, accelerator: "CmdOrCtrl+.", enabled: chatSelected},
	{
		// The Mac's ⌃B has no counterpart here: Ctrl+B toggles the sidebar.
		id: "runInBackground", title: func() string { return L("Run Command in Background") },
		enabled: func() bool { return chatSelected() && len(store.ForegroundCommands(app.main.selection.ChatID)) > 0 },
	},
	{id: "deleteChat", title: func() string {
		if chatSelected() && store.Chat(app.main.selection.ChatID).IsDM() {
			return L("Delete Bot")
		}
		return L("Delete Chat")
	}, enabled: chatSelected},
	{id: "help", title: func() string { return L("Lorca Help") }, accelerator: "F1"},
	{id: "architecture", title: func() string { return L("Architecture Notes") }},
	{id: "checkForUpdates", title: func() string { return L("Check for Updates…") }, enabled: updatesEnabled},
	{id: "about", title: func() string { return L("About %@", appName()) }},
	{id: "simulateOffline", title: func() string {
		if store.IsMock {
			return "Simulate CLI Offline"
		}
		return "Reconnect to CLI"
	}, accelerator: "CmdOrCtrl+Alt+D", checked: func() bool { return store.IsMock && !store.IsConnected }},
	{id: "replayMock", title: func() string { return "Replay Mock Data" }, enabled: func() bool { return store.IsMock }},
	{id: "showOnboarding", title: func() string { return "Show Onboarding" }},
}

func commandByID(id string) *command {
	for i := range commandTable {
		if commandTable[i].id == id {
			return &commandTable[i]
		}
	}
	return nil
}

// isEnabled is whether a command can run now.
func (cmd *command) isEnabled() bool {
	return cmd.enabled == nil || cmd.enabled()
}

// runCommand runs a command, from the menu bar, the palette, or a button. The app's own commands
// run anywhere; the rest are the main window's, which comes up first for those that open it.
func runCommand(id string) {
	cmd := commandByID(id)
	if cmd != nil && !cmd.isEnabled() {
		return
	}
	switch id {
	case "settings":
		app.showSettings()
		return
	case "showOnboarding":
		app.showOnboarding()
		return
	case "quit":
		mygo.App.Quit()
		return
	case "help":
		showHelp(frontWindow())
		return
	case "architecture":
		showArchitecture(frontWindow())
		return
	case "about":
		showAbout(frontWindow())
		return
	case "checkForUpdates":
		checkForUpdates()
		return
	case "simulateOffline":
		if store.IsMock {
			store.SetConnected(!store.IsConnected)
		} else {
			store.Reconnect()
		}
		return
	case "replayMock":
		store.ResetMockData()
		return
	}
	if cmd != nil && cmd.opensMain {
		if app.onboarding != nil {
			return
		}
		app.showMainWindow()
	}
	m := app.main
	if m == nil {
		return
	}
	if m.hasSheet() && !slices.Contains(whileSheet, id) {
		return
	}
	// The palette's own shortcut opens it afresh; any other command closes it first.
	if id != "palette" {
		m.palette.close()
	}
	m.run(id)
}

// frontWindow is the window a note from the Help menu goes over.
func frontWindow() *appWindow {
	for _, w := range windows() {
		if w.focused {
			return w
		}
	}
	if app.main != nil {
		return &app.main.appWindow
	}
	if ws := windows(); len(ws) > 0 {
		return ws[0]
	}
	return nil
}

// shortcutText is how a shortcut reads here: "Ctrl+Shift+N" on Windows and Linux, "⇧⌘N" on a Mac.
func shortcutText(accelerator string) string {
	parts := strings.Split(accelerator, "+")
	key := parts[len(parts)-1]
	parts = parts[:len(parts)-1]
	if runtime.GOOS == "darwin" {
		glyphs := map[string]string{"Ctrl": "⌃", "Alt": "⌥", "Shift": "⇧", "CmdOrCtrl": "⌘", "Cmd": "⌘"}
		var out strings.Builder
		for _, modifier := range []string{"Ctrl", "Alt", "Shift", "CmdOrCtrl", "Cmd"} {
			if slices.Contains(parts, modifier) {
				out.WriteString(glyphs[modifier])
			}
		}
		if key == "Enter" {
			key = "Return"
		}
		out.WriteString(strings.ToUpper(key))
		return out.String()
	}
	names := map[string]string{"CmdOrCtrl": "Ctrl", "Cmd": "Win"}
	var out []string
	for _, modifier := range parts {
		if name, ok := names[modifier]; ok {
			out = append(out, name)
		} else {
			out = append(out, modifier)
		}
	}
	if len(key) == 1 {
		key = strings.ToUpper(key)
	}
	return strings.Join(append(out, key), "+")
}

// MARK: - The menu bar

type windowKind int

const (
	windowMain windowKind = iota
	windowOther
)

// menuStates are the menu bar's items as they were last set, by id, so a frame changes only what
// changed.
var menuStates = map[string]menuState{}

type menuState struct {
	label   string
	enabled bool
	checked bool
}

var appMenu *mygo.Menu

func (cmd *command) enabledIn(kind windowKind) bool {
	if kind == windowOther {
		return cmd.role != "" || slices.Contains(appCommands, cmd.id) && cmd.isEnabled()
	}
	if app.main != nil && app.main.hasSheet() && !slices.Contains(whileSheet, cmd.id) {
		return false
	}
	return cmd.isEnabled()
}

func menuItem(id string, kind windowKind) *mygo.MenuItem {
	cmd := commandByID(id)
	item := &mygo.MenuItem{
		ID:          id,
		Label:       cmd.title(),
		Accelerator: cmd.accelerator,
		Role:        cmd.role,
		Disabled:    !cmd.enabledIn(kind),
	}
	if cmd.checked != nil {
		item.Type = mygo.MenuItemCheckbox
		item.Checked = cmd.checked()
	}
	if cmd.role == "" {
		item.Click = func(*mygo.MenuItem, *mygo.Window) { runCommand(id) }
	}
	return item
}

func submenu(label string, items ...*mygo.MenuItem) *mygo.MenuItem {
	return &mygo.MenuItem{Label: label, Submenu: items}
}

// windowMenuBar is the menu bar, in the order the macOS app's has it; what belongs to the app menu
// there (Settings, Quit, About, Check for Updates) sits in File and Help, as on Windows and Linux.
func windowMenuBar(kind windowKind) *mygo.Menu {
	at := func(id string) *mygo.MenuItem { return menuItem(id, kind) }
	sep := mygo.Separator
	help := []*mygo.MenuItem{at("help"), at("architecture")}
	if updatesEnabled() {
		help = append(help, sep(), at("checkForUpdates"))
	}
	help = append(help, sep(), at("about"))
	return mygo.NewMenu([]*mygo.MenuItem{
		submenu(L("File"), at("newBot"), at("newGroupChat"), at("newTask"), sep(), at("marketplace"), sep(), at("pairDevice"), sep(), at("settings"), sep(), at("closeWindow"), at("quit")),
		submenu(L("Edit"),
			&mygo.MenuItem{Role: mygo.RoleUndo, Label: L("Undo")},
			&mygo.MenuItem{Role: mygo.RoleRedo, Label: L("Redo")},
			sep(),
			&mygo.MenuItem{Role: mygo.RoleCut, Label: L("Cut")},
			&mygo.MenuItem{Role: mygo.RoleCopy, Label: L("Copy")},
			&mygo.MenuItem{Role: mygo.RolePaste, Label: L("Paste")},
			&mygo.MenuItem{Role: mygo.RolePasteAndMatchStyle, Label: L("Paste and Match Style"), Accelerator: "CmdOrCtrl+Alt+Shift+V"},
			&mygo.MenuItem{Role: mygo.RoleSelectAll, Label: L("Select All")},
			sep(),
			at("find"),
		),
		submenu(L("View"), at("palette"), at("attention"), sep(), at("toggleSidebar"), at("toggleInspector"), sep(), at("scrollToLatest"), sep(), at("fullScreen")),
		submenu(L("Chat"), at("addBot"), at("renameChat"), at("pinChat"), at("newSkill"), sep(), at("stopResponding"), at("runInBackground"), sep(), at("deleteChat")),
		submenu(L("Window"), &mygo.MenuItem{Role: mygo.RoleMinimize, Label: L("Minimize")}, &mygo.MenuItem{Role: mygo.RoleZoom, Label: L("Zoom")}),
		submenu("Debug", at("simulateOffline"), at("replayMock"), sep(), at("showOnboarding")),
		submenu(L("Help"), help...),
	})
}

// installMenuBar puts the main window's menu bar up as the app's, which every window without one
// of its own shares, in the language in force.
func installMenuBar() {
	appMenu = windowMenuBar(windowMain)
	clear(menuStates)
	for i := range commandTable {
		menuStates[commandTable[i].id] = commandTable[i].state()
	}
	mygo.App.SetMenu(appMenu)
}

func (cmd *command) state() menuState {
	state := menuState{label: cmd.title(), enabled: cmd.enabledIn(windowMain)}
	if cmd.checked != nil {
		state.checked = cmd.checked()
	}
	return state
}

// refreshMenuBar keeps the menu bar's titles, checks, and enabled states in step with the store
// and the selection, after a frame of the main window.
func refreshMenuBar() {
	if appMenu == nil {
		return
	}
	for i := range commandTable {
		cmd := &commandTable[i]
		item := appMenu.ItemByID(cmd.id)
		if item == nil {
			continue
		}
		state := cmd.state()
		before := menuStates[cmd.id]
		if before == state {
			continue
		}
		menuStates[cmd.id] = state
		if before.label != state.label {
			item.SetLabel(state.label)
		}
		if before.enabled != state.enabled {
			item.SetEnabled(state.enabled)
		}
		if cmd.checked != nil && before.checked != state.checked {
			item.SetChecked(state.checked)
		}
	}
}

// chatNumberKeys are Ctrl+1 to Ctrl+9 (⌘ on a Mac): the chat at that place in the sidebar.
var chatNumberKeys = []ui.Key{ui.Key1, ui.Key2, ui.Key3, ui.Key4, ui.Key5, ui.Key6, ui.Key7, ui.Key8, ui.Key9}
