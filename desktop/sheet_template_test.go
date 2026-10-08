package main

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

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
	transport.mu.Unlock()
	result, err := transport.handler(call)
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

func templateUIFixtureDocument() map[string]any {
	return map[string]any{"format": "lorca.bot-template", "version": 1, "profile": map[string]any{"name": "Release Reviewer", "description": "Review pull requests and report risks and checks.", "symbol_name": "checklist", "accent": "blue"}, "memories": []string{"Use concise reviews; contact review@example.test."}, "routines": []any{map[string]any{"name": "Morning pull-request scan", "schedule": "every 2h", "prompt": "Summarize open pull requests and flag missing checks."}}, "requirements": []any{map[string]any{"service_id": "github"}}}
}

func templateUIFixtureReply(call templateUICall) (any, error) {
	doc := templateUIFixtureDocument()
	warnings := []any{map[string]any{"path": "memories.0", "message": "Selected memory may contain personal information. Contains an email address."}}
	switch call.method {
	case "templates.contents":
		return map[string]any{"profile": doc["profile"], "skills": []any{}, "memories": []any{map[string]any{"id": "fixture-memory", "content": "Use concise reviews; contact review@example.test."}}, "routines": []any{map[string]any{"id": "fixture-routine", "content": doc["routines"].([]any)[0]}}, "requirements": doc["requirements"], "notes": []string{"Reusable skills need playbook support in this CLI. Update Lorca before exporting or importing skills."}}, nil
	case "templates.export.preview":
		return map[string]any{"template": doc, "digest": "fixture-export-digest", "visibility": "private_file", "warnings": warnings}, nil
	case "templates.export":
		return map[string]any{"path": call.params["path"], "digest": "fixture-export-digest", "visibility": "private_file"}, nil
	case "templates.import.preview":
		path, _ := call.params["path"].(string)
		if strings.Contains(path, "future") {
			return map[string]any{"digest": "future-file", "can_import": false, "issues": []string{"Unsupported template version 2; this Lorca reads version 1."}}, nil
		}
		if strings.Contains(path, "skills") {
			doc["requirements"] = []any{}
			doc["skills"] = []any{map[string]any{"name": "review-checklist", "description": "Review a proposed change", "instructions": "Read the diff, identify risks and report tests."}}
			return map[string]any{"template": doc, "digest": "skill-file", "can_import": false, "warnings": warnings, "issues": []string{"Reusable skills need playbook support in this CLI. Update Lorca before exporting or importing skills."}}, nil
		}
		mappings, _ := call.params["mappings"].(map[string]any)
		ready := mappings["github"] == "github"
		issues := []string{}
		if !ready {
			issues = append(issues, "Select your own connection for github.")
		}
		return map[string]any{"template": doc, "digest": "fixture-import-digest", "can_import": ready, "issues": issues, "warnings": warnings, "requirements": []any{map[string]any{"service_id": "github", "selected": mappings["github"], "candidates": []any{map[string]any{"id": "github", "name": "GitHub · Demo workspace", "state": "ready", "detail": "Ready"}}}}}, nil
	case "templates.import":
		return map[string]any{"bot": map[string]any{"id": "bot-imported-fixture", "name": call.params["name"], "description": "Review pull requests.", "symbol_name": "checklist", "accent": "blue", "provider": call.params["provider"], "runner_id": call.params["runner_id"], "created_at": 1}, "chat_id": "dm-imported-fixture", "routines_paused": true}, nil
	case "bots.memory":
		return map[string]any{"bot_id": "bot-patch", "here": true, "runner": "Workbench", "path": "/fixture/workspace", "text": "Fixture memory", "hash": "fixture-hash", "max_lines": 200, "max_bytes": 24000}, nil
	case "chats.mark_read":
		return map[string]any{}, nil
	default:
		return nil, errors.New("Unexpected fixture method " + call.method)
	}
}

type templateUIFixture struct {
	m         *mainWindow
	tt        *ui.Tester
	transport *templateUITransport
	queue     chan func()
}

func newTemplateUIFixture(t *testing.T) *templateUIFixture {
	t.Helper()
	m := demoWindow(t)
	previous := store
	transport := &templateUITransport{handler: templateUIFixtureReply}
	queue := make(chan func(), 128)
	store = model.NewStore(transport, func(fn func()) { queue <- fn }, false)
	store.Devices, store.Bots, store.Chats, store.Routines = previous.Devices, previous.Bots, previous.Chats, previous.Routines
	store.Providers, store.Models = previous.Providers, previous.Models
	store.HasIdentity, store.IsConnected, store.IsStarting = previous.HasIdentity, true, false
	if bot := store.Bot("bot-patch"); bot != nil {
		bot.Name = "Release Reviewer"
	}
	f := &templateUIFixture{m: m, transport: transport, queue: queue}
	f.tt = ui.NewTester(m.frame(m.view), 1180, 900)
	f.step()
	return f
}
func (fixture *templateUIFixture) step() {
	for {
		select {
		case fn := <-fixture.queue:
			fn()
		default:
			runPosts()
			fixture.tt.Frame()
			return
		}
	}
}
func (fixture *templateUIFixture) wait(t *testing.T, condition func() bool) {
	t.Helper()
	deadline := time.Now().Add(2 * time.Second)
	for !condition() {
		if time.Now().After(deadline) {
			t.Fatalf("fixture timeout: %q", fixture.tt.Texts())
		}
		fixture.step()
		time.Sleep(5 * time.Millisecond)
	}
	fixture.step()
}
func (fixture *templateUIFixture) click(t *testing.T, label string) {
	t.Helper()
	if err := fixture.tt.Click(label); err != nil {
		t.Fatal(err)
	}
	fixture.step()
}

func TestTemplateDesktopExportSelectionReviewAndPrivateSave(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.m.presentTemplateExport("bot-patch")
	f.wait(t, func() bool { return f.tt.HasText("Profile and instructions: Release Reviewer") })
	renderBoth(t, f.tt, "desktop-template-export-selection")
	for _, label := range []string{"Profile and instructions: Release Reviewer", "Use concise reviews; contact review@example.test.", "Morning pull-request scan", "github"} {
		f.click(t, label)
	}
	f.click(t, "Preview Contents")
	f.wait(t, func() bool { return f.tt.HasText("Save Private File…") })
	if !f.tt.HasText("Review before sharing") {
		t.Fatal("review warnings absent")
	}
	calls := f.transport.methodCalls("templates.export.preview")
	if len(calls) != 1 {
		t.Fatalf("preview calls %v", calls)
	}
	selection := calls[0].params["selection"].(map[string]any)
	if selection["profile"] != true || selection["memory_ids"].([]any)[0] != "fixture-memory" || selection["routine_ids"].([]any)[0] != "fixture-routine" || selection["requirement_ids"].([]any)[0] != "github" {
		t.Fatalf("wrong explicit selection: %v", selection)
	}
	oldChooser := chooseTemplateDestination
	t.Cleanup(func() { chooseTemplateDestination = oldChooser })
	chooseTemplateDestination = func(_ *mygo.Window, _ string, done func(string, error)) {
		done(filepath.Join(t.TempDir(), "reviewer.lorca-template"), nil)
	}
	f.click(t, "Save Private File…")
	if len(f.transport.methodCalls("templates.export")) > 0 {
		t.Fatal("unreviewed save ran")
	}
	f.click(t, "I reviewed the selected content for personal information.")
	renderBoth(t, f.tt, "desktop-template-export-review")
	f.click(t, "Save Private File…")
	f.wait(t, func() bool { return !f.m.hasSheet() })
	saved := f.transport.methodCalls("templates.export")
	if len(saved) != 1 || saved[0].params["reviewed"] != true || saved[0].params["expected_digest"] != "fixture-export-digest" || saved[0].params["overwrite"] != false {
		t.Fatalf("save authority boundary %v", saved)
	}
}

func TestTemplateDesktopChangingSelectionRequiresFreshReview(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.m.presentTemplateExport("bot-patch")
	f.wait(t, func() bool { return f.tt.HasText("Profile and instructions: Release Reviewer") })
	f.click(t, "Profile and instructions: Release Reviewer")
	f.click(t, "Preview Contents")
	f.wait(t, func() bool { return f.tt.HasText("Save Private File…") })
	f.click(t, "I reviewed the selected content for personal information.")
	f.click(t, "Use concise reviews; contact review@example.test.")
	if f.tt.HasText("Save Private File…") || !f.tt.HasText("Preview Contents") {
		t.Fatal("selection change kept review")
	}
	if len(f.transport.methodCalls("templates.export")) > 0 {
		t.Fatal("selection edit saved automatically")
	}
}

func TestTemplateDesktopImportTypingConnectionsAndIndependentDM(t *testing.T) {
	f := newTemplateUIFixture(t)
	var opened string
	f.m.presentTemplateImportPath("/fixture/reviewer.lorca-template", func(id string) { opened = id; f.m.selectChat(id) })
	f.wait(t, func() bool { return f.tt.HasText("Select your own connection for github.") })
	renderBoth(t, f.tt, "desktop-template-import-needs-connection")
	f.click(t, "New bot name")
	f.tt.Key(ui.Cmd, ui.KeyA)
	f.tt.Type("Independent Reviewer")
	f.step()
	f.wait(t, func() bool {
		calls := f.transport.methodCalls("templates.import.preview")
		return len(calls) > 1 && calls[len(calls)-1].params["name"] == "Independent Reviewer"
	})
	f.click(t, "Choose your connection…")
	if err := f.tt.ChooseMenuItem("GitHub · Demo workspace"); err != nil {
		t.Fatal(err)
	}
	f.step()
	f.wait(t, func() bool {
		return f.tt.HasText("Routines stay paused. Review instructions and scripts before running the new bot.")
	})
	f.click(t, "Create Independent Bot")
	if len(f.transport.methodCalls("templates.import")) > 0 {
		t.Fatal("connection choice bypassed review")
	}
	f.click(t, "I reviewed the contents and selected my own connections.")
	renderBoth(t, f.tt, "desktop-template-import-reviewed")
	f.click(t, "Create Independent Bot")
	f.wait(t, func() bool { return opened != "" })
	if opened != "dm-imported-fixture" || store.Chat(opened) == nil || len(store.Chat(opened).Messages) != 0 {
		t.Fatal("independent DM not opened")
	}
	imports := f.transport.methodCalls("templates.import")
	if len(imports) != 1 || imports[0].params["name"] != "Independent Reviewer" || imports[0].params["mappings"].(map[string]any)["github"] != "github" || imports[0].params["expected_digest"] != "fixture-import-digest" || imports[0].params["reviewed"] != true {
		t.Fatalf("lost recipient choice/review: %v", imports)
	}
}

func TestTemplateDesktopCapabilityBlockersAndMenuEntry(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.m.presentTemplateImportPath("/fixture/skills.lorca-template", nil)
	f.wait(t, func() bool {
		return f.tt.HasText("Reusable skills need playbook support in this CLI. Update Lorca before exporting or importing skills.")
	})
	renderBoth(t, f.tt, "desktop-template-import-missing-capability")
	f.click(t, "Create Independent Bot")
	if len(f.transport.methodCalls("templates.import")) > 0 {
		t.Fatal("missing capability imported")
	}
	f.click(t, "Cancel")
	f.m.presentTemplateImportPath("/fixture/future.lorca-template", nil)
	f.wait(t, func() bool { return f.tt.HasText("Unsupported template version 2; this Lorca reads version 1.") })
	f.click(t, "Create Independent Bot")
	if len(f.transport.methodCalls("templates.import")) > 0 {
		t.Fatal("future format imported")
	}
	if commandByID("importBotTemplate") == nil || commandByID("importBotTemplate").title() != "Import Bot Template…" {
		t.Fatal("menu/palette entry absent")
	}
}

func TestTemplateDesktopSupersededRepliesAndDismissedSheets(t *testing.T) {
	f := newTemplateUIFixture(t)
	entered, release, returned := make(chan struct{}), make(chan struct{}), make(chan struct{})
	f.transport.handler = func(call templateUICall) (any, error) {
		if call.method == "templates.import.preview" && call.params["name"] == nil {
			close(entered)
			<-release
			defer close(returned)
			return map[string]any{"digest": "old", "can_import": false, "issues": []string{"Stale response must not replace the new preview."}}, nil
		}
		return templateUIFixtureReply(call)
	}
	f.m.presentTemplateImportPath("/fixture/reviewer.lorca-template", nil)
	f.step()
	select {
	case <-entered:
	case <-time.After(time.Second):
		t.Fatal("initial request missing")
	}
	f.click(t, "New bot name")
	f.tt.Type("Latest Reviewer")
	f.step()
	f.wait(t, func() bool { return f.tt.HasText("Select your own connection for github.") })
	close(release)
	select {
	case <-returned:
	case <-time.After(time.Second):
		t.Fatal("old reply stuck")
	}
	for range 4 {
		f.step()
		time.Sleep(5 * time.Millisecond)
	}
	if f.tt.HasText("Stale response must not replace the new preview.") {
		t.Fatal("old reply replaced new input")
	}
	calls := f.transport.methodCalls("templates.import.preview")
	if calls[len(calls)-1].params["name"] != "Latest Reviewer" {
		t.Fatal("latest field state lost")
	}
	f.click(t, "Cancel")
	if f.m.hasSheet() {
		t.Fatal("sheet did not close")
	}
	f.m.presentTemplateExport("bot-patch")
	f.step()
	f.click(t, "Cancel")
	for range 4 {
		f.step()
		time.Sleep(5 * time.Millisecond)
	}
	if f.m.hasSheet() {
		t.Fatal("late contents resurrected a dismissed sheet")
	}
}

func TestTemplateDesktopInspectorEntryAndReplacementConfirmation(t *testing.T) {
	f := newTemplateUIFixture(t)
	f.m.selectChat("chat-patch")
	f.wait(t, func() bool { return f.tt.HasText("Template") })
	f.click(t, "Export…")
	f.wait(t, func() bool { return f.tt.HasText("Profile and instructions: Release Reviewer") })
	f.click(t, "Profile and instructions: Release Reviewer")
	f.click(t, "Preview Contents")
	f.wait(t, func() bool { return f.tt.HasText("Save Private File…") })
	f.click(t, "I reviewed the selected content for personal information.")
	path := filepath.Join(t.TempDir(), "reviewer")
	if err := os.WriteFile(path+".lorca-template", []byte("original"), 0o600); err != nil {
		t.Fatal(err)
	}
	old := chooseTemplateDestination
	t.Cleanup(func() { chooseTemplateDestination = old })
	chooseTemplateDestination = func(_ *mygo.Window, _ string, done func(string, error)) { done(path, nil) }
	f.click(t, "Save Private File…")
	if !f.tt.HasText("Replace the existing private template?") || len(f.transport.methodCalls("templates.export")) > 0 {
		t.Fatal("extension changed an existing destination without review")
	}
	f.click(t, "Replace")
	f.wait(t, func() bool { return !f.m.hasSheet() })
	call := f.transport.methodCalls("templates.export")[0]
	if call.params["path"] != path+".lorca-template" || call.params["overwrite"] != true {
		t.Fatal("confirmed destination not used")
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
	_, exists, extra := templateDestination(path)
	if !exists || extra {
		t.Fatal("already-confirmed native path asks again")
	}
	actual, exists, extra := templateDestination(strings.TrimSuffix(path, ".lorca-template"))
	if actual != path || !exists || !extra {
		t.Fatal("extension-induced replacement needs explicit choice")
	}
}
