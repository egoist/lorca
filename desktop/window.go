package main

import (
	"sync"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// appWindow is one of the app's windows: the native window, the sheets up over it, and its title,
// which the taskbar and Alt+Tab show.
type appWindow struct {
	win       *mygo.Window
	sheets    []*sheet
	nextSheet int
	title     string
	// focused is the window being where the user looks: in front, shown, not minimized.
	focused bool
}

func (w *appWindow) invalidate() {
	if w != nil && w.win != nil && !w.win.IsDestroyed() {
		w.win.Invalidate()
	}
}

// setTitle sets the window's title: the page's title, then " - Lorca", or the app's name alone
// when the page's title is the app's name.
func (w *appWindow) setTitle(text string) {
	name := appName()
	title := name
	if text != "" && text != name {
		title = text + " - " + name
	}
	if title == w.title || w.win == nil {
		return
	}
	w.title = title
	w.win.SetTitle(title)
}

// frame wraps a window's view: the app's theme over the desktop's, the view, and the sheets over it.
func (w *appWindow) frame(view func(c *ui.Context)) func(c *ui.Context) {
	return func(c *ui.Context) {
		applyTheme(c)
		view(c)
		w.sheetsView(c)
	}
}

// windows are the app's windows that are open.
func windows() []*appWindow {
	var out []*appWindow
	if app.main != nil {
		out = append(out, &app.main.appWindow)
	}
	if app.onboarding != nil {
		out = append(out, &app.onboarding.appWindow)
	}
	if app.settings != nil {
		out = append(out, &app.settings.appWindow)
	}
	return out
}

// invalidateWindows draws every window anew, after the store or something they show changed.
func invalidateWindows() {
	for _, w := range windows() {
		w.invalidate()
	}
}

// posts are the functions waiting for the main thread, in the order they came: the CLI's events
// change the store in the order the CLI sent them.
var posts struct {
	sync.Mutex
	queue   []func()
	running bool
}

// post runs fn on the main thread after what runs there now, then draws the windows anew.
func post(fn func()) {
	posts.Lock()
	posts.queue = append(posts.queue, fn)
	if posts.running {
		posts.Unlock()
		return
	}
	posts.running = true
	posts.Unlock()
	go drainPosts()
}

func drainPosts() {
	for {
		posts.Lock()
		batch := posts.queue
		posts.queue = nil
		if len(batch) == 0 {
			posts.running = false
			posts.Unlock()
			return
		}
		posts.Unlock()
		mygo.RunOnMain(func() {
			for _, fn := range batch {
				fn()
			}
			invalidateWindows()
		})
	}
}
