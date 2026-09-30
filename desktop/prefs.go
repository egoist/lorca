package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"sync"

	"github.com/egoist/mygo"
)

// Preferences are this computer's own settings: what the app remembers between launches, and
// the CLI port and relay URL a computer with no CLI answering needs. Nothing here is synced.
type Preferences struct {
	// HadIdentity is whether the CLI's last answer had an identity. Launch opens the main window
	// at once when it did, and otherwise waits for the answer, so a fresh install goes straight
	// to onboarding.
	HadIdentity bool `json:"hadIdentity"`
	// Selection is what the main window showed ("chat:<id>", "settings:<pane>"), so a relaunch
	// lands back on it.
	Selection      string `json:"selection"`
	ShowsInspector bool   `json:"showsInspector"`
	SendOnReturn   bool   `json:"sendOnReturn"`
	ShowTimestamps bool   `json:"showTimestamps"`
	RelayURL       string `json:"relayURL"`
	// CLIPort is the port the app looks for the CLI on. `LORCA_PORT` wins, so a second app
	// instance can run against its own CLI.
	CLIPort int `json:"cliPort"`
	// AppLanguage is the language the app's own words are in ("en", "zh-Hans"); empty follows
	// the system.
	AppLanguage string `json:"appLanguage"`
	// Appearance is "light" or "dark"; empty follows the system.
	Appearance string `json:"appearance"`
	// The split view's panes, as the user left them.
	SidebarWidth     int  `json:"sidebarWidth"`
	InspectorWidth   int  `json:"inspectorWidth"`
	SidebarCollapsed bool `json:"sidebarCollapsed"`
	// The updater's: when it last looked, and its two switches.
	LastUpdateCheck          int64 `json:"lastUpdateCheck"`
	NoAutomaticUpdateChecks  bool  `json:"noAutomaticUpdateChecks"`
	AutomaticUpdateDownloads bool  `json:"automaticUpdateDownloads"`
}

// PreferencesPatch changes the preferences it names.
type PreferencesPatch struct {
	HadIdentity      *bool   `json:"hadIdentity,omitempty"`
	Selection        *string `json:"selection,omitempty"`
	ShowsInspector   *bool   `json:"showsInspector,omitempty"`
	SendOnReturn     *bool   `json:"sendOnReturn,omitempty"`
	ShowTimestamps   *bool   `json:"showTimestamps,omitempty"`
	RelayURL         *string `json:"relayURL,omitempty"`
	CLIPort          *int    `json:"cliPort,omitempty"`
	AppLanguage      *string `json:"appLanguage,omitempty"`
	Appearance       *string `json:"appearance,omitempty"`
	SidebarWidth     *int    `json:"sidebarWidth,omitempty"`
	InspectorWidth   *int    `json:"inspectorWidth,omitempty"`
	SidebarCollapsed *bool   `json:"sidebarCollapsed,omitempty"`

	LastUpdateCheck          *int64 `json:"lastUpdateCheck,omitempty"`
	NoAutomaticUpdateChecks  *bool  `json:"noAutomaticUpdateChecks,omitempty"`
	AutomaticUpdateDownloads *bool  `json:"automaticUpdateDownloads,omitempty"`
}

// PreferencesChanged tells every window what the preferences are now, after any window changed
// them: a new language or appearance shows everywhere at once.
var PreferencesChanged = mygo.NewEvent[Preferences]("prefs:changed")

type prefsStore struct {
	mu    sync.Mutex
	path  string
	value Preferences
}

var prefs = &prefsStore{}

func (s *prefsStore) load() {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.value = Preferences{ShowsInspector: true, SendOnReturn: true, ShowTimestamps: true}
	dir, err := mygo.App.Path(mygo.PathUserData)
	if err != nil {
		return
	}
	s.path = filepath.Join(dir, "preferences.json")
	if data, err := os.ReadFile(s.path); err == nil {
		_ = json.Unmarshal(data, &s.value)
	}
}

func (s *prefsStore) get() Preferences {
	s.mu.Lock()
	defer s.mu.Unlock()
	value := s.value
	value.CLIPort = cliPortLocked(value)
	return value
}

// cliPort is the port in force: `LORCA_PORT`, else the saved one, else this build's default.
func (s *prefsStore) cliPort() int {
	return s.get().CLIPort
}

func cliPortLocked(value Preferences) int {
	if raw := os.Getenv("LORCA_PORT"); raw != "" {
		if port, err := strconv.Atoi(raw); err == nil && port > 0 {
			return port
		}
	}
	if value.CLIPort > 0 {
		return value.CLIPort
	}
	return defaultCLIPort()
}

func (s *prefsStore) update(patch PreferencesPatch) Preferences {
	s.mu.Lock()
	v := &s.value
	if patch.HadIdentity != nil {
		v.HadIdentity = *patch.HadIdentity
	}
	if patch.Selection != nil {
		v.Selection = *patch.Selection
	}
	if patch.ShowsInspector != nil {
		v.ShowsInspector = *patch.ShowsInspector
	}
	if patch.SendOnReturn != nil {
		v.SendOnReturn = *patch.SendOnReturn
	}
	if patch.ShowTimestamps != nil {
		v.ShowTimestamps = *patch.ShowTimestamps
	}
	if patch.RelayURL != nil {
		v.RelayURL = *patch.RelayURL
	}
	if patch.CLIPort != nil {
		v.CLIPort = *patch.CLIPort
	}
	if patch.AppLanguage != nil {
		v.AppLanguage = *patch.AppLanguage
	}
	if patch.Appearance != nil {
		v.Appearance = *patch.Appearance
	}
	if patch.SidebarWidth != nil {
		v.SidebarWidth = *patch.SidebarWidth
	}
	if patch.InspectorWidth != nil {
		v.InspectorWidth = *patch.InspectorWidth
	}
	if patch.SidebarCollapsed != nil {
		v.SidebarCollapsed = *patch.SidebarCollapsed
	}
	if patch.LastUpdateCheck != nil {
		v.LastUpdateCheck = *patch.LastUpdateCheck
	}
	if patch.NoAutomaticUpdateChecks != nil {
		v.NoAutomaticUpdateChecks = *patch.NoAutomaticUpdateChecks
	}
	if patch.AutomaticUpdateDownloads != nil {
		v.AutomaticUpdateDownloads = *patch.AutomaticUpdateDownloads
	}
	saved := *v
	path := s.path
	s.mu.Unlock()
	if path != "" {
		if data, err := json.MarshalIndent(saved, "", "  "); err == nil {
			tmp := path + ".tmp"
			if os.WriteFile(tmp, data, 0o600) == nil {
				_ = os.Rename(tmp, path)
			}
		}
	}
	return s.get()
}

// Prefs is this computer's settings, for the pages.
type Prefs struct{}

// All returns the preferences as they are now.
func (Prefs) All() Preferences { return prefs.get() }

// Set changes the preferences the patch names and tells every window. A new CLI port makes the
// app look for the CLI there; a new appearance applies to every window.
func (Prefs) Set(patch PreferencesPatch) Preferences {
	before := prefs.get()
	after := prefs.update(patch)
	if after.Appearance != before.Appearance {
		applyAppearance(after.Appearance)
	}
	PreferencesChanged.Broadcast(after)
	if after.CLIPort != before.CLIPort {
		go app.cli.reconnect()
	}
	return after
}

// applyAppearance forces light or dark for the windows and pages, or follows the system.
func applyAppearance(appearance string) {
	switch appearance {
	case "light":
		mygo.Theme.SetSource(mygo.ThemeLight)
	case "dark":
		mygo.Theme.SetSource(mygo.ThemeDark)
	default:
		mygo.Theme.SetSource(mygo.ThemeSystem)
	}
}
