package main

import (
	"encoding/json"
	"slices"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func browserFixture(s *browserSheet, state string, local bool) model.BrowserSessionList {
	return model.BrowserSessionList{
		Sessions:     []model.BrowserSession{{ID: "browser-demo-1", BotID: s.botID, RunnerID: s.runnerID, Account: "Demo work account", Profile: "Research", State: state, Selected: true, Revision: 19}},
		Capabilities: model.BrowserCapabilities{VisibleOpen: local, LocalInput: local, Pause: true, Resume: true, Stop: true, Screenshot: true},
	}
}

func TestBrowserSheetActionsAndPersistentDrafts(t *testing.T) {
	var s *browserSheet
	m, tt := sheetTester(t, func(m *mainWindow) { s = m.presentBrowserSessions("bot-nova", "chat-nova") })
	t.Cleanup(s.close)
	s.account, s.profile = "", ""
	tt.Frame()
	for _, field := range []struct{ label, value string }{{"Account", "Work"}, {"Profile", "Sign-in"}} {
		r, found := tt.Find(field.label)
		if !found {
			t.Fatal(field.label)
		}
		tt.ClickAt(r.X+110, r.Y+r.H/2)
		tt.Type(field.value)
		tt.Frame()
	}
	if s.account != "Work" || s.profile != "Sign-in" {
		t.Fatalf("drafts %q/%q", s.account, s.profile)
	}
	s.load()
	settle(tt)
	if s.account != "Work" || s.profile != "Sign-in" {
		t.Fatal("a refresh overwrote input")
	}
	if err := tt.Click(L("Create Profile")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	created, ok := s.selected()
	if !ok || created.Account != "Work" || created.Profile != "Sign-in" || created.State != model.BrowserStopped {
		t.Fatalf("profile %+v", created)
	}
	for _, step := range []struct{ title, state string }{
		{"Open Browser", model.BrowserHuman}, {"Return to Bot", model.BrowserBotControl}, {"Take Over", model.BrowserHuman}, {"Stop Browser", model.BrowserStopped},
	} {
		if err := tt.Click(L(step.title)); err != nil {
			t.Fatal(err)
		}
		settle(tt)
		current, _ := s.selected()
		if current.State != step.state || current.ID != created.ID {
			t.Fatalf("%s -> %+v", step.title, current)
		}
	}
	if err := tt.Click(L("Done")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !s.closed || m.hasSheet() {
		t.Fatal("dismissal left the refresh lifetime open")
	}
}

func TestRenderBrowserControlStates(t *testing.T) {
	for _, scenario := range []struct {
		name, state   string
		local         bool
		botID, chatID string
	}{
		{"browser-bot-control", model.BrowserBotControl, true, "bot-nova", "chat-nova"},
		{"browser-human-control", model.BrowserHuman, true, "bot-nova", "chat-nova"},
		{"browser-takeover-waiting", model.BrowserTakingOver, true, "bot-nova", "chat-nova"},
		{"browser-paired-device", model.BrowserBotControl, false, "bot-patch", "chat-patch"},
	} {
		t.Run(scenario.name, func(t *testing.T) {
			var s *browserSheet
			_, tt := sheetTester(t, func(m *mainWindow) { s = m.presentBrowserSessions(scenario.botID, scenario.chatID) })
			t.Cleanup(s.close)
			s.apply(browserFixture(s, scenario.state, scenario.local))
			settle(tt)
			if s.allowed(model.BrowserOpen) != (scenario.local && scenario.state != model.BrowserTakingOver) {
				t.Fatal("visible open capability inferred incorrectly")
			}
			if !s.allowed(model.BrowserStop) {
				t.Fatal("Stop unavailable during takeover")
			}
			if scenario.state == model.BrowserTakingOver && (s.allowed(model.BrowserTakeover) || s.allowed(model.BrowserResume) || s.allowed(model.BrowserScreenshot)) {
				t.Fatal("takeover is competing for input")
			}
			if scenario.state == model.BrowserHuman && !slices.Contains(tt.Texts(), L("Return to Bot")) {
				t.Fatal(tt.Texts())
			}
			if !scenario.local && (!tt.HasText(L("Pause on Runner")) || !tt.HasText(L("This Device can pause, return control, stop, and attach screenshots. Sign-in and interactive input happen on %@; live remote viewing and input are unavailable.", s.runnerName))) {
				t.Fatal(tt.Texts())
			}
			renderBoth(t, tt, scenario.name)
		})
	}
}

type desktopBrowserCall struct {
	method string
	params map[string]any
	reply  chan any
}
type desktopBrowserTransport struct{ calls chan desktopBrowserCall }

func (desktopBrowserTransport) Reconnect() {}
func (b desktopBrowserTransport) Request(method string, params any) (json.RawMessage, error) {
	var values map[string]any
	bytes, _ := json.Marshal(params)
	_ = json.Unmarshal(bytes, &values)
	call := desktopBrowserCall{method, values, make(chan any, 1)}
	b.calls <- call
	return json.Marshal(<-call.reply)
}

func browserWireSheet(t *testing.T) (*browserSheet, *ui.Tester, <-chan desktopBrowserCall, <-chan func()) {
	t.Helper()
	m := demoWindow(t)
	m.userWantsInspector = false
	calls, posted := make(chan desktopBrowserCall, 10), make(chan func(), 10)
	source := model.NewStore(desktopBrowserTransport{calls}, func(fn func()) { posted <- fn }, false)
	copyBot := *store.Bot("bot-nova")
	source.Bots, source.Devices, source.IsConnected = []*model.Bot{&copyBot}, store.Devices, true
	s := &browserSheet{source: source, botID: copyBot.ID, runnerID: copyBot.RunnerID, chatID: "chat-nova", botName: "Demo Bot", runnerName: "Demo Runner", account: "Default", profile: "Browser"}
	s.apply(browserFixture(s, model.BrowserBotControl, true))
	m.present(s.view, s.close)
	t.Cleanup(s.close)
	tt := ui.NewTester(m.frame(m.view), 1000, 760)
	settle(tt)
	return s, tt, calls, posted
}
func browserCallNext(t *testing.T, calls <-chan desktopBrowserCall) desktopBrowserCall {
	t.Helper()
	select {
	case call := <-calls:
		return call
	case <-time.After(time.Second):
		t.Fatal("no browser request")
		return desktopBrowserCall{}
	}
}
func browserPostNext(t *testing.T, posted <-chan func()) {
	t.Helper()
	select {
	case fn := <-posted:
		fn()
	case <-time.After(time.Second):
		t.Fatal("no posted browser callback")
	}
}

func TestBrowserStopSupersedesPendingTakeover(t *testing.T) {
	s, tt, calls, posted := browserWireSheet(t)
	if err := tt.Click(L("Take Over")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	takeover := browserCallNext(t, calls)
	if takeover.method != "browser.takeover" {
		t.Fatal(takeover.method)
	}
	if !s.allowed(model.BrowserStop) || s.allowed(model.BrowserOpen) || s.allowed(model.BrowserResume) {
		t.Fatal("pending input controls")
	}
	if err := tt.Click(L("Stop Browser")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	stop := browserCallNext(t, calls)
	if stop.method != "browser.stop" || stop.params["bot_id"] != s.botID || stop.params["session_id"] != s.selectedID {
		t.Fatal(stop.params)
	}
	s.load()
	stalePoll := browserCallNext(t, calls)
	stopped := browserFixture(s, model.BrowserStopped, true)
	stopped.Sessions[0].Revision = 21
	stop.reply <- model.BrowserReply{Session: &stopped.Sessions[0], Capabilities: stopped.Capabilities}
	browserPostNext(t, posted)
	refresh := browserCallNext(t, calls)
	refresh.reply <- stopped
	browserPostNext(t, posted)
	human := browserFixture(s, model.BrowserHuman, true)
	stalePoll.reply <- human
	browserPostNext(t, posted)
	takeover.reply <- model.BrowserReply{Session: &human.Sessions[0], Capabilities: human.Capabilities}
	browserPostNext(t, posted)
	settle(tt)
	current, _ := s.selected()
	if current.State != model.BrowserStopped || current.Revision != 21 || slices.Contains(tt.Texts(), L("Return to Bot")) {
		t.Fatalf("late takeover: current=%+v epoch=%d pending=%s text=%q", current, s.epoch, s.pending, tt.Texts())
	}
}

func TestBrowserWireResumeScreenshotAndClosedReply(t *testing.T) {
	s, tt, calls, posted := browserWireSheet(t)
	s.apply(browserFixture(s, model.BrowserHuman, true))
	settle(tt)
	if err := tt.Click(L("Return to Bot")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	resume := browserCallNext(t, calls)
	if resume.method != "browser.resume" || resume.params["revision"] != float64(19) {
		t.Fatal(resume.params)
	}
	bot := browserFixture(s, model.BrowserBotControl, true)
	bot.Sessions[0].Revision = 20
	resume.reply <- model.BrowserReply{Session: &bot.Sessions[0]}
	browserPostNext(t, posted)
	refresh := browserCallNext(t, calls)
	refresh.reply <- bot
	browserPostNext(t, posted)
	settle(tt)
	if err := tt.Click(L("Attach Screenshot")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	capture := browserCallNext(t, calls)
	if capture.method != "browser.screenshot" || capture.params["chat_id"] != s.chatID || capture.params["bot_id"] != s.botID || capture.params["session_id"] != s.selectedID {
		t.Fatal(capture.params)
	}
	capture.reply <- model.BrowserReply{MessageID: "synthetic-evidence-reference", SessionID: s.selectedID}
	browserPostNext(t, posted)
	refresh = browserCallNext(t, calls)
	before := s.list.Sessions[0]
	s.close()
	refresh.reply <- browserFixture(s, model.BrowserHuman, true)
	browserPostNext(t, posted)
	if s.list.Sessions[0] != before {
		t.Fatal("dismissed sheet applied a late response")
	}
}

func TestBrowserUnknownCapabilitiesAndReassignmentFailClosed(t *testing.T) {
	s, _, _, _ := browserWireSheet(t)
	list := browserFixture(s, model.BrowserBotControl, true)
	list.Capabilities = model.BrowserCapabilities{}
	s.apply(list)
	for _, op := range []model.BrowserOperation{model.BrowserOpen, model.BrowserTakeover, model.BrowserResume, model.BrowserStop, model.BrowserScreenshot} {
		if s.allowed(op) {
			t.Fatal("missing capability allows", op)
		}
	}
	s.apply(browserFixture(s, model.BrowserBotControl, true))
	s.source.Bots[0].RunnerID = "another-runner"
	if s.allowed(model.BrowserOpen) || s.allowed(model.BrowserStop) {
		t.Fatal("stale Runner assignment allows input")
	}
}

func TestBrowserInspectorEntry(t *testing.T) {
	m := demoWindow(t)
	bot := store.Bot("bot-nova")
	runner := store.Device(bot.RunnerID)
	runner.Plugins = append(runner.Plugins, model.InstalledPlugin{ID: "playwright", Name: "Browser", State: model.PluginReady})
	tt := ui.NewTester(func(c *ui.Context) { applyTheme(c); m.inspectorPlugins(c, bot); m.sheetsView(c) }, 720, 760)
	settle(tt)
	if err := tt.Click(L("Manage…")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !m.hasSheet() || !tt.HasText(L("Create Profile")) {
		t.Fatal(tt.Texts())
	}
	for _, sh := range m.sheets {
		sh.dismiss()
	}
}
