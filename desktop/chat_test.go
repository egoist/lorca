package main

import (
	"fmt"
	"strings"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Wheeling a long transcript down to its end moves its rows by the wheel's steps, and no further
// once it is there, as the button to the latest goes.
func TestTranscriptScrollsSteadilyToEnd(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1000, 700)
	runPosts()
	chat := store.Chat(m.chat.chatID)
	bot := model.BotAuthor(store.BotsIn(chat)[0].ID)
	const count = 120
	texts := make([]string, count)
	var messages []*model.Message
	at := time.Now().Add(-time.Hour)
	for i := range count {
		author := model.You
		if i%2 == 1 {
			author = bot
		}
		texts[i] = fmt.Sprintf("Message %d.", i) + strings.Repeat(" Words that wrap the paragraph onto another line.", (i*7)%11)
		text := texts[i]
		if i%9 == 0 {
			text += "\n\n```\none\ntwo\n```"
		}
		messages = append(messages, &model.Message{ID: fmt.Sprintf("long-%d", i), Author: author, Body: model.Body{Kind: model.BodyText, Text: text},
			CreatedAt: at.Add(time.Duration(i) * time.Second), State: model.MessageState{Kind: model.StateComplete}})
	}
	chat.Messages, chat.HasMore = messages, false
	for range 5 {
		tt.Frame()
	}
	time.Sleep(300 * time.Millisecond)
	tt.Frame()
	m.chat.list.ScrollTo(count-12, ui.Start)
	settle(tt)
	transcript, ok := tt.Find(L("Transcript"))
	if !ok {
		t.Fatalf("no transcript: %q", tt.Texts())
	}
	// Where each message's first paragraph starts, for those whole below the transcript's top.
	tops := func() map[int]float32 {
		out := map[int]float32{}
		for i, text := range texts {
			if r, ok := tt.Find(text); ok && r.Y > transcript.Y+1 {
				out[i] = r.Y
			}
		}
		return out
	}
	const step = 30
	before, atEnd := tops(), 0
	for frame := 0; atEnd < 8; frame++ {
		if frame > 400 {
			t.Fatal("never reached the end")
		}
		tt.Scroll(transcript.X+transcript.W/2, transcript.Y+200, 0, step)
		tt.Frame()
		now := tops()
		for i, y := range now {
			if was, ok := before[i]; ok && (y > was+0.1 || y < was-step-0.1) {
				t.Fatalf("frame %d: message %d moved %.1f for a %d-DIP step", frame, i, y-was, step)
			}
		}
		before = now
		if m.chat.list.AtEnd() {
			atEnd++
		}
	}
}
