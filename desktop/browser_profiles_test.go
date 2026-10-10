package main

import (
	"encoding/json"
	"slices"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// withBrowser installs Browser on the demo Runner and gives the bot profiles named names.
func withBrowser(t *testing.T, runnerID, botID string, names ...string) []model.BrowserProfile {
	t.Helper()
	runner := store.Device(runnerID)
	runner.Plugins = append(runner.Plugins, model.InstalledPlugin{ID: model.BrowserPluginID, Name: "Browser", Description: "Opens web pages in a browser on the Runner to read them, fill forms, and take screenshots.", State: model.PluginReady, Detail: "Ready"})
	for _, name := range names {
		store.BrowserProfileAction("browser.create", botID, map[string]any{"name": name}, nil)
	}
	var profiles []model.BrowserProfile
	store.BrowserProfiles(botID, func(list []model.BrowserProfile, _ error) { profiles = list })
	runPosts()
	return profiles
}

// renderBrowser writes the frame at 2x, light and dark, for looking at the sheet.
func renderBrowser(t *testing.T, tt *ui.Tester, name string) {
	t.Helper()
	tt.SetScale(2)
	sheetARender(t, tt, name)
	tt.SetScale(1)
	tt.Frame()
}

func browserAction(method, botID, sessionID string) {
	store.BrowserProfileAction(method, botID, map[string]any{"session_id": sessionID}, nil)
	runPosts()
}

// profileMenu opens the menu of the profile row showing name.
func profileMenu(t *testing.T, tt *ui.Tester, name string) []string {
	t.Helper()
	if err := tt.Click(name); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	return tt.Menu()
}

func TestBrowserProfilesInThePluginSheet(t *testing.T) {
	var profiles []model.BrowserProfile
	m, tt := sheetATester(t, func(m *mainWindow) {
		profiles = withBrowser(t, "dev-workbench", "bot-quill", "Work", "Personal")
		browserAction("browser.open", "bot-quill", profiles[0].ID)
		m.presentPlugin(model.BrowserPluginID, sheetAWorkbench(), "bot-quill", "chat-launch")
	})
	for _, text := range []string{"PROFILES", "Work", L("You have control"), "Personal", Lc("Closed", "browser")} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	if slices.Contains(tt.Texts(), "SIGN-IN") || slices.Contains(tt.Texts(), "GITHUB_TOKEN") {
		t.Errorf("Browser shows another plugin's setup: %q", tt.Texts())
	}
	renderBrowser(t, tt, "browser-profiles")

	if menu := profileMenu(t, tt, "Work"); !slices.Equal(menu, []string{L("Return to Bot"), Lc("Record", "browser"), L("Take Screenshot"), L("Close Browser"), "-", L("Delete…")}) {
		t.Fatalf("menu %q", menu)
	}
	if err := tt.ChooseMenuItem(L("Return to Bot")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if !tt.HasText(Lc("Open", "browser")) {
		t.Fatalf("after Return to Bot: %q", tt.Texts())
	}
	if menu := profileMenu(t, tt, "Work"); !slices.Equal(menu, []string{L("Take Over"), Lc("Record", "browser"), L("Take Screenshot"), L("Close Browser"), "-", L("Delete…")}) {
		t.Fatalf("menu %q", menu)
	}
	if err := tt.ChooseMenuItem(L("Close Browser")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if menu := profileMenu(t, tt, "Personal"); !slices.Equal(menu, []string{L("Open Browser"), Lc("Record", "browser"), "-", L("Delete…")}) {
		t.Fatalf("menu %q", menu)
	}
	tt.CloseMenu()

	// Delete asks first, then the row goes.
	profileMenu(t, tt, "Personal")
	if err := tt.ChooseMenuItem(L("Delete…")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText(L("Delete the “%@” profile?", "Personal")) {
		t.Fatalf("no question: %q", tt.Texts())
	}
	if err := tt.Click(L("Delete")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if tt.HasText("Personal") {
		t.Errorf("deleted profile still listed: %q", tt.Texts())
	}

	// + names a new profile.
	if err := tt.Click(L("Add Profile")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	tt.Type("Shopping")
	if err := tt.Click(L("Add")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if !tt.HasText("Shopping") {
		t.Errorf("no new profile in %q", tt.Texts())
	}
	if err := tt.Click(L("Done")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if m.hasSheet() {
		t.Error("sheet still up")
	}
}

func TestBrowserProfilesFromAnotherDevice(t *testing.T) {
	_, tt := sheetATester(t, func(m *mainWindow) {
		profiles := withBrowser(t, "dev-studio", "bot-scout", "Research", "Personal")
		browserAction("browser.open", "bot-scout", profiles[0].ID)
		browserAction("browser.resume", "bot-scout", profiles[0].ID)
		m.presentPlugin(model.BrowserPluginID, store.Device("dev-studio"), "bot-scout", "chat-scout")
	})
	renderBrowser(t, tt, "browser-profiles-paired")
	// A window opens only on the Runner's own screen.
	if menu := profileMenu(t, tt, "Personal"); !slices.Equal(menu, []string{L("Open on %@", "Studio"), L("Record on %@", "Studio"), "-", L("Delete…")}) {
		t.Fatalf("menu %q", menu)
	}
	if err := tt.ChooseMenuItem(L("Open on %@", "Studio")); err == nil {
		t.Fatal("opened a window for another Device")
	}
	if err := tt.ChooseMenuItem(L("Record on %@", "Studio")); err == nil {
		t.Fatal("recorded in a browser that isn't on the Runner's screen")
	}
	tt.CloseMenu()
	// One open on the Runner's screen records from here: the user does the task there.
	if menu := profileMenu(t, tt, "Research"); !slices.Contains(menu, L("Take Over")) || !slices.Contains(menu, Lc("Record", "browser")) {
		t.Fatalf("menu %q", menu)
	}
}

// Record takes the browser for the user and the row turns red; Stop Recording sends it to the
// chat the sheet covers and closes the sheet.
func TestBrowserProfilesRecord(t *testing.T) {
	m, tt := sheetATester(t, func(m *mainWindow) {
		withBrowser(t, "dev-workbench", "bot-quill", "Work")
		m.presentPlugin(model.BrowserPluginID, sheetAWorkbench(), "bot-quill", "chat-launch")
	})
	profileMenu(t, tt, "Work")
	if err := tt.ChooseMenuItem(Lc("Record", "browser")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if !tt.HasText(L("Recording")) {
		t.Fatalf("no recording state in %q", tt.Texts())
	}
	renderBrowser(t, tt, "browser-profiles-recording")
	if menu := profileMenu(t, tt, "Work"); !slices.Equal(menu, []string{L("Stop Recording"), L("Take Screenshot"), L("Close Browser"), "-", L("Delete…")}) {
		t.Fatalf("menu %q", menu)
	}
	if err := tt.ChooseMenuItem(L("Stop Recording")); err != nil {
		t.Fatal(err)
	}
	sheetASettle(tt)
	if m.hasSheet() {
		t.Error("the sheet stays up over the chat the recording went to")
	}
}

func TestBrowserProfilesEmptyAndOtherSheets(t *testing.T) {
	_, tt := sheetATester(t, func(m *mainWindow) {
		withBrowser(t, "dev-workbench", "bot-nova")
		m.presentPlugin(model.BrowserPluginID, sheetAWorkbench(), "bot-nova", "chat-nova")
	})
	if !tt.HasText(L("A profile keeps sign-ins for %@'s browser. Add one, then open it on %@ to sign in.", "Project Manager", "Workbench")) {
		t.Errorf("no empty state in %q", tt.Texts())
	}
	renderBrowser(t, tt, "browser-profiles-empty")
	// Opened from Settings, the sheet is the Runner's, for no bot.
	_, tt = sheetATester(t, func(m *mainWindow) { m.presentPlugin(model.BrowserPluginID, sheetAWorkbench(), "", "") })
	if tt.HasText("PROFILES") {
		t.Errorf("profiles without a bot: %q", tt.Texts())
	}
}

type browserCall struct {
	method string
	params map[string]any
	reply  chan any
}

type browserTransport struct{ calls chan browserCall }

func (browserTransport) Reconnect() {}
func (b browserTransport) Request(method string, params any) (json.RawMessage, error) {
	var values map[string]any
	bytes, _ := json.Marshal(params)
	_ = json.Unmarshal(bytes, &values)
	call := browserCall{method, values, make(chan any, 1)}
	b.calls <- call
	return json.Marshal(<-call.reply)
}

func nextBrowserCall(t *testing.T, calls <-chan browserCall) browserCall {
	t.Helper()
	select {
	case call := <-calls:
		return call
	case <-time.After(time.Second):
		t.Fatal("no browser request")
		return browserCall{}
	}
}

// Against the CLI: Return to Bot names the revision this app saw, a screenshot the sheet's chat,
// and an answer that comes after the sheet went down changes nothing.
func TestBrowserProfilesWire(t *testing.T) {
	m := demoWindow(t)
	calls, posted := make(chan browserCall, 10), make(chan func(), 10)
	demo := store
	t.Cleanup(func() { store = demo })
	store = model.NewStore(browserTransport{calls}, func(fn func()) { posted <- fn }, false)
	store.Bots, store.Devices, store.IsConnected = demo.Bots, demo.Devices, true
	bot, runner := store.Bot("bot-quill"), store.Device("dev-workbench")
	b := newBrowserProfiles(&m.appWindow, bot, runner, "chat-launch")
	t.Cleanup(b.close)
	list := nextBrowserCall(t, calls)
	if list.method != "browser.sessions" || list.params["bot_id"] != "bot-quill" {
		t.Fatal(list.params)
	}
	list.reply <- map[string]any{"sessions": []model.BrowserProfile{{ID: "browser-1", Name: "Work", State: model.BrowserHuman, Revision: 7}}}
	waitFor(t, posted, func() bool { return len(b.profiles) == 1 })

	b.perform("browser.resume", b.profiles[0], map[string]any{"revision": b.profiles[0].Revision}, L("Returning…"), "")
	resume := nextBrowserCall(t, calls)
	if resume.method != "browser.resume" || resume.params["revision"] != float64(7) || resume.params["session_id"] != "browser-1" || resume.params["bot_id"] != "bot-quill" {
		t.Fatal(resume.params)
	}
	if b.pending["browser-1"] != L("Returning…") {
		t.Fatal("no pending state while the Runner answers")
	}
	resume.reply <- map[string]any{"session": map[string]any{}}
	(<-posted)()
	refresh := nextBrowserCall(t, calls)
	refresh.reply <- map[string]any{"sessions": []model.BrowserProfile{{ID: "browser-1", Name: "Work", State: model.BrowserBot, Revision: 8}}}
	waitFor(t, posted, func() bool { return b.profiles[0].State == model.BrowserBot && len(b.pending) == 0 })

	b.perform("browser.screenshot", b.profiles[0], map[string]any{"chat_id": b.chatID}, "", "")
	shot := nextBrowserCall(t, calls)
	if shot.method != "browser.screenshot" || shot.params["chat_id"] != "chat-launch" {
		t.Fatal(shot.params)
	}
	shot.reply <- map[string]any{"message_id": "message-1"}
	(<-posted)()
	shotRefresh := nextBrowserCall(t, calls)
	shotRefresh.reply <- map[string]any{"sessions": []model.BrowserProfile{{ID: "browser-1", Name: "Work", State: model.BrowserHuman, Revision: 9, Recording: true}}}
	waitFor(t, posted, func() bool { return b.profiles[0].Recording })

	// Stopping sends the chat and the user's words; a recording of nothing says so.
	b.stopRecording(b.profiles[0])
	stop := nextBrowserCall(t, calls)
	if stop.method != "browser.stop_recording" || stop.params["chat_id"] != "chat-launch" || stop.params["session_id"] != "browser-1" || stop.params["text"] == "" {
		t.Fatal(stop.params)
	}
	stop.reply <- map[string]any{"session": map[string]any{}, "message_id": nil}
	(<-posted)()
	if !m.hasSheet() {
		t.Fatal("no word that nothing was recorded")
	}
	late := nextBrowserCall(t, calls)
	b.close()
	late.reply <- map[string]any{"sessions": []model.BrowserProfile{}}
	(<-posted)()
	if len(b.profiles) != 1 {
		t.Fatal("a closed sheet took a late answer")
	}
}

// waitFor runs what the store posts until done says so.
func waitFor(t *testing.T, posted <-chan func(), done func() bool) {
	t.Helper()
	for !done() {
		select {
		case fn := <-posted:
			fn()
		case <-time.After(time.Second):
			t.Fatal("timed out")
		}
	}
}
