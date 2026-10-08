package main

import (
	"reflect"
	"strings"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A draft's mentions are found whatever their case, the longer name winning where two start
// alike, in runes.
func TestMentionRuns(t *testing.T) {
	project := &model.Bot{ID: "p", Name: "Project"}
	manager := &model.Bot{ID: "pm", Name: "Project Manager"}
	scout := &model.Bot{ID: "s", Name: "Scout"}
	bots := []*model.Bot{project, manager, scout}
	got := mentionRuns([]rune("éé @project manager, ask @SCOUT and @Project. @nobody @"), bots)
	want := []mentionRun{{3, 19, manager}, {25, 31, scout}, {36, 44, project}}
	if !reflect.DeepEqual(got, want) {
		t.Errorf("got %+v, want %+v", got, want)
	}
	if runs := mentionRuns([]rune("no mentions"), bots); len(runs) != 0 {
		t.Errorf("found %+v", runs)
	}
}

func TestComposerClearUpdatesTranscriptWithoutMoreInput(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1000, 700)
	runPosts()
	chat := store.Chat(m.chat.chatID)
	chat.Messages[len(chat.Messages)-1].Body.Text = "Latest reply."
	for range 3 {
		tt.Frame()
	}
	latest, ok := tt.Find("Latest reply.")
	if !ok {
		t.Fatal("the latest reply is not visible")
	}
	placeholder := chatPlaceholder(chat, store.BotsIn(chat))
	compact, ok := tt.Find(placeholder)
	if !ok {
		t.Fatal("the composer is not visible")
	}
	if err := tt.Click(placeholder); err != nil {
		t.Fatal(err)
	}
	tt.Type(strings.Repeat("Draft line with some words.\n", 7))
	for range 3 {
		tt.Frame()
	}
	expanded, ok := tt.Find(placeholder)
	if !ok || expanded.H <= compact.H {
		t.Fatal("the multiline draft did not expand the composer")
	}
	renderTo(t, tt, "composer-expanded")
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Command("delete")
	// Tester settles only the frames the app requests. Do not inject another frame after clearing.
	renderTo(t, tt, "composer-cleared")
	if m.chat.composer.draft != "" {
		t.Fatal("the draft was not cleared")
	}
	cleared, ok := tt.Find(placeholder)
	if !ok || cleared.H != compact.H {
		t.Fatalf("composer height after clearing %.1f, want %.1f", cleared.H, compact.H)
	}
	got, ok := tt.Find("Latest reply.")
	if !ok || got.Y < latest.Y-1 || got.Y > latest.Y+1 {
		t.Fatalf("latest reply stayed at %.1f after clearing, want %.1f", got.Y, latest.Y)
	}
}

// The composer shows a mention in its bot's color.
func TestRenderComposerMention(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1000, 700)
	runPosts()
	tt.Frame()
	chat := m.chat
	if chat == nil {
		t.Fatal("no chat on screen")
	}
	members := store.BotsIn(store.Chat(chat.chatID))
	chat.composer.draft = "@" + members[0].Name + " can you check this?"
	tt.Frame()
	tt.Frame()
	renderTo(t, tt, "composer-mention")
	field, ok := tt.Find(chatPlaceholder(store.Chat(chat.chatID), members))
	if !ok {
		t.Fatalf("no composer: %q", tt.Texts())
	}
	accent := lightPalette.accentColor(members[0].Accent)
	img := tt.Image()
	tinted := 0
	for y := int(field.Y); y < int(field.Y+field.H); y++ {
		for x := int(field.X); x < int(field.X+field.W); x++ {
			px := img.RGBAAt(x, y)
			if absDiff(px.R, accent.R) < 24 && absDiff(px.G, accent.G) < 24 && absDiff(px.B, accent.B) < 24 {
				tinted++
			}
		}
	}
	if tinted < 20 {
		t.Errorf("the mention is not in %s's color (%d pixels)", members[0].Name, tinted)
	}
}

func absDiff(a, b uint8) int {
	if a > b {
		return int(a - b)
	}
	return int(b - a)
}
