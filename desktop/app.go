package main

import (
	"context"
	_ "embed"
	"encoding/json"
	"runtime"
	"sync"
	"time"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

//go:embed assets/tray.png
var trayIcon []byte

//go:embed assets/tray-dev.png
var trayIconDev []byte

// store is the app's model: one for every window, on the main thread.
var store *model.Store

// appDelegate is the app's lifecycle, after the Mac app's AppDelegate: which window a launch
// opens (onboarding or the main window, decided by the identity only the CLI knows), the
// windows, the CLI's launcher and connection, notifications, and the tray icon that keeps the
// app reachable while its windows are closed.
type appDelegate struct {
	mu       sync.Mutex
	cli      *cliClient
	launcher *launcher

	main       *mainWindow
	onboarding *onboardingWindow
	settings   *settingsWindow
	// onboardingOverIdentity is onboarding opened again over an identity that still exists.
	// Create, restore, and pair refuse to run there, so closing it is Cancel: the main window it
	// hid comes back as it was.
	onboardingOverIdentity bool

	hasIdentity *bool
	starting    bool
	// stoppedWaiting is a launch done waiting for the CLI's first answer: the CLI failed to start,
	// or answerWait passed. Until then a computer without an identity opens no window, so
	// onboarding never replaces a main window that just appeared.
	stoppedWaiting bool
	tray           *mygo.Tray
	trayWords      [2]string
	quitting       bool
	notifier       *notifier
}

// answerWait is how long a computer without an identity waits for the CLI's first answer before
// the main window opens on the offline recovery controls. It outlasts the loading state's 2.5
// seconds, which a CLI's first start (a new binary, a new database) can take on its own.
const answerWait = 10 * time.Second

var app = &appDelegate{starting: true}

// cliTransport is the store's way to the CLI: the app's one websocket.
type cliTransport struct{ client *cliClient }

func (t cliTransport) Request(method string, params any) (json.RawMessage, error) {
	raw, err := json.Marshal(params)
	if err != nil {
		return nil, err
	}
	result, err := t.client.request(context.Background(), method, raw)
	return json.RawMessage(result), err
}

func (t cliTransport) Reconnect() { t.client.reconnect() }

func (a *appDelegate) state() model.CLIState {
	a.mu.Lock()
	starting := a.starting
	a.mu.Unlock()
	return model.CLIState{Connection: a.cli.currentState(), Launcher: a.launcher.currentStatus(), Starting: starting}
}

func (a *appDelegate) publishState() {
	state := a.state()
	post(func() { store.CLIStateChanged(state) })
}

func (a *appDelegate) didFinishLaunching() {
	startupTrace("did finish launching")
	l10n.Set(prefs.get().AppLanguage, mygo.App.Locale())
	a.cli = newCLIClient()
	a.launcher = newLauncher()
	store = model.NewStore(cliTransport{a.cli}, post, isMock())
	a.notifier = newNotifier()
	store.Subscribe(func(event model.Event) {
		a.storeChanged(event)
		if a.main != nil {
			a.main.storeChanged(event)
		}
		a.notifier.storeChanged(event)
		invalidateWindows()
	})
	store.Start()
	if isMock() {
		yes := true
		a.hasIdentity = &yes
		a.starting = false
	} else {
		a.cli.onState = func(state string) {
			if state == "connected" {
				go a.askIdentity()
			}
			a.publishState()
		}
		a.cli.onEvent = a.cliEvent
		a.cli.onReconnectNeeded = a.launcher.ensureRunning
		a.launcher.onStatus = func(status LauncherStatus) {
			// A child that is starting or restarting owns the next connection attempt.
			if status.Kind == "starting" || status.Kind == "failed" {
				a.cli.disconnect()
			}
			if status.Kind == "failed" {
				a.finishStartup()
				a.stopWaiting()
			}
			a.publishState()
		}
		a.launcher.onReady = a.cli.connect
		a.launcher.ensureRunning()
		// A slow or silent CLI leaves the user with the offline recovery controls: in the main
		// window that is up once the loading state ends, and on a computer without an identity,
		// which opens its window on the CLI's answer, once the app stops waiting for it.
		time.AfterFunc(2500*time.Millisecond, a.finishStartup)
		time.AfterFunc(answerWait, a.stopWaiting)
	}
	a.installTray()
	installMenuBar()
	if a.mainWindowIsDue() {
		a.showMainWindow()
		startupTrace("window shown")
	}
}

// storeChanged keeps what the app shows outside its windows in step with the store: the unread
// badge and the tray's words.
func (a *appDelegate) storeChanged(event model.Event) {
	a.presentPendingTemplateLink()
	switch event.Kind {
	case model.EventIdentityChanged, model.EventSnapshotReplaced, model.EventChatsChanged:
		count := 0
		if store.HasIdentity == nil || *store.HasIdentity {
			for _, chat := range store.Chats {
				count += chat.UnreadCount
			}
		}
		setBadge(count)
	}
}

func (a *appDelegate) finishStartup() {
	a.mu.Lock()
	changed := a.starting
	a.starting = false
	a.mu.Unlock()
	if changed {
		a.publishState()
	}
}

// stopWaiting gives up on the CLI's first answer: a computer still waiting for it to pick a
// window opens the main window, whose recovery controls say why the CLI is not answering.
func (a *appDelegate) stopWaiting() {
	a.mu.Lock()
	changed := !a.stoppedWaiting
	a.stoppedWaiting = true
	a.mu.Unlock()
	if changed {
		post(a.showMainWindowIfDue)
	}
}

// askIdentity asks a CLI that just answered whether it holds an identity, which decides between
// onboarding and the main window.
func (a *appDelegate) askIdentity() {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	result, err := a.cli.request(ctx, "hello", nil)
	if err == nil {
		var hello struct {
			HasIdentity bool `json:"has_identity"`
		}
		if json.Unmarshal(result, &hello) == nil {
			a.identityChanged(hello.HasIdentity)
		}
	}
	a.finishStartup()
}

func (a *appDelegate) cliEvent(name string, frame []byte) {
	var payload struct {
		Data json.RawMessage `json:"data"`
	}
	_ = json.Unmarshal(frame, &payload)
	post(func() { store.HandleEvent(name, payload.Data) })
	if name != "identity.changed" && name != "snapshot" {
		return
	}
	var identity struct {
		HasIdentity *bool `json:"has_identity"`
	}
	if json.Unmarshal(payload.Data, &identity) == nil && identity.HasIdentity != nil {
		a.identityChanged(*identity.HasIdentity)
	}
}

// identityChanged opens the window the identity calls for. Onboarding closes itself when it
// finishes, so the phrase step is never yanked away by the `identity.changed` that precedes
// the create response.
func (a *appDelegate) identityChanged(has bool) {
	a.mu.Lock()
	same := a.hasIdentity != nil && *a.hasIdentity == has
	a.hasIdentity = &has
	a.mu.Unlock()
	if same {
		return
	}
	prefs.update(PreferencesPatch{HadIdentity: &has})
	post(func() {
		if has {
			if a.onboarding == nil && a.main == nil {
				a.showMainWindow()
			}
			return
		}
		// The account is gone: a main window hidden behind onboarding goes with it, and
		// onboarding opened over the account becomes the real thing. Onboarding opens before the
		// main window closes: without a tray, the app quits once its last window is gone.
		a.onboardingOverIdentity = false
		setBadge(0)
		if a.onboarding == nil {
			a.presentOnboarding()
		}
		if a.main != nil {
			main := a.main
			a.main = nil
			main.win.Destroy()
		}
	})
}

// mainWindowIsDue is whether the main window is the one to show: with an identity, and before
// the CLI's first answer on a computer that had one, or once the app stops waiting for that
// answer (the offline recovery controls).
func (a *appDelegate) mainWindowIsDue() bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.hasIdentity != nil {
		return *a.hasIdentity
	}
	return prefs.get().HadIdentity || a.stoppedWaiting
}

func (a *appDelegate) showMainWindowIfDue() {
	if a.main == nil && a.onboarding == nil && a.mainWindowIsDue() {
		a.showMainWindow()
	}
}

// keepsRunning is whether closing the windows leaves the app running: on macOS, as apps do
// there, and wherever the tray icon brings them back. Bots on this computer run as long as it does.
func (a *appDelegate) keepsRunning() bool {
	return runtime.GOOS == "darwin" || a.tray != nil
}

// newWindow opens a window of native UI. On Windows and Linux its menu bar stays out of sight
// until Alt or F10 takes the keyboard to it, and its shortcuts work all along. The Mac's is at the
// top of the screen.
func (a *appDelegate) newWindow(w *appWindow, options mygo.WindowOptions, view func(c *ui.Context)) *mygo.Window {
	options.AutoHideMenuBar = true
	options.Content = ui.View(w.frame(view))
	win := mygo.NewWindow(options)
	w.win = win
	report := func() {
		w.focused = win.IsFocused() && win.IsVisible() && !win.IsMinimized()
		a.notifier.watchingChanged()
		w.invalidate()
	}
	win.OnFocus(report)
	win.OnBlur(report)
	win.OnShow(report)
	win.OnHide(report)
	win.OnMinimize(report)
	win.OnRestore(report)
	return win
}

func (a *appDelegate) showMainWindow() {
	if a.main == nil {
		startupTrace("window construction started")
		m := newMainWindow()
		options := mygo.WindowOptions{
			Title:           appName(),
			Width:           1180,
			Height:          760,
			MinWidth:        860,
			MinHeight:       520,
			StateKey:        "main",
			BackgroundColor: "light-dark(#ffffff, #1c1c1c)",
			// The panes carry the title bar: their headers hold the title, the buttons, and the
			// drag regions, and the window controls sit over them. On Windows and Linux the
			// controls go in a top corner and fill the 52-pixel headers or center in them.
			TitleBarStyle:  mygo.TitleBarHidden,
			TitleBarHeight: 52,
			// The sidebar shows the window's material, as the Mac's: on macOS, and Mica on
			// Windows 11; elsewhere it draws its own color (Context.Vibrancy).
			Vibrancy: mygo.VibrancySidebar,
		}
		// The traffic lights sit in the sidebar's header where the Mac app's toolbar puts them:
		// the close button 19 points in and down, centered in the 52-point header.
		if runtime.GOOS == "darwin" {
			options.TrafficLightPosition = &mygo.Point{X: 19, Y: 19}
		}
		win := a.newWindow(&m.appWindow, options, m.view)
		win.OnFocus(func() {
			if id := m.selectedChatID(); id != "" {
				store.MarkRead(id)
			}
		})
		win.OnClose(func(e *mygo.CloseEvent) {
			if a.quitting || !a.keepsRunning() || a.main == nil || win.ID() != a.main.win.ID() {
				return
			}
			// The window goes away and comes back as it was, from the tray or a new launch.
			e.PreventDefault()
			win.Hide()
		})
		win.OnClosed(func() {
			if a.main != nil && a.main.win.ID() == win.ID() {
				a.main = nil
			}
		})
		a.main = m
		m.restoreSelection()
	}
	win := a.main.win
	if win == nil {
		// A main window built without one of its own, as tests build it.
		return
	}
	if win.IsMinimized() {
		win.Restore()
	}
	win.Show()
	win.Focus()
	startupTrace("window controller shown")
}

func (a *appDelegate) presentOnboarding() {
	o := newOnboardingWindow()
	win := a.newWindow(&o.appWindow, mygo.WindowOptions{
		Title:           appName(),
		Width:           660,
		Height:          560,
		UseContentSize:  true,
		DisableResize:   true,
		DisableMinimize: true,
		DisableMaximize: true,
		BackgroundColor: "light-dark(#ffffff, #262628)",
		// As the Mac app's: the page fills the window, under the window controls (the close
		// button alone on Windows, as a window that can neither minimize nor maximize has), and
		// its background drags the window.
		TitleBarStyle: mygo.TitleBarHidden,
	}, o.view)
	win.OnClose(func(e *mygo.CloseEvent) {
		if a.quitting || a.onboarding == nil || win.ID() != a.onboarding.win.ID() {
			return
		}
		// Closing onboarding opened again over an identity is Cancel.
		if a.onboardingOverIdentity {
			post(a.endOnboarding)
			return
		}
		// Any other onboarding stays the app's window, which the tray or a new launch brings back.
		if a.keepsRunning() {
			e.PreventDefault()
			win.Hide()
		}
	})
	win.OnClosed(func() {
		if a.onboarding != nil && a.onboarding.win.ID() == win.ID() {
			a.onboarding = nil
		}
	})
	a.onboarding = o
	win.SetMenu(windowMenuBar(windowOther))
	win.Show()
	win.Focus()
}

// endOnboarding closes onboarding and the small settings window, and the main window takes over:
// the same one, as it was, when onboarding hid it. The main window shows first: without a tray,
// the app quits once its last window is gone.
func (a *appDelegate) endOnboarding() {
	if a.onboarding == nil {
		return
	}
	onboarding := a.onboarding
	a.onboarding = nil
	a.onboardingOverIdentity = false
	a.showMainWindow()
	onboarding.win.Destroy()
	if a.settings != nil {
		settings := a.settings
		a.settings = nil
		settings.win.Destroy()
	}
}

// showOnboarding opens onboarding again, from Settings › Advanced. It hides the main window
// rather than closing it, so Cancel brings back the pane, chat, and history there.
func (a *appDelegate) showOnboarding() {
	if a.onboarding != nil {
		a.onboarding.win.Show()
		a.onboarding.win.Focus()
		return
	}
	a.mu.Lock()
	a.onboardingOverIdentity = a.hasIdentity != nil && *a.hasIdentity
	a.mu.Unlock()
	if a.main != nil {
		a.main.win.Hide()
	}
	a.presentOnboarding()
}

// showSettings is Settings from the menu: a mode of the main window, or while onboarding is up,
// where there is no main window, a small window of General and Advanced.
func (a *appDelegate) showSettings() {
	if a.onboarding == nil {
		a.showMainWindow()
		a.main.showSettings("")
		return
	}
	if a.settings == nil {
		s := newSettingsWindow()
		win := a.newWindow(&s.appWindow, mygo.WindowOptions{
			Title:           L("Settings") + " - " + appName(),
			Width:           560,
			Height:          480,
			UseContentSize:  true,
			DisableResize:   true,
			DisableMaximize: true,
			BackgroundColor: "light-dark(#ffffff, #1c1c1c)",
		}, s.view)
		win.OnClosed(func() {
			if a.settings != nil && a.settings.win.ID() == win.ID() {
				a.settings = nil
			}
		})
		win.SetMenu(windowMenuBar(windowOther))
		a.settings = s
	}
	a.settings.win.Show()
	a.settings.win.Focus()
}

// reopen is a click on the tray icon or a second launch: onboarding until it finishes, then the
// main window. A launch still waiting for the CLI's answer shows its window when the answer comes.
func (a *appDelegate) reopen() {
	if a.onboarding != nil {
		a.onboarding.win.Show()
		a.onboarding.win.Focus()
	} else if a.mainWindowIsDue() {
		a.showMainWindow()
	}
}

// openChat brings the app forward on a chat, from a clicked notification.
func (a *appDelegate) openChat(chatID string) {
	post(func() {
		if a.onboardingOverIdentity {
			a.endOnboarding()
		}
		if a.onboarding != nil {
			a.onboarding.win.Show()
			a.onboarding.win.Focus()
			return
		}
		a.showMainWindow()
		a.main.open(chatID)
	})
}

// installTray puts the app in the notification area on Windows and Linux, where closing its
// windows would otherwise leave nothing to bring it back by, and bots here stop with the app.
func (a *appDelegate) installTray() {
	if runtime.GOOS == "darwin" {
		return
	}
	icon := trayIcon
	if isDevelopment() {
		icon = trayIconDev
	}
	a.trayWords = [2]string{L("Open %@", appName()), L("Quit %@", appName())}
	tray, err := mygo.NewTray(mygo.TrayOptions{
		Icon:    icon,
		ToolTip: appName(),
		Menu:    a.trayMenu(a.trayWords[0], a.trayWords[1]),
	})
	if err != nil {
		return
	}
	// A click on the icon brings the app back, as a click on its Dock icon does; the menu is on
	// the right button on Windows, and all a click shows on Linux.
	tray.OnClick(func() { post(a.reopen) })
	a.tray = tray
}

func (a *appDelegate) trayMenu(open, quit string) *mygo.Menu {
	return mygo.NewMenu([]*mygo.MenuItem{
		{Label: open, Click: func(*mygo.MenuItem, *mygo.Window) { a.reopen() }},
		mygo.Separator(),
		{Label: quit, Click: func(*mygo.MenuItem, *mygo.Window) { mygo.App.Quit() }},
	})
}

// languageChanged words what the app shows outside its windows in the language now in force.
func (a *appDelegate) languageChanged() {
	words := [2]string{L("Open %@", appName()), L("Quit %@", appName())}
	if a.tray != nil && words != a.trayWords {
		a.trayWords = words
		a.tray.SetMenu(a.trayMenu(words[0], words[1]))
	}
	installMenuBar()
	for _, w := range []*appWindow{onboardingAppWindow(), settingsAppWindow()} {
		if w != nil && w.win != nil {
			w.win.SetMenu(windowMenuBar(windowOther))
		}
	}
	if a.settings != nil {
		a.settings.win.SetTitle(L("Settings") + " - " + appName())
	}
	invalidateWindows()
}

func onboardingAppWindow() *appWindow {
	if app.onboarding == nil {
		return nil
	}
	return &app.onboarding.appWindow
}

func settingsAppWindow() *appWindow {
	if app.settings == nil {
		return nil
	}
	return &app.settings.appWindow
}

func (a *appDelegate) willTerminate() {
	a.quitting = true
	if a.launcher != nil {
		a.launcher.stop()
	}
	if a.cli != nil {
		a.cli.disconnect()
	}
}
