package main

import (
	"slices"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

// A drag runs across a message's paragraphs, list, and code, and Copy joins them a line apart,
// without the list's markers.
func TestMessageSelectsAcrossBlocks(t *testing.T) {
	text := "First paragraph.\n\nSecond paragraph.\n\n- one\n- two\n\n```\ncode line\n```"
	tt := ui.NewTester(func(c *ui.Context) {
		applyTheme(c)
		ui.Column(c).Padding(20).Width(360).Children(func() { markdownView(c, text, markdownOptions{}) })
	}, 400, 400)
	first, ok := tt.Find("First paragraph.")
	if !ok {
		t.Fatalf("no first paragraph: %q", tt.Texts())
	}
	last, ok := tt.Find("code line")
	if !ok {
		t.Fatalf("no code: %q", tt.Texts())
	}
	tt.Press(first.X+1, first.Y+first.H/2)
	tt.Move(last.X+last.W/2, last.Y+last.H/2)
	tt.Release(last.X+last.W-1, last.Y+last.H/2)
	tt.Key(ui.Cmd, ui.KeyC)
	want := "First paragraph.\nSecond paragraph.\none\ntwo\ncode line"
	if got := tt.Clipboard(); got != want {
		t.Errorf("copied %q, want %q", got, want)
	}
}

// A right-click on a message's text offers Reply above Copy and Select All, as the Mac's does:
// Reply starts a reply to it, and the others act on its text.
func TestMessageTextMenu(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1000, 700)
	runPosts()
	// The transcript settles at its end over its first frames.
	for range 5 {
		tt.Frame()
	}
	time.Sleep(300 * time.Millisecond)
	tt.Frame()
	bubble, ok := tt.Find("Great. Keep the announcement as a draft until I've reviewed it.")
	if !ok {
		t.Fatalf("no message: %q", tt.Texts())
	}
	tt.RightClickAt(bubble.X+40, bubble.Y+20)
	if want := []string{L("Reply"), "-", "Copy", "-", "Select All"}; !slices.Equal(tt.Menu(), want) {
		t.Fatalf("menu %q, want %q", tt.Menu(), want)
	}
	if err := tt.ChooseMenuItem(L("Reply")); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	tt.Frame()
	if m.chat.composer.reply == nil {
		t.Error("Reply started no reply")
	}
	// Select All and Copy from the same menu take the message's text. The reply's bar moved it.
	for _, item := range []string{"Select All", "Copy"} {
		for range 3 {
			tt.Frame()
		}
		bubble, _ = tt.Find("Great. Keep the announcement as a draft until I've reviewed it.")
		tt.RightClickAt(bubble.X+40, bubble.Y+20)
		if err := tt.ChooseMenuItem(item); err != nil {
			t.Fatal(err)
		}
		tt.Frame()
	}
	if got := tt.Clipboard(); got != "Great. Keep the announcement as a draft until I've reviewed it." {
		t.Errorf("copied %q", got)
	}
}
