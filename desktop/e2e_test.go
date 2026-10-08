package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The main window over a real CLI, drawn offscreen: it adds a custom provider through its sheet,
// runs the first bot on it, and sends a message whose reply streams into the transcript.
//
// It needs a CLI with an identity on LORCA_E2E_PORT, and an OpenAI-compatible server on
// LORCA_E2E_SERVER (a base URL ending in /v1) that lists a model and answers chat completions.
func TestE2EAgainstCLI(t *testing.T) {
	port, server := os.Getenv("LORCA_E2E_PORT"), os.Getenv("LORCA_E2E_SERVER")
	if port == "" || server == "" {
		t.Skip("LORCA_E2E_PORT and LORCA_E2E_SERVER name a running CLI and model server")
	}
	t.Setenv("LORCA_PORT", port)
	l10n.Set("en", "en-US")
	prefs.loadFrom(filepath.Join(t.TempDir(), "preferences.json"))

	// What the CLI and the store hand the main thread, run by drain as the app's queue does.
	var mu sync.Mutex
	var queue []func()
	enqueue := func(fn func()) {
		mu.Lock()
		queue = append(queue, fn)
		mu.Unlock()
	}
	drain := func() {
		mu.Lock()
		pending := queue
		queue = nil
		mu.Unlock()
		for _, fn := range pending {
			fn()
		}
	}
	client := newCLIClient()
	store = model.NewStore(cliTransport{client}, enqueue, false)
	app = &appDelegate{notifier: newNotifier(), cli: client}
	client.onEvent = func(name string, frame []byte) {
		var payload struct {
			Data json.RawMessage `json:"data"`
		}
		_ = json.Unmarshal(frame, &payload)
		enqueue(func() { store.HandleEvent(name, payload.Data) })
	}
	client.onState = func(state string) {
		enqueue(func() {
			store.CLIStateChanged(model.CLIState{Connection: state, Launcher: model.LauncherStatus{Kind: "ready"}})
		})
	}
	store.Start()
	client.connect()
	t.Cleanup(client.disconnect)

	m := newMainWindow()
	app.main = m
	store.Subscribe(func(event model.Event) { m.storeChanged(event) })
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	step := func() {
		drain()
		tt.Frame()
	}
	waitFor := func(what string, timeout time.Duration, done func() bool) {
		t.Helper()
		deadline := time.Now().Add(timeout)
		for !done() {
			if time.Now().After(deadline) {
				renderTo(t, tt, "e2e-timeout")
				t.Fatalf("timed out waiting for %s; texts %q", what, tt.Texts())
			}
			step()
			time.Sleep(20 * time.Millisecond)
		}
		step()
	}
	waitFor("the account", 15*time.Second, func() bool { return store.IsConnected && len(store.Bots) > 0 })
	m.restoreSelection()
	step()
	bot := store.Bots[0]
	renderTo(t, tt, "e2e-main")

	// Add the server through the custom provider sheet.
	var kind model.ProviderKind
	m.presentCustomProvider("", nil, func(saved model.ProviderKind) { kind = saved })
	waitFor("the sheet", 2*time.Second, m.hasSheet)
	time.Sleep(250 * time.Millisecond)
	step()
	tt.Type("Fake")
	// The base URL field sits right above its note.
	note, ok := tt.Find(L("Lorca adds %@ to it.", "/chat/completions"))
	if !ok {
		t.Fatalf("no base URL note: %q", tt.Texts())
	}
	tt.ClickAt(note.X+120, note.Y-16)
	tt.Type(server)
	waitFor("the server's models", 10*time.Second, func() bool { return tt.HasText("fake-model") })
	renderTo(t, tt, "e2e-custom-provider")
	if err := tt.Click(L("Add")); err != nil {
		t.Fatal(err)
	}
	waitFor("the provider", 10*time.Second, func() bool { return kind != "" && !m.hasSheet() })
	if !strings.HasPrefix(string(kind), "custom:") {
		t.Fatalf("saved %q", kind)
	}

	// The first bot runs on it.
	store.SetBotRuntime(bot.ID, kind, "fake-model", "")
	waitFor("the bot's provider", 5*time.Second, func() bool {
		b := store.Bot(bot.ID)
		return b != nil && b.Provider == kind
	})

	// A message, and its reply.
	chat := store.Chat(store.DM(bot.ID))
	if chat == nil {
		t.Fatalf("no chat with %s", bot.Name)
	}
	m.selectChat(chat.ID)
	step()
	composer, ok := tt.Find(L("Message %@", bot.Name))
	if !ok {
		t.Fatalf("no composer: %q", tt.Texts())
	}
	tt.ClickAt(composer.X+composer.W/2, composer.Y+composer.H/2)
	tt.Type("Hello there")
	tt.Key(0, ui.KeyEnter)
	waitFor("the reply", 30*time.Second, func() bool {
		chat := store.Chat(chat.ID)
		if chat == nil {
			return false
		}
		for _, message := range chat.Messages {
			if message.Author.Kind == model.AuthorBot && message.Body.Kind == model.BodyText &&
				strings.Contains(message.Body.Text, "fake model") && message.State.Kind == model.StateComplete {
				return true
			}
		}
		return false
	})
	waitFor("the bot to finish", 10*time.Second, func() bool { return !store.IsWorking(bot.ID) })
	time.Sleep(300 * time.Millisecond)
	step()
	renderTo(t, tt, "e2e-reply")
	if !tt.HasText("Hello there") || !tt.HasText("fake model") {
		t.Errorf("the transcript shows %q", tt.Texts())
	}
}
