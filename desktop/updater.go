package main

import (
	"context"
	"log"
	"sync"
	"time"

	"github.com/egoist/lorca/desktop/model"
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
	// A check that went through, the update window's checkbox, or a switch: General shows it,
	// and the other Devices see whether updates install by themselves.
	updater.OnChange(func() {
		invalidateWindows()
		go remoteUpdates.report()
	})
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

// MARK: - Updates another Device asks for

// remoteUpdates tells this app's CLI how the app's updates stand, which the CLI puts in this
// Device's `machine` blob, so every paired Device shows the app's version and offers Update. A
// Device's Update reaches the CLI as a sealed request, and the CLI hands it here
// (`app.update.install`): the app installs the version its signed feed offers, as the update
// window does, waits for the CLI to say no bot is at work on this Runner, and relaunches into
// it, which starts the CLI the new version carries.
var remoteUpdates = &appUpdates{
	enabled:  updatesEnabled,
	version:  mygo.App.Version,
	auto:     func() bool { return updater.AutomaticChecks() && updater.AutomaticDownloads() },
	check:    checkAppRelease,
	relaunch: mygo.App.Relaunch,
	now:      time.Now,
}

// appRelease is a newer version the feed offers.
type appRelease struct {
	version string
	install func(ctx context.Context) error
}

func checkAppRelease(ctx context.Context) (*appRelease, error) {
	up, err := mygo.Updater.Check(ctx)
	if err != nil || up == nil {
		return nil, err
	}
	return &appRelease{version: up.Version, install: func(ctx context.Context) error { return up.Install(ctx, nil) }}, nil
}

// appCheckInterval is how often the app looks for a newer version for the other Devices to see.
const appCheckInterval = 24 * time.Hour

type appUpdates struct {
	enabled  func() bool
	version  func() string
	auto     func() bool
	check    func(context.Context) (*appRelease, error)
	relaunch func()
	now      func() time.Time

	mu sync.Mutex
	// latest is the newest version the last check found, when it is newer than this one.
	latest string
	// checked is when a check last went through.
	checked time.Time
	// state is "installing", "restarting" while the installed version waits for the bots, or "".
	state string
	// err is why the last check or install failed, until one goes through.
	err string
	// busy is set while a check or an install runs.
	busy bool
	// wanted is an install a Device asked for while a background check ran, which follows it.
	// One asked for while an install checks is answered by that install.
	wanted bool
	// waiting is set while a request waits for the CLI to say the Runner is idle.
	waiting bool
	timer   *time.Timer
}

// connected runs each time the app's CLI answers: it says how the app's updates stand, after a
// check when the last one is a day old, and picks up the relaunch a restarted CLI interrupted.
func (u *appUpdates) connected() {
	if !u.enabled() || store.IsMock {
		return
	}
	u.mu.Lock()
	due := u.now().Sub(u.checked) >= appCheckInterval && !u.busy
	if due {
		u.busy = true
	}
	restarting := u.state == "restarting"
	u.mu.Unlock()
	if due {
		u.refresh()
	} else {
		u.report()
	}
	if restarting {
		u.relaunchWhenIdle()
	}
}

// refresh checks the feed for a newer version, says what it found, and checks again a day later.
// The caller set busy.
func (u *appUpdates) refresh() {
	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	release, err := u.check(ctx)
	cancel()
	u.mu.Lock()
	u.busy = false
	u.found(release, err)
	if u.wanted {
		u.wanted = false
		go u.install()
	}
	if u.timer != nil {
		u.timer.Stop()
	}
	u.timer = time.AfterFunc(appCheckInterval, func() {
		u.mu.Lock()
		due := !u.busy && u.state == ""
		if due {
			u.busy = true
		}
		u.mu.Unlock()
		if due {
			u.refresh()
		}
	})
	u.mu.Unlock()
	u.report()
}

// found records what a check answered. u.mu is held.
func (u *appUpdates) found(release *appRelease, err error) {
	if err != nil {
		u.err = err.Error()
		return
	}
	u.checked, u.err, u.latest = u.now(), "", ""
	if release != nil {
		u.latest = release.version
	}
}

// install is the `app.update.install` a Device asked for: checks the feed, installs a newer
// version, and relaunches into it once no bot is at work on this Runner.
func (u *appUpdates) install() {
	if !u.enabled() {
		return
	}
	u.mu.Lock()
	if u.busy || u.state != "" {
		// A background check runs: the install follows it. One under way goes on.
		u.wanted = u.busy && u.state == ""
		u.mu.Unlock()
		return
	}
	u.busy = true
	u.mu.Unlock()

	ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
	release, err := u.check(ctx)
	cancel()
	u.mu.Lock()
	u.found(release, err)
	if release == nil {
		u.busy, u.wanted = false, false
		u.mu.Unlock()
		u.report()
		return
	}
	u.state = "installing"
	u.mu.Unlock()
	u.report()

	ctx, cancel = context.WithTimeout(context.Background(), 30*time.Minute)
	err = release.install(ctx)
	cancel()
	u.mu.Lock()
	u.busy, u.wanted = false, false
	if err != nil {
		u.state, u.err = "", err.Error()
		u.mu.Unlock()
		u.report()
		return
	}
	u.state = "restarting"
	u.mu.Unlock()
	u.report()
	u.relaunchWhenIdle()
}

// relaunchWhenIdle asks the CLI to answer once no bot is at work on this Runner, then relaunches
// into the installed version. A CLI that restarts meanwhile drops the request; `connected` asks
// the next one.
func (u *appUpdates) relaunchWhenIdle() {
	u.mu.Lock()
	if u.waiting {
		u.mu.Unlock()
		return
	}
	u.waiting = true
	u.mu.Unlock()
	err := store.RelaunchWhenIdle()
	u.mu.Lock()
	u.waiting = false
	u.mu.Unlock()
	if err == nil {
		u.relaunch()
	}
}

// report tells the CLI how the app's updates stand.
func (u *appUpdates) report() {
	if !u.enabled() || store.IsMock {
		return
	}
	u.mu.Lock()
	report := model.AppUpdateReport{
		Version: u.version(),
		Auto:    u.auto(),
		State:   u.state,
	}
	if u.latest != "" {
		latest := u.latest
		report.Latest = &latest
	}
	if u.err != "" {
		err := u.err
		report.Error = &err
	}
	u.mu.Unlock()
	if err := store.ReportAppUpdate(report); err != nil {
		log.Printf("telling the CLI about the app's updates: %s", model.ErrorText(err))
	}
}
