package main

import (
	"image/png"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// pendingPosts are what the store posted to the main thread in a test, run between frames.
var pendingPosts []func()

func runPosts() {
	for len(pendingPosts) > 0 {
		batch := pendingPosts
		pendingPosts = nil
		for _, fn := range batch {
			fn()
		}
	}
}

// demoWindow is the main window over the demo's store, as `LORCA_MOCK=1` shows it, without a
// window of its own.
func demoWindow(t *testing.T) *mainWindow {
	t.Helper()
	l10n.Set("en", "en-US")
	// Each test starts from the defaults, saving where no other run reads.
	prefs.loadFrom(filepath.Join(t.TempDir(), "preferences.json"))
	store = model.NewStore(nil, func(fn func()) { pendingPosts = append(pendingPosts, fn) }, true)
	app = &appDelegate{notifier: newNotifier()}
	store.Start()
	m := newMainWindow()
	m.userWantsInspector = true
	app.main = m
	m.restoreSelection()
	return m
}

// renderTo writes the tester's last frame to $LORCA_RENDER/name.png, for looking at a view.
func renderTo(t *testing.T, tt *ui.Tester, name string) {
	t.Helper()
	dir := os.Getenv("LORCA_RENDER")
	if dir == "" {
		return
	}
	file, err := os.Create(filepath.Join(dir, name+".png"))
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	if err := png.Encode(file, tt.Image()); err != nil {
		t.Fatal(err)
	}
}

func TestRenderMainWindow(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	runPosts()
	for range 4 {
		tt.Frame()
	}
	if r, ok := tt.Find(L("Message %@ — @ to address one bot", "Launch room")); ok {
		t.Logf("composer field %+v", r)
	} else {
		t.Logf("no composer field")
	}
	renderTo(t, tt, "main-light")
	tt.SetDark(true)
	tt.Frame()
	renderTo(t, tt, "main-dark")
}

func TestRenderPalette(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	runPosts()
	tt.Frame()
	m.palette.show()
	tt.Frame()
	renderTo(t, tt, "palette")
	tt.Type("developer")
	tt.Frame()
	if !tt.HasText("Developer") {
		t.Errorf("palette does not find Developer: %q", tt.Texts())
	}
	renderTo(t, tt, "palette-query")
	tt.Key(0, ui.KeyEnter)
	tt.Frame()
	if m.palette.open || m.selection.ChatID != "chat-patch" {
		t.Errorf("Return left the palette open (%v) on %+v", m.palette.open, m.selection)
	}
}

func TestRenderRunningCommand(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	run := &model.CommandRun{SessionID: "s1", Command: "npm install\nnpm test", State: model.CommandWaiting, Prompt: "Proceed? [y/N]", Output: "added 120 packages\nProceed? [y/N]", Device: "Studio", HandedOver: true}
	store.Append(&model.Message{
		ID:        "msg-run",
		Author:    model.BotAuthor("bot-patch"),
		Body:      model.Body{Kind: model.BodyTool, Tool: &model.ToolInvocation{Name: "bash", Summary: "$ npm install", Description: "Install dependencies", IsRunning: true, Run: run}},
		State:     model.MessageState{Kind: model.StateStreaming},
		CreatedAt: time.Now(),
	}, "chat-patch")
	time.Sleep(model.TaskDelay + 100*time.Millisecond)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	runPosts()
	for range 3 {
		tt.Frame()
	}
	renderTo(t, tt, "command-card")
	if err := tt.Click(L("Running tasks (%d)", 1)); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	renderTo(t, tt, "running-tasks")
	if !tt.HasText(L("Run in Background")) {
		t.Errorf("no Run in Background in %q", tt.Texts())
	}
}

// settle runs what the store posted and draws a few frames.
func settle(tt *ui.Tester) {
	for range 6 {
		runPosts()
		tt.Frame()
	}
}

// settleTransitions waits out the sheet's entrance and recoloring, which run on the clock.
func settleTransitions(tt *ui.Tester) {
	time.Sleep(250 * time.Millisecond)
	settle(tt)
}

// renderBoth writes the frame in the light and the dark appearance.
func renderBoth(t *testing.T, tt *ui.Tester, name string) {
	t.Helper()
	settleTransitions(tt)
	renderTo(t, tt, name+"-light")
	tt.SetDark(true)
	settleTransitions(tt)
	renderTo(t, tt, name+"-dark")
	tt.SetDark(false)
	settleTransitions(tt)
}
