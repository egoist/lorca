package main

import (
	"image/color"
	"testing"
	"time"

	"github.com/egoist/mygo/ui"
)

func TestChatSelectionPaintsOnPressFromOutsideSidebar(t *testing.T) {
	m := demoWindow(t)
	runPosts()
	old, next := store.Chats[0], store.Chats[1]
	m.selectChat(old.ID)
	var paintedSelection string
	var p *palette
	frame := m.frame(m.view)
	tt := ui.NewTester(func(c *ui.Context) {
		paintedSelection, p = m.selection.ChatID, colors(c)
		frame(c)
	}, 1000, 700)
	// Take focus away without changing the selected chat.
	placeholder := chatPlaceholder(old, store.BotsIn(old))
	if err := tt.Click(placeholder); err != nil {
		t.Fatal(err)
	}
	if !tt.Focused(placeholder) {
		t.Fatal("the composer did not take focus")
	}
	oldRow, ok := tt.Find(store.Title(old))
	if !ok {
		t.Fatal("the selected chat row is missing")
	}
	row, ok := tt.Find(store.Title(next))
	if !ok {
		t.Fatal("the next chat row is missing")
	}
	pixel := func(r ui.Rect) color.RGBA {
		return tt.Image().RGBAAt(int(r.X+r.W-6), int(r.Y+r.H/2))
	}
	if got := pixel(oldRow); int(got.B) > int(got.R)+30 {
		t.Fatalf("the sidebar still has focus: %v", got)
	}
	tt.Press(row.X+row.W/2, row.Y+row.H/2)
	if m.selection.ChatID != next.ID || paintedSelection != next.ID {
		t.Fatalf("selected %q, painted %q, want %q on pointer down", m.selection.ChatID, paintedSelection, next.ID)
	}
	if got := pixel(oldRow); int(got.B) > int(got.R)+30 {
		t.Fatalf("the old chat flashed blue: %v", got)
	}
	want := color.RGBA{R: p.Selection.R, G: p.Selection.G, B: p.Selection.B, A: 255}
	if got := pixel(row); got != want {
		t.Fatalf("the new chat's fill is %v, want %v", got, want)
	}
	tt.Release(row.X+row.W/2, row.Y+row.H/2)
	if m.selection.ChatID != next.ID {
		t.Fatal("release changed the selection again")
	}
}

// Holding Cmd (Ctrl on Windows and Linux) alone for a moment puts each of the first chats' shortcuts
// in its stamp, and letting go takes them away.
func TestChatShortcutHints(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1000, 700)
	runPosts()
	tt.Frame()
	tt.HoldModifiers(ui.Cmd)
	if tt.HasText(shortcutText("CmdOrCtrl+1")) {
		t.Fatal("hints at once")
	}
	time.Sleep(300 * time.Millisecond)
	tt.Frame()
	renderTo(t, tt, "sidebar-shortcut-hints")
	if !tt.HasText(shortcutText("CmdOrCtrl+1")) || !tt.HasText(shortcutText("CmdOrCtrl+7")) {
		t.Fatalf("no hints: %q", tt.Texts())
	}
	tt.HoldModifiers(0)
	if tt.HasText(shortcutText("CmdOrCtrl+1")) {
		t.Fatal("hints stayed")
	}
}
