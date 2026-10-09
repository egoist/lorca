package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// The template sheets against answers in the CLI's shape, over the demo roster. `LORCA_RENDER`
// also saves each state at 2x in the light and dark appearances.

type templateUICall struct {
	method string
	params map[string]any
}

type templateUITransport struct {
	mu      sync.Mutex
	calls   []templateUICall
	handler func(templateUICall) (any, error)
}

func (transport *templateUITransport) Reconnect() {}
func (transport *templateUITransport) Request(method string, params any) (json.RawMessage, error) {
	data, err := json.Marshal(params)
	if err != nil {
		return nil, err
	}
	var payload map[string]any
	_ = json.Unmarshal(data, &payload)
	call := templateUICall{method, payload}
	transport.mu.Lock()
	transport.calls = append(transport.calls, call)
	handler := transport.handler
	transport.mu.Unlock()
	result, err := handler(call)
	if err != nil {
		return nil, err
	}
	return json.Marshal(result)
}

func (transport *templateUITransport) methodCalls(method string) []templateUICall {
	transport.mu.Lock()
	defer transport.mu.Unlock()
	var calls []templateUICall
	for _, call := range transport.calls {
		if call.method == method {
			calls = append(calls, call)
		}
	}
	return calls
}

var templateProfileFixture = map[string]any{
	"name": "Project Manager", "symbol_name": "list.bullet.clipboard.fill", "accent": "indigo",
	"description": "Plans the work and delegates it to the team. Breaks work down, hands it off with message_bot, and summarizes what came back.",
}

var templateBriefFixture = map[string]any{"name": "Morning brief", "schedule": "0 9 * * 1-5",
	"prompt": "Read the recent messages in every chat you are in and post a short brief: what changed and what needs a decision."}

func templateContentsFixture() map[string]any {
	return map[string]any{
		"profile": map[string]any{"id": "profile", "content": templateProfileFixture},
		"memories": []any{
			map[string]any{"id": "memory-launch", "content": "- The launch is on Friday; the go/no-go call is Thursday at 4 PM."},
			map[string]any{"id": "memory-summary", "content": "- Send the weekly summary to team@example.com.", "flags": []string{"email"}},
			map[string]any{"id": "memory-staging", "content": "- Staging deploys use API_KEY=«redacted credential».", "flags": []string{"credential"}},
			map[string]any{"id": "topic-release", "content": "# Release process\nCut the release branch on Thursday. Tag it after the checklist passes and the notes are approved."},
		},
		"routines": []any{
			map[string]any{"id": "rt-brief", "content": templateBriefFixture},
			map[string]any{"id": "rt-checklist", "content": map[string]any{"name": "Launch checklist", "schedule": "every 2h", "prompt": "Review the launch checklist and report new blockers, or PASS when nothing changed."}},
		},
		"requirements": []any{map[string]any{"service_id": "github", "name": "GitHub"}, map[string]any{"service_id": "linear", "name": "Linear"}},
	}
}

// templateImportFixture is a file that uses GitHub and Linear, read for a Runner whose Linear is
// ready or not.
func templateImportFixture(call templateUICall, linearReady bool) map[string]any {
	path, _ := call.params["path"].(string)
	if strings.Contains(path, "future") {
		return map[string]any{"digest": "future", "can_import": false, "issues": []string{"Unsupported template version 2; this Lorca reads version 1."}, "requirements": []any{}}
	}
	linear := "needs_auth"
	if linearReady {
		linear = "ready"
	}
	return map[string]any{
		"digest": "digest-1", "can_import": linearReady, "issues": []string{},
		"template": map[string]any{"profile": templateProfileFixture,
			"memories": []string{"- The launch is on Friday; the go/no-go call is Thursday at 4 PM."},
			"routines": []any{map[string]any{"name": "Morning brief", "schedule": "0 9 * * 1-5", "schedule_text": "Weekdays at 9:00 AM", "prompt": templateBriefFixture["prompt"]}}},
		"requirements": []any{
			map[string]any{"service_id": "github", "name": "GitHub", "selected": "github", "candidates": []any{map[string]any{"id": "github", "name": "GitHub", "state": "ready"}}},
			map[string]any{"service_id": "linear", "name": "Linear", "selected": "linear", "candidates": []any{map[string]any{"id": "linear", "name": "Linear", "state": linear}}},
		},
	}
}

type templateUIFixture struct {
	m         *mainWindow
	tt        *ui.Tester
	transport *templateUITransport
	queue     chan func()
	// linearReady is the Runner's Linear sign-in, as the import preview reads it.
	linearReady atomic.Bool
	// copied is what Copy Link put on the clipboard, which tests keep apart from the user's.
	copied string
}

func newTemplateUIFixture(t *testing.T) *templateUIFixture {
	t.Helper()
	m := demoWindow(t)
	previous := store
	f := &templateUIFixture{m: m, queue: make(chan func(), 128)}
	f.transport = &templateUITransport{handler: func(call templateUICall) (any, error) {
		switch call.method {
		case "templates.contents":
			return templateContentsFixture(), nil
		case "templates.export.preview":
			return map[string]any{"digest": "export-digest", "template": map[string]any{}}, nil
		case "templates.export":
			return map[string]any{"path": call.params["path"]}, nil
		case "templates.share":
			id := "yWUtxVvEA7X0edjxRUKd2A"
			if linkID, ok := call.params["link_id"].(string); ok {
				id = linkID
			}
			return map[string]any{"link": map[string]any{"id": id, "url": "https://lorca.app/t/" + id + "#-xMo1zlzI63iQ_tY2ATkxlkle7thq-OfylCF-jFDqYw", "bot_id": call.params["bot_id"], "name": "Project Manager", "selection": call.params["selection"], "updated_at": 1}}, nil
		case "templates.import.preview":
			return templateImportFixture(call, f.linearReady.Load()), nil
		case "templates.import":
			return map[string]any{"bot": map[string]any{"id": "bot-imported", "name": call.params["name"], "description": "Plans the work.", "symbol_name": "list.bullet.clipboard.fill", "accent": "indigo", "provider": call.params["provider"], "runner_id": call.params["runner_id"], "created_at": 1}, "chat_id": "dm-imported"}, nil
		case "chats.mark_read", "bots.memory":
			return map[string]any{}, nil
		}
		return nil, errors.New("unexpected " + call.method)
	}}
	store = model.NewStore(f.transport, func(fn func()) { f.queue <- fn }, false)
	store.Devices, store.Bots, store.Chats, store.Routines = previous.Devices, previous.Bots, previous.Chats, previous.Routines
	store.Providers, store.Models = previous.Providers, previous.Models
	store.HasIdentity, store.IsConnected, store.IsStarting = previous.HasIdentity, true, false
	store.SharedLinks = previous.SharedLinks
	oldCopy := copyText
	t.Cleanup(func() { copyText = oldCopy })
	copyText = func(text string) { f.copied = text }
	f.tt = ui.NewTester(m.frame(m.view), 1180, 900)
	if os.Getenv("LORCA_RENDER") != "" {
		f.tt.SetScale(2)
	}
	f.step()
	return f
}

func (f *templateUIFixture) step() {
	for {
		select {
		case fn := <-f.queue:
			fn()
		default:
			runPosts()
			f.tt.Frame()
			return
		}
	}
}

func (f *templateUIFixture) wait(t *testing.T, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for !condition() {
		if time.Now().After(deadline) {
			t.Fatalf("never got there: %q", f.tt.Texts())
		}
		f.step()
		time.Sleep(5 * time.Millisecond)
	}
	// A sheet slides in on the clock; clicks go where it comes to rest.
	settleTransitions(f.tt)
	f.step()
}

func (f *templateUIFixture) click(t *testing.T, label string) {
	t.Helper()
	if err := f.tt.Click(label); err != nil {
		t.Fatal(err)
	}
	f.step()
}

// scrollList scrolls the export's cards to their end, where the memories are.
func (f *templateUIFixture) scrollList(t *testing.T) {
	t.Helper()
	list, ok := f.tt.Find("Launch checklist")
	if !ok {
		t.Fatal("no list to scroll")
	}
	for range 12 {
		f.tt.Scroll(list.X, list.Y, 0, 40)
		f.step()
	}
}

func (f *templateUIFixture) stubDestination(t *testing.T, path string) {
	old := chooseTemplateDestination
	t.Cleanup(func() { chooseTemplateDestination = old })
	chooseTemplateDestination = func(_ *mygo.Window, _ string, done func(string, error)) { done(path, nil) }
}

func TestTemplateExportSendsThePickedContentInTheBotsOrder(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.m.presentTemplateShare("bot-nova")
	f.wait(t, func() bool { return f.tt.HasText("Launch checklist") })
	renderBoth(t, f.tt, "desktop-template-share")
	if !f.tt.HasText("Share Link") || !f.tt.HasText("Save as File…") {
		t.Fatal("Share Link is not the sheet's button")
	}

	if f.tt.HasText("Select All") {
		t.Fatal("a short list offers Select All")
	}
	// Picked out of order, listed in the bot's: routines, plugins, then memories.
	f.click(t, "GitHub")
	f.click(t, "Morning brief")
	f.scrollList(t)
	f.click(t, "Release process")
	f.click(t, "The launch is on Friday; the go/no-go call is Thursday at 4 PM.")
	path := filepath.Join(t.TempDir(), "Project Manager.lorca-template")
	f.stubDestination(t, path)
	f.click(t, "Save as File…")
	f.wait(t, func() bool { return !f.m.hasSheet() })
	previews, saves := f.transport.methodCalls("templates.export.preview"), f.transport.methodCalls("templates.export")
	if len(previews) != 1 || len(saves) != 1 {
		t.Fatalf("calls: %d previews, %d saves", len(previews), len(saves))
	}
	selection, _ := json.Marshal(saves[0].params["selection"])
	if string(selection) != `{"memory_ids":["memory-launch","topic-release"],"profile":true,"requirement_ids":["github"],"routine_ids":["rt-brief"]}` {
		t.Fatalf("selection %s", selection)
	}
	if saves[0].params["path"] != path || saves[0].params["expected_digest"] != "export-digest" || saves[0].params["reviewed"] != true || saves[0].params["overwrite"] != false {
		t.Fatalf("save %v", saves[0].params)
	}
}

func TestTemplateExportOffersSelectAllOnALongList(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.transport.mu.Lock()
	next := f.transport.handler
	f.transport.handler = func(call templateUICall) (any, error) {
		if call.method != "templates.contents" {
			return next(call)
		}
		contents := templateContentsFixture()
		memories := contents["memories"].([]any)
		for _, text := range []string{"- Release notes go out on Monday.", "- The design review is on Tuesdays."} {
			memories = append(memories, map[string]any{"id": "memory-" + text[2:9], "content": text})
		}
		contents["memories"] = memories
		return contents, nil
	}
	f.transport.mu.Unlock()
	f.m.presentTemplateShare("bot-nova")
	f.wait(t, func() bool { return f.tt.HasText("Launch checklist") })
	f.scrollList(t)
	f.click(t, "Select All")
	f.click(t, "Share Link")
	f.wait(t, func() bool { return len(f.transport.methodCalls("templates.export.preview")) == 1 })
	selection := f.transport.methodCalls("templates.export.preview")[0].params["selection"].(map[string]any)
	if ids, _ := selection["memory_ids"].([]any); len(ids) != 6 {
		t.Fatalf("Select All picked %v", selection["memory_ids"])
	}
}

func TestTemplateExportShowsWhyItCannotSave(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.transport.mu.Lock()
	next := f.transport.handler
	f.transport.handler = func(call templateUICall) (any, error) {
		if call.method == "templates.export.preview" {
			return nil, errors.New("What you picked uses GitHub. Check it under Plugins too.")
		}
		return next(call)
	}
	f.transport.mu.Unlock()
	f.m.presentTemplateShare("bot-nova")
	f.wait(t, func() bool { return f.tt.HasText("Launch checklist") })
	f.click(t, "Morning brief")
	f.stubDestination(t, filepath.Join(t.TempDir(), "never.lorca-template"))
	f.click(t, "Share Link")
	f.wait(t, func() bool { return f.tt.HasText("What you picked uses GitHub. Check it under Plugins too.") })
	renderBoth(t, f.tt, "desktop-template-share-error")
	if !f.m.hasSheet() || len(f.transport.methodCalls("templates.export")) > 0 {
		t.Fatal("a refused export saved or closed the sheet")
	}
	// Picking again clears the error.
	f.click(t, "GitHub")
	if f.tt.HasText("What you picked uses GitHub. Check it under Plugins too.") {
		t.Fatal("the error outlived the change")
	}
}

func TestTemplateImportUsesTheRunnersConnectionAndOpensTheNewChat(t *testing.T) {
	f := newTemplateUIFixture(t)
	var opened string
	f.m.presentTemplateImport("", "/fixture/Project Manager.lorca-template", func(id string) { opened = id; f.m.selectChat(id) })
	f.wait(t, func() bool { return f.tt.HasText("Finish setting up Linear on Workbench first.") })
	renderBoth(t, f.tt, "desktop-template-import-setup")
	if calls := f.transport.methodCalls("templates.import.preview"); calls[0].params["runner_id"] != "dev-workbench" {
		t.Fatalf("not this computer first: %v", calls[0].params)
	}
	f.click(t, "Create Bot")
	if len(f.transport.methodCalls("templates.import")) > 0 {
		t.Fatal("imported with a plugin that isn't ready")
	}

	// Signing in to Linear on the Runner previews again by itself.
	f.linearReady.Store(true)
	for i := range store.Devices[0].Plugins {
		if store.Devices[0].Plugins[i].ID == "linear" {
			store.Devices[0].Plugins[i].State = model.PluginReady
		}
	}
	f.wait(t, func() bool { return f.tt.HasText("Routines start paused.") })
	renderBoth(t, f.tt, "desktop-template-import")
	// The Name field is the line above Runner, as far as Provider is below it.
	runner, _ := f.tt.Find("Runner")
	provider, _ := f.tt.Find("Provider")
	f.tt.ClickAt(runner.X+200, runner.Y+runner.H/2-(provider.Y-runner.Y))
	f.step()
	f.tt.Key(ui.Cmd, ui.KeyA)
	f.tt.Type("Launch Manager")
	f.step()
	f.click(t, "Create Bot")
	f.wait(t, func() bool { return opened != "" })
	imports := f.transport.methodCalls("templates.import")
	mappings, _ := imports[0].params["mappings"].(map[string]any)
	if len(imports) != 1 || imports[0].params["name"] != "Launch Manager" || mappings["github"] != "github" || mappings["linear"] != "linear" ||
		imports[0].params["expected_digest"] != "digest-1" || imports[0].params["reviewed"] != true || imports[0].params["runner_id"] != "dev-workbench" {
		t.Fatalf("import %v", imports)
	}
	if opened != "dm-imported" || store.Chat(opened) == nil || store.Bot("bot-imported") == nil {
		t.Fatal("the new chat did not open")
	}
}

func TestTemplateImportExplainsAFileItCannotRead(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.m.presentTemplateImport("", "/fixture/future.lorca-template", nil)
	f.wait(t, func() bool { return f.tt.HasText("Unsupported template version 2; this Lorca reads version 1.") })
	f.click(t, "Create Bot")
	if len(f.transport.methodCalls("templates.import")) > 0 {
		t.Fatal("an unreadable file imported")
	}
	if title := commandByID("importBotTemplate").title(); title != "New Bot from Template…" {
		t.Fatalf("menu says %q", title)
	}
}

func TestTemplateExportIsForTheDirectChatShowing(t *testing.T) {
	f := newTemplateUIFixture(t)
	export := commandByID("shareBotTemplate")
	f.m.selectChat("chat-launch")
	f.step()
	if export.isEnabled() {
		t.Fatal("a group chat exports")
	}
	for _, chat := range store.Chats {
		if chat.IsDM() && slices.Contains(chat.BotIDs, "bot-nova") {
			f.m.selectChat(chat.ID)
		}
	}
	f.step()
	if !export.isEnabled() {
		t.Fatal("a direct chat doesn't export")
	}
	runCommand("shareBotTemplate")
	f.wait(t, func() bool { return f.tt.HasText("Share “Project Manager”") })
}

func TestTemplateSupersededRepliesAndDismissedSheets(t *testing.T) {
	f := newTemplateUIFixture(t)
	entered, release, returned := make(chan struct{}), make(chan struct{}), make(chan struct{})
	f.transport.mu.Lock()
	next := f.transport.handler
	f.transport.handler = func(call templateUICall) (any, error) {
		// The preview for Studio comes back late, after the one for Workbench again.
		if call.method == "templates.import.preview" && call.params["runner_id"] == "dev-studio" {
			close(entered)
			<-release
			defer close(returned)
			return map[string]any{"digest": "old", "can_import": false, "issues": []string{"An old answer."}}, nil
		}
		return next(call)
	}
	f.transport.mu.Unlock()
	f.m.presentTemplateImport("", "/fixture/Project Manager.lorca-template", nil)
	f.wait(t, func() bool { return f.tt.HasText("Finish setting up Linear on Workbench first.") })
	f.click(t, "Workbench (this computer)")
	if err := f.tt.ChooseMenuItem("Studio"); err != nil {
		t.Fatal(err)
	}
	f.step()
	select {
	case <-entered:
	case <-time.After(time.Second):
		t.Fatal("no preview for Studio")
	}
	// The Runner pop-up, right of its label: a Studio row behind the sheet shows Studio too.
	runner, _ := f.tt.Find("Runner")
	f.tt.ClickAt(runner.X+200, runner.Y+runner.H/2)
	f.step()
	if err := f.tt.ChooseMenuItem("Workbench (this computer)"); err != nil {
		t.Fatal(err)
	}
	f.step()
	f.wait(t, func() bool { return f.tt.HasText("Finish setting up Linear on Workbench first.") })
	close(release)
	<-returned
	for range 4 {
		f.step()
		time.Sleep(5 * time.Millisecond)
	}
	if f.tt.HasText("An old answer.") {
		t.Fatal("an old answer replaced the new one")
	}
	f.click(t, "Cancel")
	f.m.presentTemplateShare("bot-nova")
	f.step()
	f.click(t, "Cancel")
	for range 4 {
		f.step()
		time.Sleep(5 * time.Millisecond)
	}
	if f.m.hasSheet() {
		t.Fatal("late contents brought back a closed sheet")
	}
}

func TestTemplateExportAsksBeforeTheExtensionReplacesAFile(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.m.presentTemplateShare("bot-nova")
	f.wait(t, func() bool { return f.tt.HasText("Launch checklist") })
	path := filepath.Join(t.TempDir(), "brief")
	if err := os.WriteFile(path+".lorca-template", []byte("original"), 0o600); err != nil {
		t.Fatal(err)
	}
	f.stubDestination(t, path)
	f.click(t, "Save as File…")
	f.wait(t, func() bool {
		return f.tt.HasText("“brief.lorca-template” already exists. Do you want to replace it?")
	})
	if len(f.transport.methodCalls("templates.export")) > 0 {
		t.Fatal("replaced without asking")
	}
	f.click(t, "Replace")
	f.wait(t, func() bool { return !f.m.hasSheet() })
	call := f.transport.methodCalls("templates.export")[0]
	if call.params["path"] != path+".lorca-template" || call.params["overwrite"] != true {
		t.Fatalf("save %v", call.params)
	}
}

func TestTemplateFilenameAndExtensionSafety(t *testing.T) {
	for _, name := range []string{"../../private", "C:\\Credentials\r\n", "CON", "aux.txt", "LPT1", ""} {
		file := templateFilename(name)
		if strings.ContainsAny(file, "/\\\r\n") || !strings.HasSuffix(file, ".lorca-template") {
			t.Fatalf("unsafe suggested file %q", file)
		}
	}
	if templateFilename("CON") != "bot-CON.lorca-template" {
		t.Fatal("Windows reserved name not guarded")
	}
	path := filepath.Join(t.TempDir(), "chosen.lorca-template")
	if err := os.WriteFile(path, []byte("existing"), 0o600); err != nil {
		t.Fatal(err)
	}
	if _, exists, extra := templateDestination(path); !exists || extra {
		t.Fatal("a path the dialog confirmed asks again")
	}
	if actual, exists, extra := templateDestination(strings.TrimSuffix(path, ".lorca-template")); actual != path || !exists || !extra {
		t.Fatal("a replacement the extension caused needs asking")
	}
}

func TestTemplateShareLinkShowsTheLinkCopied(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.m.presentTemplateShare("bot-nova")
	f.wait(t, func() bool { return f.tt.HasText("Launch checklist") })
	f.click(t, "Morning brief")
	f.click(t, "Share Link")
	f.wait(t, func() bool { return f.tt.HasText("Done") })
	shares := f.transport.methodCalls("templates.share")
	if len(shares) != 1 || shares[0].params["expected_digest"] != "export-digest" || shares[0].params["reviewed"] != true || shares[0].params["link_id"] != nil {
		t.Fatalf("share %v", shares)
	}
	url := "https://lorca.app/t/yWUtxVvEA7X0edjxRUKd2A#-xMo1zlzI63iQ_tY2ATkxlkle7thq-OfylCF-jFDqYw"
	if f.copied != url || !f.tt.HasText(url) || f.tt.HasText("Cancel") {
		t.Fatalf("the link isn't shown copied: %q", f.copied)
	}
	renderBoth(t, f.tt, "desktop-template-shared")
	f.click(t, "Done")
	if f.m.hasSheet() {
		t.Fatal("Done left the sheet up")
	}
}

func TestTemplateUpdateKeepsTheLinkAndStartsFromWhatItHolds(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.transport.mu.Lock()
	next := f.transport.handler
	f.transport.handler = func(call templateUICall) (any, error) {
		if call.method != "templates.contents" {
			return next(call)
		}
		return map[string]any{
			"profile": map[string]any{"id": "profile", "content": map[string]any{"name": "Writer", "symbol_name": "pencil.and.scribble", "accent": "pink",
				"description": "Writes docs, copy, and release notes in plain language: short sentences, no filler, and no exclamation marks."}},
			"memories": []any{map[string]any{"id": "memory-voice", "content": "- Plain words, short sentences, no exclamation marks."},
				map[string]any{"id": "memory-launch", "content": "- The launch post goes out Friday morning."}},
			"requirements": []any{map[string]any{"service_id": "notion", "name": "Notion"}},
		}, nil
	}
	f.transport.mu.Unlock()
	link := store.SharedLinkFor("bot-quill")
	if link == nil {
		t.Fatal("no demo link")
	}
	f.m.presentTemplateShare("bot-quill")
	f.wait(t, func() bool {
		return f.tt.HasText("Update Link") && f.tt.HasText("The launch post goes out Friday morning.")
	})
	if !f.tt.HasText(link.URL) {
		t.Fatal("the sheet doesn't show the link")
	}
	renderBoth(t, f.tt, "desktop-template-update")
	f.click(t, "Update Link")
	f.wait(t, func() bool { return f.tt.HasText("Done") })
	share := f.transport.methodCalls("templates.share")[0]
	selection, _ := json.Marshal(share.params["selection"])
	if share.params["link_id"] != link.ID || string(selection) != `{"memory_ids":["memory-voice"],"profile":true}` {
		t.Fatalf("update %v", share.params)
	}
}

func TestTemplateSharedLinksListTheAccountsLinks(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.m.showSettings(model.PaneSharedLinks)
	f.wait(t, func() bool { return f.tt.HasText("Writer") })
	renderBoth(t, f.tt, "desktop-template-shared-links")
	if err := f.tt.RightClick("Writer"); err != nil {
		t.Fatal(err)
	}
	if menu := f.tt.Menu(); !slices.Equal(menu, []string{"Copy Link", "-", "Revoke Link…"}) {
		t.Fatalf("menu %q", menu)
	}
	if err := f.tt.ChooseMenuItem("Copy Link"); err != nil {
		t.Fatal(err)
	}
	f.step()
	if f.copied != store.SharedLinks[0].URL {
		t.Fatalf("copied %q", f.copied)
	}
	if err := f.tt.RightClick("Writer"); err != nil {
		t.Fatal(err)
	}
	if err := f.tt.ChooseMenuItem("Revoke Link…"); err != nil {
		t.Fatal(err)
	}
	f.step()
	f.wait(t, func() bool { return f.tt.HasText("Revoke the link to “Writer”?") })
	f.click(t, "Revoke")
	f.wait(t, func() bool { return len(f.transport.methodCalls("templates.unshare")) == 1 })
	if f.transport.methodCalls("templates.unshare")[0].params["link_id"] != "mock-link" {
		t.Fatal("revoked another link")
	}
}

func TestTemplateImportOpensAPastedLink(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.linearReady.Store(true)
	f.m.presentTemplateImport("", "", nil)
	f.wait(t, func() bool { return f.tt.HasText("Choose File…") })
	if f.tt.HasText("Provider") || len(f.transport.methodCalls("templates.import.preview")) > 0 {
		t.Fatal("the form shows before a template opens")
	}
	renderBoth(t, f.tt, "desktop-template-import-empty")
	link := "https://lorca.app/t/yWUtxVvEA7X0edjxRUKd2A#-xMo1zlzI63iQ_tY2ATkxlkle7thq-OfylCF-jFDqYw"
	// The From field is left of Choose File….
	choose, _ := f.tt.Find("Choose File…")
	f.tt.ClickAt(choose.X-100, choose.Y+choose.H/2)
	f.step()
	f.tt.Type(link)
	f.step()
	f.wait(t, func() bool { return f.tt.HasText("Routines start paused.") })
	preview := f.transport.methodCalls("templates.import.preview")[0]
	if preview.params["link"] != link || preview.params["path"] != nil {
		t.Fatalf("preview %v", preview.params)
	}
	renderBoth(t, f.tt, "desktop-template-import-link")
}
