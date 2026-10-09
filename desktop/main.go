// Lorca for Windows and Linux: a MyGo app of native UI whose Go code launches and talks to the
// local CLI and draws the chat UI. It follows the macOS app (macos/) screen for screen.
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
	useUpdater()

	mygo.App.OnSecondInstance(func(args []string, workingDir string) { app.reopen() })
	// Open in Lorca on a shared bot's page: lorca://t/<id>#<key> (lorca-dev:// for Lorca Dev),
	// with the launch or from a second instance.
	mygo.App.OnOpenURL(func(link string) { post(func() { app.openTemplateLink(link) }) })
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
	mygo.App.WhenReady(app.didFinishLaunching)
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
}
