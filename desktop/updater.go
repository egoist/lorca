package main

import (
	"context"
	"errors"
	"sync"
	"time"

	"github.com/egoist/mygo"
)

// UpdateInfo is a newer version of the app: its version and release notes in Markdown.
type UpdateInfo struct {
	Version string `json:"version"`
	Notes   string `json:"notes"`
}

// UpdaterState is what Settings › General shows about updates.
type UpdaterState struct {
	// Enabled is a build that updates itself: a release build, installed where it can write.
	Enabled bool   `json:"enabled"`
	Version string `json:"version"`
	// LastCheck is when the app last looked, in Unix seconds; 0 is never.
	LastCheck          int64 `json:"lastCheck"`
	AutomaticChecks    bool  `json:"automaticChecks"`
	AutomaticDownloads bool  `json:"automaticDownloads"`
	Checking           bool  `json:"checking"`
}

var (
	// UpdateAvailable offers the main window a newer version a background check found.
	UpdateAvailable = mygo.NewEvent[UpdateInfo]("update:available")
	// UpdaterChanged tells Settings that a check finished or a switch moved.
	UpdaterChanged = mygo.NewEvent[UpdaterState]("updater:changed")
)

// updater checks for signed updates (mygo.Updater, from the release the build names) and
// installs them; the new version runs after a relaunch. Its settings are this computer's.
type updater struct {
	mu       sync.Mutex
	found    *mygo.Update
	checking bool
}

var updates = &updater{}

func (u *updater) state() UpdaterState {
	settings := prefs.get()
	u.mu.Lock()
	checking := u.checking
	u.mu.Unlock()
	return UpdaterState{
		Enabled:            mygo.Updater.Enabled(),
		Version:            mygo.App.Version(),
		LastCheck:          settings.LastUpdateCheck,
		AutomaticChecks:    !settings.NoAutomaticUpdateChecks,
		AutomaticDownloads: settings.AutomaticUpdateDownloads,
		Checking:           checking,
	}
}

// check looks for a newer version. It returns nil when the app is up to date.
func (u *updater) check(ctx context.Context) (*mygo.Update, error) {
	if !mygo.Updater.Enabled() {
		return nil, mygo.ErrUpdatesDisabled
	}
	u.mu.Lock()
	if u.checking {
		u.mu.Unlock()
		return nil, errors.New("A check for updates is already running")
	}
	u.checking = true
	u.mu.Unlock()
	UpdaterChanged.Broadcast(u.state())
	found, err := mygo.Updater.Check(ctx)
	now := time.Now().Unix()
	prefs.update(PreferencesPatch{LastUpdateCheck: &now})
	u.mu.Lock()
	u.checking = false
	if err == nil {
		u.found = found
	}
	u.mu.Unlock()
	UpdaterChanged.Broadcast(u.state())
	return found, err
}

// start arms the daily background check of a build that updates itself. A found update is
// installed for the next launch when automatic downloads are on, and offered otherwise.
func (u *updater) start() {
	if !mygo.Updater.Enabled() {
		return
	}
	go func() {
		for {
			settings := prefs.get()
			due := time.Unix(settings.LastUpdateCheck, 0).Add(24 * time.Hour)
			if !settings.NoAutomaticUpdateChecks && time.Now().After(due) {
				u.background()
			}
			time.Sleep(time.Hour)
		}
	}()
}

func (u *updater) background() {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()
	found, err := u.check(ctx)
	if err != nil || found == nil {
		return
	}
	if prefs.get().AutomaticUpdateDownloads {
		_ = found.Install(ctx, nil)
		return
	}
	mygo.RunOnMain(func() {
		if app.main != nil {
			_ = UpdateAvailable.Emit(app.main, UpdateInfo{Version: found.Version, Notes: found.Notes})
		}
	})
}

// UpdaterState returns what Settings shows about updates.
func (Host) UpdaterState() UpdaterState { return updates.state() }

// CheckForUpdates looks for a newer version now; nil means the app is up to date.
func (Host) CheckForUpdates(ctx context.Context) (*UpdateInfo, error) {
	found, err := updates.check(ctx)
	if err != nil || found == nil {
		return nil, err
	}
	return &UpdateInfo{Version: found.Version, Notes: found.Notes}, nil
}

// InstallUpdate downloads the version the last check found, checks its signature, puts it in
// place of this one, and relaunches.
func (Host) InstallUpdate(ctx context.Context) error {
	updates.mu.Lock()
	found := updates.found
	updates.mu.Unlock()
	if found == nil {
		return errors.New("No update to install")
	}
	if err := found.Install(ctx, nil); err != nil {
		return err
	}
	mygo.App.Relaunch()
	return nil
}

// SetAutomaticUpdates turns the daily check and the automatic install on or off.
func (Host) SetAutomaticUpdates(checks, downloads bool) UpdaterState {
	off := !checks
	prefs.update(PreferencesPatch{NoAutomaticUpdateChecks: &off, AutomaticUpdateDownloads: &downloads})
	state := updates.state()
	UpdaterChanged.Broadcast(state)
	return state
}
