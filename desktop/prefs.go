package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"sync"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/mygo"
)

// Preferences are this computer's own settings: what the app remembers between launches, and
// the CLI port and relay URL a computer with no CLI answering needs. Nothing here is synced.
type Preferences struct {
	// HadIdentity is whether the CLI's last answer had an identity. Launch opens the main window
	// at once when it did, and otherwise waits for the answer, so a fresh install goes straight
	// to onboarding.
	HadIdentity bool `json:"hadIdentity"`
	// Selection is the chat the main window showed last ("chat:<id>"), so a relaunch lands back
	// on it rather than on a settings pane.
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
	// ShowsHiddenChats is the sidebar's Hidden group open; it starts folded. CollapsesOtherChats
	// folds the group of chats in no section. Sections fold on every Device through the roster;
	// these two fold on this computer alone.
	ShowsHiddenChats    bool `json:"showsHiddenChats"`
	CollapsesOtherChats bool `json:"collapsesOtherChats"`
}

// PreferencesPatch changes the preferences it names.
type PreferencesPatch struct {
	HadIdentity         *bool   `json:"hadIdentity,omitempty"`
	Selection           *string `json:"selection,omitempty"`
	ShowsInspector      *bool   `json:"showsInspector,omitempty"`
	SendOnReturn        *bool   `json:"sendOnReturn,omitempty"`
	ShowTimestamps      *bool   `json:"showTimestamps,omitempty"`
	RelayURL            *string `json:"relayURL,omitempty"`
	CLIPort             *int    `json:"cliPort,omitempty"`
	AppLanguage         *string `json:"appLanguage,omitempty"`
	Appearance          *string `json:"appearance,omitempty"`
	SidebarWidth        *int    `json:"sidebarWidth,omitempty"`
	InspectorWidth      *int    `json:"inspectorWidth,omitempty"`
	SidebarCollapsed    *bool   `json:"sidebarCollapsed,omitempty"`
	ShowsHiddenChats    *bool   `json:"showsHiddenChats,omitempty"`
	CollapsesOtherChats *bool   `json:"collapsesOtherChats,omitempty"`
}

type prefsStore struct {
	mu    sync.Mutex
	path  string
	value Preferences
}

var prefs = &prefsStore{}

// load reads preferences.json in the app's data directory.
func (s *prefsStore) load() {
	path := ""
	if dir, err := mygo.App.Path(mygo.PathUserData); err == nil {
		path = filepath.Join(dir, "preferences.json")
	}
	s.loadFrom(path)
}

// loadFrom reads the preferences at `path`, which later changes are saved to; none is the defaults.
func (s *prefsStore) loadFrom(path string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.value = Preferences{ShowsInspector: true, SendOnReturn: true, ShowTimestamps: true}
	s.path = path
	if data, err := os.ReadFile(path); path != "" && err == nil {
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
	if patch.ShowsHiddenChats != nil {
		v.ShowsHiddenChats = *patch.ShowsHiddenChats
	}
	if patch.CollapsesOtherChats != nil {
		v.CollapsesOtherChats = *patch.CollapsesOtherChats
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

// setPrefs changes the preferences the patch names and applies them: a new appearance or
// language to every window, a new CLI port by looking for the CLI there.
func setPrefs(patch PreferencesPatch) Preferences {
	before := prefs.get()
	after := prefs.update(patch)
	if after.Appearance != before.Appearance {
		applyAppearance(after.Appearance)
	}
	if after.AppLanguage != before.AppLanguage {
		if l10n.Set(after.AppLanguage, mygo.App.Locale()) {
			app.languageChanged()
		}
	}
	if after.CLIPort != before.CLIPort && app.cli != nil {
		go app.cli.reconnect()
	}
	invalidateWindows()
	return after
}

// applyAppearance forces light or dark for the windows, or follows the system.
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
