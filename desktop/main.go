// Lorca for Windows and Linux: a MyGo app whose Go side launches and talks to the local CLI, and
// whose pages (src/) are the chat UI. It follows the macOS app (macos/) screen for screen.
package main

import (
	"log"

	"github.com/egoist/mygo"
)

func main() {
	// Starting the app again brings the running one forward, as a click on the Dock icon does.
	if !mygo.App.RequestSingleInstanceLock() {
		return
	}
	prefs.load()
	applyAppearance(prefs.get().Appearance)

	mygo.Bind(CLI{}, Host{}, Files{}, Menus{}, &Notices{}, Prefs{})
	if err := mygo.Protocol.HandleFunc(fileScheme, serveFile); err != nil {
		log.Fatal(err)
	}

	mygo.App.OnSecondInstance(func(args []string, workingDir string) { app.reopen() })
	mygo.App.OnActivate(func(hasVisibleWindows bool) {
		if !hasVisibleWindows {
			app.reopen()
		}
	})
	mygo.App.OnWindowAllClosed(func() {
		if !app.keepsRunning() {
			mygo.App.Quit()
		}
	})
	mygo.App.OnBeforeQuit(func(*mygo.QuitEvent) { app.quitting = true })
	mygo.App.OnQuit(app.willTerminate)
	mygo.App.WhenReady(func() {
		app.didFinishLaunching()
		updates.start()
	})
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
}
