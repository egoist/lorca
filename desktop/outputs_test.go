package main

import (
	"encoding/json"
	"errors"
	"image"
	"image/color"
	"image/png"
	"os"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

type outputTestTransport struct {
	request func(string, any) (json.RawMessage, error)
}

func (f outputTestTransport) Request(method string, params any) (json.RawMessage, error) {
	return f.request(method, params)
}
func (outputTestTransport) Reconnect() {}

func outputFixture(version uint32, mime string) *model.Message {
	code := 0
	return &model.Message{ID: "msg-fixture-v" + string(rune('0'+version)), Author: model.BotAuthor("bot-patch"), Body: model.Body{Kind: model.BodyText, Text: "Synthetic fixture output"},
		CreatedAt: time.Date(2026, 10, 8, 9, int(version), 0, 0, time.UTC), Output: &model.Output{ID: "out-fixture", Name: "Verification report", Mime: mime, BotID: "bot-patch", ChatID: "chat-patch", TaskID: "task-3a249410-8034-4be2-bf42-9e68b4fe2c41", Version: version,
			Evidence: &model.OutputEvidence{Kind: "test_result", Summary: "Synthetic focused checks pass", Status: "passed", Command: "sample-test --layout", ExitCode: &code}}}
}

func pumpOutputReply(t *testing.T, posts <-chan func(), tt *ui.Tester) {
	t.Helper()
	select {
	case reply := <-posts:
		reply()
	case <-time.After(3 * time.Second):
		t.Fatal("missing output reply")
	}
	settle(tt)
}

func TestOutputsHeaderVersionsAndPreviewNavigation(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	chat := store.Chat("chat-patch")
	first, second := outputFixture(1, "text/html"), outputFixture(2, "text/html")
	first.Output.URL, second.Output.URL = "https://docs.example.com/report", "https://docs.example.com/report"
	first.Output.Evidence.Status = "failed"
	first.Output.Evidence.Summary = "Synthetic layout check fails"
	failed := 1
	first.Output.Evidence.ExitCode = &failed
	second.Output.PreviousMessageID = first.ID
	chat.Messages = []*model.Message{first, second}
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settle(tt)
	renderTo(t, tt, "outputs-header")
	if err := tt.Click(L("Outputs")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if !tt.HasText("Verification report · v2") || !tt.HasText("Verification report · v1") || !tt.HasText(L("Exit code: %d", 1)) {
		t.Fatalf("versions/evidence absent: %q", tt.Texts())
	}
	secondRect, _ := tt.Find("Verification report · v2")
	firstRect, _ := tt.Find("Verification report · v1")
	if secondRect.Y >= firstRect.Y {
		t.Fatal("latest version is not first")
	}
	renderBoth(t, tt, "outputs-versions")
	tt.Key(0, ui.KeyEscape)
	settle(tt)
	if m.hasSheet() {
		t.Fatal("Escape did not close Outputs")
	}
}

func TestOutputsRetrievalRetryPreviewOpenAndSave(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	m.userWantsInspector = false
	source := filepath.Join(t.TempDir(), "Tests.txt")
	if err := os.WriteFile(source, []byte("Synthetic checks passed\nNo private data\n"), 0600); err != nil {
		t.Fatal(err)
	}
	message := outputFixture(2, "text/plain")
	message.Output.Name = "Tests.txt"
	message.Attachments = []model.Attachment{{ID: "att-fixture", Name: "Tests.txt", Mime: "text/plain", Size: 40}}
	posts := make(chan func(), 12)
	var fetches atomic.Int32
	transport := outputTestTransport{func(method string, params any) (json.RawMessage, error) {
		switch method {
		case "outputs.list":
			data, _ := json.Marshal(map[string]any{"outputs": []any{map[string]any{"id": message.ID, "chat_id": "chat-patch", "author": map[string]any{"kind": "bot", "bot_id": "bot-patch"}, "body": map[string]any{"kind": "text", "text": "Synthetic report", "attachments": []any{map[string]any{"id": "att-fixture", "name": "Tests.txt", "mime": "text/plain", "size": 40}}}, "state": map[string]any{"kind": "complete"}, "created_at": 1728000000, "output": message.Output}}})
			return data, nil
		case "files.path":
			if params.(map[string]any)["named"] != true {
				return nil, errors.New("missing named retrieval")
			}
			if fetches.Add(1) == 1 {
				return nil, errors.New("relay no longer has Tests.txt")
			}
			data, _ := json.Marshal(map[string]any{"path": source})
			return data, nil
		default:
			return json.RawMessage(`null`), nil
		}
	}}
	old := store
	store = model.NewStore(transport, func(fn func()) { posts <- fn }, false)
	store.Chats, store.Bots, store.Devices = old.Chats, old.Bots, old.Devices
	store.Chats[0] = old.Chats[0]
	store.IsStarting = false
	store.HasIdentity = old.HasIdentity
	store.IsConnected = true
	store.Chat("chat-patch").Messages = []*model.Message{message}
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	m.presentOutputs("chat-patch")
	pumpOutputReply(t, posts, tt)
	pumpOutputReply(t, posts, tt)
	settleTransitions(tt)
	if !tt.HasText("File unavailable: relay no longer has Tests.txt") {
		t.Fatalf("no unavailable reason: %q", tt.Texts())
	}
	renderBoth(t, tt, "outputs-unavailable")
	if err := tt.Click(L("Retry %@", "Tests.txt")); err != nil {
		t.Fatal(err)
	}
	pumpOutputReply(t, posts, tt)
	if fetches.Load() != 2 {
		t.Fatal("Retry did not fetch once")
	}
	renderBoth(t, tt, "outputs-file-ready")
	if err := tt.Click(L("Preview %@", "Tests.txt")); err != nil {
		t.Fatal(err)
	}
	pumpOutputReply(t, posts, tt)
	settleTransitions(tt)
	if !tt.HasText("Synthetic checks passed") {
		t.Fatalf("native text preview absent: %q", tt.Texts())
	}
	renderBoth(t, tt, "outputs-text-preview")
	tt.Key(0, ui.KeyEscape)
	settle(tt)
	if len(m.sheets) != 1 {
		t.Fatal("preview did not return to Outputs")
	}
	opened := make(chan string, 1)
	originalOpen := outputOpenPath
	outputOpenPath = func(path string) error { opened <- path; return nil }
	t.Cleanup(func() { outputOpenPath = originalOpen })
	if err := tt.Click(L("Open %@", "Tests.txt")); err != nil {
		t.Fatal(err)
	}
	pumpOutputReply(t, posts, tt)
	if got := <-opened; got != source {
		t.Fatalf("opened runner path instead of local retrieval: %s", got)
	}
	destination := filepath.Join(t.TempDir(), "Saved Tests.txt")
	originalSave := outputSaveDestination
	outputSaveDestination = func(_ *mygo.Window, name string) (string, error) {
		if name != "Tests.txt" {
			return "", errors.New("wrong name")
		}
		return destination, nil
	}
	t.Cleanup(func() { outputSaveDestination = originalSave })
	if err := tt.Click(L("Save %@", "Tests.txt")); err != nil {
		t.Fatal(err)
	}
	pumpOutputReply(t, posts, tt)
	saved, err := os.ReadFile(destination)
	if err != nil || string(saved) != "Synthetic checks passed\nNo private data\n" {
		t.Fatalf("bad saved bytes: %q %v", saved, err)
	}
}

func TestOutputsStaleRepliesCannotReplaceNewerOrClosedSheet(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	old := store
	posts := make(chan func(), 4)
	requests := make(chan chan json.RawMessage, 4)
	store = model.NewStore(outputTestTransport{func(method string, params any) (json.RawMessage, error) {
		reply := make(chan json.RawMessage, 1)
		requests <- reply
		return <-reply, nil
	}}, func(fn func()) { posts <- fn }, false)
	store.Chats, store.Bots = old.Chats, old.Bots
	state := &outputsState{chatID: "chat-patch", actions: map[string]*outputActionState{}}
	state.sheet = m.present(func(c *ui.Context, s *sheet) { state.view(c) }, nil)
	state.reload()
	first := <-requests
	state.reload()
	second := <-requests
	second <- json.RawMessage(`{"outputs":[],"has_more":true}`)
	(<-posts)()
	if !state.hasMore {
		t.Fatal("new response not applied")
	}
	first <- json.RawMessage(`{"outputs":[],"has_more":false}`)
	(<-posts)()
	if !state.hasMore {
		t.Fatal("stale response overwrote current state")
	}
	state.reload()
	last := <-requests
	state.sheet.dismiss()
	last <- json.RawMessage(`{"outputs":[],"has_more":false}`)
	(<-posts)()
	if !state.hasMore {
		t.Fatal("closed sheet callback mutated state")
	}
}

func TestOutputImagePreview(t *testing.T) {
	m := demoWindow(t)
	file, err := os.Create(filepath.Join(t.TempDir(), "Synthetic chart.png"))
	if err != nil {
		t.Fatal(err)
	}
	picture := image.NewRGBA(image.Rect(0, 0, 320, 180))
	for y := range 180 {
		for x := range 320 {
			value := color.RGBA{240, 246, 255, 255}
			if x > 30 && x < 80 && y > 90 || x > 120 && x < 170 && y > 60 || x > 210 && x < 260 && y > 30 {
				value = color.RGBA{55, 120, 220, 255}
			}
			picture.SetRGBA(x, y, value)
		}
	}
	if err := png.Encode(file, picture); err != nil {
		t.Fatal(err)
	}
	file.Close()
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	m.presentOutputPreview(model.Attachment{ID: "att-chart", Name: "Synthetic chart.png", Mime: "image/png", Size: 1000}, file.Name())
	deadline := time.Now().Add(3 * time.Second)
	for tt.HasText(L("Loading preview…")) || len(m.sheets) == 1 {
		runPosts()
		tt.Frame()
		if !tt.HasText(L("Loading preview…")) {
			break
		}
		if time.Now().After(deadline) {
			t.Fatal("preview did not load")
		}
		time.Sleep(10 * time.Millisecond)
	}
	settleTransitions(tt)
	renderBoth(t, tt, "outputs-image-preview")
	if strings.Contains(strings.Join(tt.Texts(), " "), "Preview unavailable:") {
		t.Fatal("image preview failed")
	}
}

func TestCopyOutputPreservesDestinationOnFailure(t *testing.T) {
	root := t.TempDir()
	target := filepath.Join(root, "saved.txt")
	os.WriteFile(target, []byte("existing"), 0600)
	if err := copyOutput(filepath.Join(root, "missing"), target); err == nil {
		t.Fatal("missing source succeeded")
	}
	data, _ := os.ReadFile(target)
	if string(data) != "existing" {
		t.Fatal("failed save damaged destination")
	}
	if err := copyOutput(target, target); err != nil {
		t.Fatal(err)
	}
	source := filepath.Join(root, "source.txt")
	os.WriteFile(source, []byte("new result"), 0600)
	if err := copyOutput(source, target); err != nil {
		t.Fatal(err)
	}
	data, _ = os.ReadFile(target)
	if !strings.Contains(string(data), "new result") {
		t.Fatal("save did not replace approved destination")
	}
}

func TestOutputsDeletedChatLeavesNoLoadingOrCachedRecords(t *testing.T) {
	m := demoWindow(t)
	state := &outputsState{chatID: "chat-gone", loading: true, messages: []*model.Message{outputFixture(1, "text/plain")}, actions: map[string]*outputActionState{}}
	state.sheet = m.present(func(c *ui.Context, s *sheet) { state.view(c) }, nil)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settle(tt)
	if state.loading || len(state.messages) != 0 || !tt.HasText(L("Chat unavailable")) {
		t.Fatal("removed chat retained loading or output records")
	}
}
