package main

import (
	"context"
	_ "embed"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"net/url"
	"runtime"
	"sync"
	"time"

	"github.com/egoist/mygo"
)

//go:embed assets/tray.png
var trayIcon []byte

//go:embed assets/tray-dev.png
var trayIconDev []byte

// CLIState is where the connection to the CLI stands, for the pages' loading and offline states.
type CLIState struct {
	// Connection is "disconnected", "connecting", or "connected".
	Connection string         `json:"connection"`
	Launcher   LauncherStatus `json:"launcher"`
	// Starting is the first connection still loading: the window shows a spinner, not the
	// offline recovery controls.
	Starting bool `json:"starting"`
}

// WindowState tells a page whether its window is where the user looks: in front, shown, not
// minimized. A reply the user watches arrive is neither pushed to their phone nor notified.
type WindowState struct {
	Focused   bool `json:"focused"`
	Visible   bool `json:"visible"`
	Minimized bool `json:"minimized"`
	// Maximized tells a page that draws its own title bar which window button to show:
	// Maximize or Restore.
	Maximized bool `json:"maximized"`
}

// stateOf is how a window stands, as WindowState answers and WindowStateChanged reports it.
func stateOf(win *mygo.Window) WindowState {
	return WindowState{Focused: win.IsFocused(), Visible: win.IsVisible(), Minimized: win.IsMinimized(), Maximized: win.IsMaximized()}
}

var (
	// CLIEvents carries every event frame of the CLI, `{ event, data }`, as it came.
	CLIEvents = mygo.NewEvent[jsontext.Value]("cli:event")
	// CLIStateChanged reports the connection and the launcher.
	CLIStateChanged = mygo.NewEvent[CLIState]("cli:state")
	// WindowStateChanged goes to a page when its window gains or loses the user's eye.
	WindowStateChanged = mygo.NewEvent[WindowState]("window:state")
	// OpenChat asks the main window to show a chat, from a clicked notification.
	OpenChat = mygo.NewEvent[string]("open:chat")
)

// appDelegate is the app's lifecycle, after the Mac app's AppDelegate: which window a launch
// opens (onboarding or the main window, decided by the identity only the CLI knows), the
// windows, the CLI's launcher and connection, notifications, and the tray icon that keeps the
// app reachable while its windows are closed.
type appDelegate struct {
	mu       sync.Mutex
	cli      *cliClient
	launcher *launcher

	main       *mygo.Window
	onboarding *mygo.Window
	settings   *mygo.Window
	// onboardingOverIdentity is onboarding opened again over an identity that still exists.
	// Create, restore, and pair refuse to run there, so closing it is Cancel: the main window it
	// hid comes back as it was.
	onboardingOverIdentity bool

	hasIdentity *bool
	starting    bool
	tray        *mygo.Tray
	quitting    bool
}

var app = &appDelegate{starting: true}

func (a *appDelegate) state() CLIState {
	a.mu.Lock()
	starting := a.starting
	a.mu.Unlock()
	if isMock() {
		return CLIState{Connection: "connected", Launcher: LauncherStatus{Kind: "running"}}
	}
	return CLIState{Connection: a.cli.currentState(), Launcher: a.launcher.currentStatus(), Starting: starting}
}

func (a *appDelegate) publishState() {
	CLIStateChanged.Broadcast(a.state())
}

func (a *appDelegate) didFinishLaunching() {
	startupTrace("did finish launching")
	a.cli = newCLIClient()
	a.launcher = newLauncher()
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
			mygo.RunOnMain(a.showMainWindowIfDue)
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
			}
			a.publishState()
		}
		a.launcher.onReady = a.cli.connect
		a.launcher.ensureRunning()
		// A slow or silent CLI leaves the user with the offline recovery controls. This bounds
		// the loading state, never the time until a window shows.
		time.AfterFunc(2500*time.Millisecond, a.finishStartup)
	}
	a.installTray()
	if a.mainWindowIsDue() {
		a.showMainWindow()
		startupTrace("window shown")
	}
}

func (a *appDelegate) finishStartup() {
	a.mu.Lock()
	changed := a.starting
	a.starting = false
	a.mu.Unlock()
	if changed {
		a.publishState()
		mygo.RunOnMain(a.showMainWindowIfDue)
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
	CLIEvents.Broadcast(jsontext.Value(frame))
	if name != "identity.changed" && name != "snapshot" {
		return
	}
	var payload struct {
		Data struct {
			HasIdentity *bool `json:"has_identity"`
		} `json:"data"`
	}
	if json.Unmarshal(frame, &payload) == nil && payload.Data.HasIdentity != nil {
		a.identityChanged(*payload.Data.HasIdentity)
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
	mygo.RunOnMain(func() {
		if has {
			if a.onboarding == nil && a.main == nil {
				a.showMainWindow()
			}
			return
		}
		// The account is gone: a main window hidden behind onboarding goes with it, and
		// onboarding opened over the account becomes the real thing.
		a.onboardingOverIdentity = false
		if a.main != nil {
			main := a.main
			a.main = nil
			main.Destroy()
		}
		if a.onboarding == nil {
			a.presentOnboarding()
		}
	})
}

// mainWindowIsDue is whether the main window is the one to show: with an identity, and before
// the CLI's first answer on a computer that had one, or once the app stops waiting (the offline
// recovery controls).
func (a *appDelegate) mainWindowIsDue() bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.hasIdentity != nil {
		return *a.hasIdentity
	}
	return prefs.get().HadIdentity || !a.starting
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

func (a *appDelegate) isMainWindow(win *mygo.Window) bool {
	return win != nil && a.main != nil && win.ID() == a.main.ID()
}

func (a *appDelegate) newWindow(options mygo.WindowOptions) *mygo.Window {
	win := mygo.NewWindow(options)
	report := func() { _ = WindowStateChanged.Emit(win, stateOf(win)) }
	win.OnFocus(report)
	win.OnBlur(report)
	win.OnShow(report)
	win.OnHide(report)
	win.OnMinimize(report)
	win.OnRestore(report)
	win.OnMaximize(report)
	win.OnUnmaximize(report)
	win.OnDOMReady(report)
	// The app's pages stay in the window; a link goes to the browser.
	win.OnWillNavigate(func(e *mygo.NavigateEvent) {
		if external(e.URL) {
			e.PreventDefault()
			go mygo.Shell.OpenExternal(e.URL)
		}
	})
	win.SetWindowOpenHandler(func(req mygo.WindowOpenRequest) *mygo.WindowOptions {
		if external(req.URL) {
			go mygo.Shell.OpenExternal(req.URL)
		}
		return nil
	})
	return win
}

// external is a web or mail link, not one of the app's pages or the dev server's.
func external(raw string) bool {
	u, err := url.Parse(raw)
	if err != nil {
		return false
	}
	switch u.Scheme {
	case "mailto":
		return true
	case "http", "https":
		host := u.Hostname()
		return host != "localhost" && host != "127.0.0.1" && host != "mygo.localhost" && host != fileScheme+".localhost"
	}
	return false
}

func (a *appDelegate) showMainWindow() {
	if a.main == nil {
		startupTrace("window construction started")
		options := mygo.WindowOptions{
			Title:           appName(),
			URL:             "/",
			Width:           1180,
			Height:          760,
			MinWidth:        860,
			MinHeight:       520,
			StateKey:        "main",
			BackgroundColor: "light-dark(#ffffff, #1c1c1c)",
		}
		// The page's panes carry the title bar: their headers hold the title, the buttons, and
		// the drag regions. On macOS the traffic lights stay, in the sidebar's header, where the
		// Mac app's toolbar puts them: the close button 19 points in and down, centered in the
		// 52-point header. Elsewhere the page draws the window buttons and the menu too.
		if runtime.GOOS == "darwin" {
			options.TitleBarStyle = mygo.TitleBarHidden
			options.TrafficLightPosition = &mygo.Point{X: 19, Y: 19}
		} else {
			options.Frameless = true
		}
		win := a.newWindow(options)
		win.OnClose(func(e *mygo.CloseEvent) {
			if a.quitting || !a.keepsRunning() || a.main == nil || win.ID() != a.main.ID() {
				return
			}
			// The window goes away and comes back as it was, from the tray or a new launch.
			e.PreventDefault()
			win.Hide()
		})
		win.OnClosed(func() {
			if a.main != nil && a.main.ID() == win.ID() {
				a.main = nil
			}
		})
		a.main = win
	}
	if a.main.IsMinimized() {
		a.main.Restore()
	}
	a.main.Show()
	a.main.Focus()
	startupTrace("window controller shown")
}

func (a *appDelegate) presentOnboarding() {
	win := a.newWindow(mygo.WindowOptions{
		Title:           appName(),
		URL:             "/onboarding",
		Width:           660,
		Height:          560,
		UseContentSize:  true,
		DisableResize:   true,
		DisableMaximize: true,
		BackgroundColor: "light-dark(#f5f5f5, #262626)",
	})
	win.OnClose(func(e *mygo.CloseEvent) {
		if a.quitting || a.onboarding == nil || win.ID() != a.onboarding.ID() {
			return
		}
		// Closing onboarding opened again over an identity is Cancel.
		if a.onboardingOverIdentity {
			mygo.RunOnMain(a.endOnboarding)
			return
		}
		// Any other onboarding stays the app's window, which the tray or a new launch brings back.
		if a.keepsRunning() {
			e.PreventDefault()
			win.Hide()
		}
	})
	win.OnClosed(func() {
		if a.onboarding != nil && a.onboarding.ID() == win.ID() {
			a.onboarding = nil
		}
	})
	a.onboarding = win
	win.Show()
	win.Focus()
}

// endOnboarding closes onboarding and the small settings window, and the main window takes
// over: the same one, as it was, when onboarding hid it.
func (a *appDelegate) endOnboarding() {
	if a.onboarding == nil {
		return
	}
	onboarding := a.onboarding
	a.onboarding = nil
	a.onboardingOverIdentity = false
	onboarding.Destroy()
	if a.settings != nil {
		settings := a.settings
		a.settings = nil
		settings.Destroy()
	}
	a.showMainWindow()
}

// showOnboarding opens onboarding again, from Settings › Advanced. It hides the main window
// rather than closing it, so Cancel brings back the pane, chat, and history there.
func (a *appDelegate) showOnboarding() {
	if a.onboarding != nil {
		a.onboarding.Show()
		a.onboarding.Focus()
		return
	}
	a.mu.Lock()
	a.onboardingOverIdentity = a.hasIdentity != nil && *a.hasIdentity
	a.mu.Unlock()
	if a.main != nil {
		a.main.Hide()
	}
	a.presentOnboarding()
}

// showSettings is Settings from the menu: a mode of the main window, or while onboarding is up,
// where there is no main window, a small window of General and Advanced.
func (a *appDelegate) showSettings() {
	if a.onboarding == nil {
		a.showMainWindow()
		_ = MenuCommand.Emit(a.main, "settings")
		return
	}
	if a.settings == nil {
		win := a.newWindow(mygo.WindowOptions{
			Title:           appName(),
			URL:             "/settings-window",
			Width:           560,
			Height:          480,
			UseContentSize:  true,
			DisableResize:   true,
			DisableMaximize: true,
			BackgroundColor: "light-dark(#ffffff, #1c1c1c)",
		})
		win.OnClosed(func() {
			if a.settings != nil && a.settings.ID() == win.ID() {
				a.settings = nil
			}
		})
		a.settings = win
	}
	a.settings.Show()
	a.settings.Focus()
}

// reopen is a click on the tray icon or a second launch: onboarding until it finishes, then the
// main window. A launch still waiting for the CLI's answer shows its window when the answer comes.
func (a *appDelegate) reopen() {
	if a.onboarding != nil {
		a.onboarding.Show()
		a.onboarding.Focus()
	} else if a.mainWindowIsDue() {
		a.showMainWindow()
	}
}

// menuCommand routes a menu bar item: the app's own commands here, the rest to the page, with
// the main window brought up first for the commands that open it.
func (a *appDelegate) menuCommand(spec MenuItemSpec, win *mygo.Window) {
	switch spec.ID {
	case "settings":
		a.showSettings()
		return
	case "showOnboarding":
		a.showOnboarding()
		return
	case "quit":
		mygo.App.Quit()
		return
	}
	if spec.OpensMain {
		if a.onboarding != nil {
			return
		}
		a.showMainWindow()
		_ = MenuCommand.Emit(a.main, spec.ID)
		return
	}
	target := win
	if target == nil || target.IsDestroyed() {
		target = a.main
	}
	if target != nil {
		_ = MenuCommand.Emit(target, spec.ID)
	}
}

// openChat brings the app forward on a chat, from a clicked notification.
func (a *appDelegate) openChat(chatID string) {
	mygo.RunOnMain(func() {
		if a.onboardingOverIdentity {
			a.endOnboarding()
		}
		if a.onboarding != nil {
			a.onboarding.Show()
			a.onboarding.Focus()
			return
		}
		a.showMainWindow()
		_ = OpenChat.Emit(a.main, chatID)
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
	tray, err := mygo.NewTray(mygo.TrayOptions{
		Icon:    icon,
		ToolTip: appName(),
		Menu:    a.trayMenu("Open "+appName(), "Quit "+appName()),
	})
	if err != nil {
		return
	}
	a.tray = tray
}

func (a *appDelegate) trayMenu(open, quit string) *mygo.Menu {
	return mygo.NewMenu([]*mygo.MenuItem{
		{Label: open, Click: func(*mygo.MenuItem, *mygo.Window) { a.reopen() }},
		mygo.Separator(),
		{Label: quit, Click: func(*mygo.MenuItem, *mygo.Window) { mygo.App.Quit() }},
	})
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
