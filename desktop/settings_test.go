package main

import (
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// settingsWindowTester is the demo's main window on a settings pane, its updater standing in
// with automatic checks on.
func settingsWindowTester(t *testing.T, height int) (*mainWindow, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	enabled, state := settingsUpdatesEnabled, settingsUpdaterState
	settingsUpdatesEnabled = func() bool { return true }
	settingsUpdaterState = func() updaterState {
		return updaterState{LastCheck: time.Now().Add(-2 * time.Hour), AutomaticChecks: true}
	}
	t.Cleanup(func() { settingsUpdatesEnabled, settingsUpdaterState = enabled, state })
	m.showSettingsDevice("")
	tt := ui.NewTester(m.frame(m.view), 1180, height)
	return m, tt
}

func TestRenderSettingsPanes(t *testing.T) {
	m, tt := settingsWindowTester(t, 900)
	for _, pane := range model.SettingsPanes {
		m.showSettings(pane)
		settle(tt)
		renderTo(t, tt, "settings-"+string(pane)+"-light")
		tt.SetDark(true)
		tt.Frame()
		renderTo(t, tt, "settings-"+string(pane)+"-dark")
		tt.SetDark(false)
		tt.Frame()
	}
	// Another Runner, with an update waiting, and a phone, which runs no bots.
	m.showSettingsDevice("dev-studio")
	m.showSettings(model.PaneDevice)
	settle(tt)
	renderTo(t, tt, "settings-device-studio")
	if !tt.HasText(L("%@ · %@ is available", "0.1.10", "0.1.11")) {
		t.Errorf("no update offered: %q", tt.Texts())
	}
	m.showSettingsDevice("dev-phone")
	m.showSettings(model.PaneBots)
	settle(tt)
	renderTo(t, tt, "settings-bots-phone")
	if !tt.HasText(L("%@ Devices hold your keys and chats but never run a bot. Pick a Runner: a Device running macOS, Linux, or Windows.", "iOS")) {
		t.Errorf("no note for a phone: %q", tt.Texts())
	}
}

func TestSettingsPlugins(t *testing.T) {
	m, tt := settingsWindowTester(t, 900)
	m.showSettings(model.PanePlugins)
	settle(tt)
	file := m.settings.mcp.file
	if file == nil || len(file.Servers) == 0 {
		t.Fatalf("no mcp.json: %+v, failure %q", file, m.settings.mcp.failure)
	}
	server := file.Servers[0]
	// A server's row and Add from Plugins… each open their sheet.
	for _, text := range []string{server.Name, L("Add from Plugins…")} {
		if err := tt.Click(text); err != nil {
			t.Fatal(err)
		}
		settle(tt)
		if !m.hasSheet() {
			t.Errorf("%s opened no sheet", text)
		}
		for m.hasSheet() {
			m.sheets[len(m.sheets)-1].dismiss()
		}
		settle(tt)
	}
	// The switch turns the server off at once, and the Runner confirms it.
	before := server.Enabled
	r, ok := tt.Find(server.Name)
	if !ok {
		t.Fatal("no server row")
	}
	// The switch is 26 wide, 12 in from the row's end.
	tt.ClickAt(r.X+r.W-25, r.Y+r.H/2)
	if got := m.settings.mcp.file.Servers[0]; got.Enabled == before {
		t.Errorf("the switch left %s %v", got.Name, got.Enabled)
	}
	settle(tt)
	// Once the switch has slid.
	time.Sleep(200 * time.Millisecond)
	settle(tt)
	renderTo(t, tt, "settings-plugins-toggled")
	if got := m.settings.mcp.file.Servers[0]; got.Enabled == before {
		t.Errorf("the Runner left %s %v", got.Name, got.Enabled)
	}
}

func TestSettingsAutoReview(t *testing.T) {
	m, tt := settingsWindowTester(t, 760)
	m.showSettings(model.PaneAutoReview)
	settle(tt)
	rules := len(store.AutoReview.Rules)
	if err := tt.Click(L("Add rule")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	renderTo(t, tt, "settings-rule-editor-light")
	tt.SetDark(true)
	tt.Frame()
	renderTo(t, tt, "settings-rule-editor-dark")
	tt.SetDark(false)
	tt.Type("open a pull request")
	tt.Frame()
	// The sheet's Add rule, after its Cancel.
	cancel, ok := tt.Find(L("Cancel"))
	if !ok {
		t.Fatal("no Cancel")
	}
	tt.ClickAt(cancel.X+cancel.W+30, cancel.Y+cancel.H/2)
	settle(tt)
	if m.hasSheet() {
		t.Fatalf("the sheet stayed up: %q", tt.Texts())
	}
	if got := store.AutoReview.Rules; len(got) != rules+1 || got[len(got)-1].Text != "open a pull request" || got[len(got)-1].Behavior != "allow" {
		t.Fatalf("rules %+v", got)
	}
	// The exact rule keeps its words; only its behavior changes.
	if err := tt.Click(L("Edit rule")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	renderTo(t, tt, "settings-rule-editor-exact")
	if !tt.HasText(L("Exact actions cannot be changed. Delete this rule and allow a different action instead.")) {
		t.Errorf("not the exact rule's sheet: %q", tt.Texts())
	}
	tt.Key(0, ui.KeyEscape)
	settle(tt)
	if m.hasSheet() {
		t.Fatal("Escape left the sheet up")
	}
	if err := tt.Click(L("Delete rule")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if got := store.AutoReview.Rules; len(got) != rules || got[0].ID != "ar-2" {
		t.Fatalf("after delete, rules %+v", got)
	}
	renderTo(t, tt, "settings-auto-review-after")
}

func TestSettingsReveal(t *testing.T) {
	m, tt := settingsWindowTester(t, 400)
	m.showSettings(model.PaneGeneral)
	settle(tt)
	m.reveal(automaticDownloadsEntry())
	tt.Frame()
	tt.Frame()
	renderTo(t, tt, "settings-reveal-row")
	if m.settings.flash != automaticDownloadsEntry().row || m.settings.flashSection {
		t.Errorf("flash %q section %v", m.settings.flash, m.settings.flashSection)
	}
	r, ok := tt.Find(automaticDownloadsEntry().row)
	if !ok || r.Y+r.H > 400 {
		t.Errorf("row not in view: %+v %v", r, ok)
	}
	// A section of the name flashes its card.
	m.reveal(deleteAccountEntry())
	tt.Frame()
	tt.Frame()
	renderTo(t, tt, "settings-reveal-section")
	if m.settings.flash != L("Account") || !m.settings.flashSection {
		t.Errorf("flash %q section %v", m.settings.flash, m.settings.flashSection)
	}
	// A server of mcp.json, whose row comes once the Runner answers.
	m.reveal(mcpServerEntry(model.InstalledPlugin{Name: "sentry"}))
	tt.Frame()
	if m.settings.target != "sentry" {
		t.Errorf("target %q before the list came", m.settings.target)
	}
	settle(tt)
	renderTo(t, tt, "settings-reveal-mcp")
	if m.settings.flash != "sentry" || m.settings.flashSection {
		t.Errorf("flash %q section %v", m.settings.flash, m.settings.flashSection)
	}
	if r, ok := tt.Find("sentry"); !ok || r.Y+r.H > 400 {
		t.Errorf("row not in view: %+v %v", r, ok)
	}
}

func TestRenderSettingsWindow(t *testing.T) {
	demoWindow(t)
	s := newSettingsWindow()
	tt := ui.NewTester(s.frame(s.view), 560, 480)
	tt.Frame()
	renderTo(t, tt, "settings-window-general-light")
	tt.SetDark(true)
	tt.Frame()
	renderTo(t, tt, "settings-window-general-dark")
	if err := tt.Click(L("Advanced")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	renderTo(t, tt, "settings-window-advanced-dark")
	tt.SetDark(false)
	tt.Frame()
	renderTo(t, tt, "settings-window-advanced-light")
	if s.tab != model.PaneAdvanced || !tt.HasText(L("Relay URL")) {
		t.Errorf("tab %q, texts %q", s.tab, tt.Texts())
	}
}

func TestSettingsGeneralAndAdvanced(t *testing.T) {
	m, tt := settingsWindowTester(t, 900)
	m.showSettings(model.PaneGeneral)
	settle(tt)
	// A row's switch is 26 wide, 12 in from its end.
	flip := func(label string) {
		t.Helper()
		r, ok := tt.Find(label)
		if !ok {
			t.Fatalf("no %q", label)
		}
		tt.ClickAt(r.X+r.W-25, r.Y+r.H/2)
		settle(tt)
	}
	before := prefs.get().SendOnReturn
	flip(sendOnReturnEntry().row)
	if prefs.get().SendOnReturn == before {
		t.Errorf("send on Return stayed %v", before)
	}
	// Downloads wait on checks: with checks off, its switch does nothing (the updater, which a
	// test has not, would panic).
	settingsUpdaterState = func() updaterState { return updaterState{} }
	tt.Frame()
	flip(automaticDownloadsEntry().row)

	m.showSettings(model.PaneAdvanced)
	settle(tt)
	port := func(text string) {
		t.Helper()
		r, ok := tt.Find(cliPortEntry().row)
		if !ok {
			t.Fatal("no CLI port")
		}
		tt.ClickAt(r.X+r.W/2, r.Y+r.H/2)
		tt.Key(ui.Cmd, ui.KeyA)
		tt.Type(text)
		tt.Key(0, ui.KeyEnter)
		settle(tt)
	}
	port("5000")
	if got := prefs.get().CLIPort; got != 5000 {
		t.Errorf("port %d", got)
	}
	port("70000")
	if got := prefs.get().CLIPort; got != 5000 {
		t.Errorf("an out-of-range port took: %d", got)
	}
}

func TestRenderSettingsChinese(t *testing.T) {
	m, tt := settingsWindowTester(t, 760)
	l10n.Set("zh-Hans", "zh-CN")
	t.Cleanup(func() { l10n.Set("en", "en-US") })
	for _, pane := range []model.SettingsPane{model.PaneGeneral, model.PaneAutoReview, model.PaneDevice} {
		m.showSettings(pane)
		settle(tt)
		renderTo(t, tt, "settings-zh-"+string(pane))
	}
}
