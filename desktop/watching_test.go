package main

import (
	"encoding/json"
	"fmt"
	"sync"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
)

// The chat on screen, as the main window's events report it, against a CLI that counts a reply
// unread unless `ui.watching` names its chat when it arrives. A reply the user watches arrive
// alerts nobody and counts nowhere; one that arrives behind their back does both.

type watchingCLI struct {
	mu       sync.Mutex
	watching string
	readMark []string
}

func (cli *watchingCLI) Reconnect() {}
func (cli *watchingCLI) Request(method string, params any) (json.RawMessage, error) {
	data, _ := json.Marshal(params)
	var payload struct {
		ChatID *string `json:"chat_id"`
	}
	_ = json.Unmarshal(data, &payload)
	cli.mu.Lock()
	defer cli.mu.Unlock()
	switch method {
	case "ui.watching":
		cli.watching = ""
		if payload.ChatID != nil {
			cli.watching = *payload.ChatID
		}
	case "chats.mark_read":
		cli.readMark = append(cli.readMark, *payload.ChatID)
	}
	return json.RawMessage("{}"), nil
}

func (cli *watchingCLI) watched() string {
	cli.mu.Lock()
	defer cli.mu.Unlock()
	return cli.watching
}

func (cli *watchingCLI) markedRead(chatID string) bool {
	cli.mu.Lock()
	defer cli.mu.Unlock()
	for _, id := range cli.readMark {
		if id == chatID {
			return true
		}
	}
	return false
}

// fakeWindow is where the main window stands as its native window reports it.
type fakeWindow struct{ focused, visible, minimized bool }

func (w fakeWindow) IsFocused() bool   { return w.focused }
func (w fakeWindow) IsVisible() bool   { return w.visible }
func (w fakeWindow) IsMinimized() bool { return w.minimized }

var (
	inFront     = fakeWindow{focused: true, visible: true}
	minimized   = fakeWindow{minimized: true}
	hidden      = fakeWindow{}
	shownNotKey = fakeWindow{visible: true}
)

type watchingFixture struct {
	t      *testing.T
	m      *mainWindow
	cli    *watchingCLI
	chat   *model.Chat
	other  *model.Chat
	sent   int
	notice bool
}

// newWatchingFixture opens the main window on a chat, in front of the frontmost app.
func newWatchingFixture(t *testing.T) *watchingFixture {
	t.Helper()
	m := demoWindow(t)
	previous := store
	f := &watchingFixture{t: t, m: m, cli: &watchingCLI{}}
	store = model.NewStore(f.cli, testPost, false)
	store.Devices, store.Bots, store.Chats = previous.Devices, previous.Bots, previous.Chats
	store.HasIdentity, store.IsConnected, store.IsStarting = previous.HasIdentity, true, false
	var dms []*model.Chat
	for _, chat := range store.Chats {
		chat.UnreadCount, chat.Mute = 0, nil
		if chat.IsBotDM() {
			dms = append(dms, chat)
		}
	}
	if len(dms) < 2 {
		t.Fatal("the demo roster has fewer than two bot chats")
	}
	f.chat, f.other = dms[0], dms[1]
	// One report reaches the CLI: the window opens in front on the chat.
	store.IsStarting = true
	m.selectChat(f.chat.ID)
	app.activeChanged(true)
	f.window(inFront)
	store.IsStarting = false
	app.notifier.watchingChanged()
	f.waitWatching(f.chat.ID, "in front")
	return f
}

func (f *watchingFixture) window(state fakeWindow) {
	app.windowChanged(&f.m.appWindow, state)
	runPosts()
}

// waitWatching waits for the CLI to hear which chat is on screen.
func (f *watchingFixture) waitWatching(want, when string) {
	f.t.Helper()
	deadline := time.Now().Add(time.Second)
	for f.cli.watched() != want {
		if time.Now().After(deadline) {
			f.t.Fatalf("%s: the CLI was told the user watches %q, want %q", when, f.cli.watched(), want)
		}
		time.Sleep(5 * time.Millisecond)
	}
}

// reply is a bot's reply finishing in a chat: the CLI counts it unread unless the chat is watched,
// and the app's listeners hear the new count.
func (f *watchingFixture) reply(chat *model.Chat) {
	f.sent++
	message := &model.Message{ID: fmt.Sprintf("reply-%d", f.sent), Author: model.BotAuthor(chat.BotIDs[0]), Body: model.Body{Kind: model.BodyText, Text: "Done."}, State: model.MessageState{Kind: model.StateComplete}, CreatedAt: time.Now()}
	chat.Messages = append(chat.Messages, message)
	if f.cli.watched() != chat.ID {
		chat.UnreadCount++
	}
	app.notifier.storeChanged(model.Event{Kind: model.EventChatsChanged})
	runPosts()
	f.notice = canDeliver(notificationFor(message), chat, app.notifier.watchedChat())
}

// expectRead is the reply seen: no notification, no unread in the sidebar, no badge.
func (f *watchingFixture) expectRead(when string) {
	f.t.Helper()
	if f.notice || f.chat.UnreadCount != 0 || unreadBadge() != 0 {
		f.t.Fatalf("%s: notification %v, unread %d, badge %d; want none", when, f.notice, f.chat.UnreadCount, unreadBadge())
	}
}

// expectUnread is the reply behind the user's back: a notification, an unread count, a badge.
func (f *watchingFixture) expectUnread(chat *model.Chat, when string) {
	f.t.Helper()
	if !f.notice || chat.UnreadCount == 0 || unreadBadge() == 0 {
		f.t.Fatalf("%s: notification %v, unread %d, badge %d; want all three", when, f.notice, chat.UnreadCount, unreadBadge())
	}
}

// Restoring a minimized window, or showing a hidden one, puts the chat back on screen whether or
// not the window took the keyboard: the reply that arrived meanwhile is read at once, and the next
// one is watched arriving.
func TestRestoredWindowWatchesItsChat(t *testing.T) {
	for _, away := range []struct {
		name  string
		state fakeWindow
	}{{"minimized", minimized}, {"hidden", hidden}} {
		t.Run(away.name, func(t *testing.T) {
			f := newWatchingFixture(t)
			f.waitWatching(f.chat.ID, "in front")
			f.reply(f.chat)
			f.expectRead("in front")

			f.window(away.state)
			f.waitWatching("", away.name)
			f.reply(f.chat)
			f.expectUnread(f.chat, away.name)

			// The app stays frontmost and AppKit orders the window back in without making it key,
			// as a window restored from the Dock's thumbnail.
			f.window(shownNotKey)
			f.waitWatching(f.chat.ID, "restored")
			if f.chat.UnreadCount != 0 || unreadBadge() != 0 || !f.cli.markedRead(f.chat.ID) {
				t.Fatalf("restored: unread %d, badge %d, marked read %v", f.chat.UnreadCount, unreadBadge(), f.cli.markedRead(f.chat.ID))
			}
			f.reply(f.chat)
			f.expectRead("restored")
		})
	}
}

// The window takes the keyboard before AppKit finishes deminiaturizing it, so the focus event
// still reads it minimized; the reply that lands before the restore event is read once it comes.
func TestFocusDuringDeminiaturizeWatchesItsChat(t *testing.T) {
	f := newWatchingFixture(t)
	f.window(minimized)
	f.waitWatching("", "minimized")

	f.window(fakeWindow{focused: true, visible: true, minimized: true})
	f.reply(f.chat)
	f.window(inFront)
	f.waitWatching(f.chat.ID, "restored")
	f.expectRead("restored")
	f.reply(f.chat)
	f.expectRead("after the restore")
}

// A window behind another app, minimized, or hidden, and another chat on screen, leave a reply
// unread and alert.
func TestReplyBehindTheUsersBackStillAlerts(t *testing.T) {
	for _, test := range []struct {
		name string
		away func(f *watchingFixture)
	}{
		// AppKit takes the keyboard from the window and then says the app resigned.
		{"another app in front", func(f *watchingFixture) { f.window(shownNotKey); app.activeChanged(false) }},
		{"minimized", func(f *watchingFixture) { f.window(minimized) }},
		{"hidden", func(f *watchingFixture) { f.window(hidden) }},
	} {
		t.Run(test.name, func(t *testing.T) {
			f := newWatchingFixture(t)
			f.waitWatching(f.chat.ID, "in front")
			test.away(f)
			f.waitWatching("", test.name)
			f.reply(f.chat)
			f.expectUnread(f.chat, test.name)
		})
	}
	t.Run("another chat on screen", func(t *testing.T) {
		f := newWatchingFixture(t)
		f.m.selectChat(f.other.ID)
		f.waitWatching(f.other.ID, "the other chat")
		f.reply(f.chat)
		f.expectUnread(f.chat, "the other chat")
	})
}

// Reports that follow each other at once reach the CLI in their order: a window minimized and
// restored right away leaves the CLI on the chat on screen, every time.
func TestWatchedChatReachesTheCLIInOrder(t *testing.T) {
	f := newWatchingFixture(t)
	wrong := 0
	for i := 0; i < 50; i++ {
		f.window(minimized)
		f.window(inFront)
		time.Sleep(20 * time.Millisecond)
		if f.cli.watched() != f.chat.ID {
			wrong++
		}
	}
	if wrong > 0 {
		t.Fatalf("the CLI ended on %q, not the chat on screen, %d of 50 times", f.cli.watched(), wrong)
	}
	f.reply(f.chat)
	f.expectRead("after minimizing and restoring")
}
