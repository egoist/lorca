package main

import (
	"time"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/plugins/updater"
)

// useUpdater gives the app MyGo's update window, as Sparkle gives the macOS app its own: a daily
// check in the background, the release notes of a new version with Install Update, Remind Me
// Later and Skip This Version, the download's progress, and the offer to relaunch. It installs
// signed updates, by delta from the running version when the release has one. The user's choices
// are this computer's, in updater.json. The window speaks the language picked in Settings, else
// the system's.
func useUpdater() {
	mygo.Use(updater.New(updater.Options{Language: prefs.get().AppLanguage}))
	// A check that went through, the update window's checkbox, or a switch: General shows it.
	updater.OnChange(invalidateWindows)
}

// updatesEnabled is whether this build updates itself: not one that cannot write where it is
// installed, as `mygo dev`'s or the Debian package's in /opt.
func updatesEnabled() bool { return mygo.Updater.Enabled() }

// checkForUpdates opens the update window, which says what the check finds.
func checkForUpdates() { updater.CheckForUpdates() }

// updaterState is what Settings › General shows about updates.
type updaterState struct {
	LastCheck          time.Time
	AutomaticChecks    bool
	AutomaticDownloads bool
}

func currentUpdaterState() updaterState {
	return updaterState{
		LastCheck:          updater.LastCheck(),
		AutomaticChecks:    updater.AutomaticChecks(),
		AutomaticDownloads: updater.AutomaticDownloads(),
	}
}

// setAutomaticUpdates turns the daily check and the automatic install on or off.
func setAutomaticUpdates(checks, downloads bool) {
	updater.SetAutomaticChecks(checks)
	updater.SetAutomaticDownloads(downloads)
}
