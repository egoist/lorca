package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

type projectUICall struct {
	method string
	params map[string]any
}
type projectUITransport struct {
	mu       sync.Mutex
	entries  []model.ProjectEntry
	revision int
	calls    []projectUICall
	failSave bool
	gate     chan struct{}
}

func (f *projectUITransport) Reconnect() {}
func (f *projectUITransport) Request(method string, params any) (json.RawMessage, error) {
	f.mu.Lock()
	gate := f.gate
	f.mu.Unlock()
	if gate != nil && method == "projects.get" {
		<-gate
	}
	f.mu.Lock()
	defer f.mu.Unlock()
	raw, _ := json.Marshal(params)
	var input map[string]any
	_ = json.Unmarshal(raw, &input)
	f.calls = append(f.calls, projectUICall{method, input})
	switch method {
	case "projects.get":
		var entries []model.ProjectEntry
		for _, entry := range f.entries {
			if input["history"] == true || entry.Current && !entry.Removed {
				entries = append(entries, entry)
			}
		}
		return json.Marshal(model.ProjectContext{ChatID: "chat-launch", Revision: fmt.Sprintf("r%d", f.revision), Entries: entries, Conflicts: map[string][]string{}, MaxContextBytes: 8000})
	case "projects.save":
		if f.failSave {
			return nil, errors.New("Project context changed since you opened it. Reload and apply your correction.")
		}
		supersedes, _ := input["supersedes"].([]any)
		if len(supersedes) > 0 && input["expected_revision"] != fmt.Sprintf("r%d", f.revision) {
			return nil, errors.New("stale revision")
		}
		var entry model.ProjectEntry
		_ = json.Unmarshal(raw, &entry)
		entry.ID, entry.Current, entry.UpdatedAt, entry.Freshness = "ctx-updated", true, 1791447300, entry.Verification
		for _, id := range entry.Supersedes {
			for i := range f.entries {
				if f.entries[i].ID == id {
					f.entries[i].Current = false
				}
			}
		}
		f.entries = append(f.entries, entry)
		f.revision++
		return json.Marshal(entry)
	case "projects.refresh":
		id := input["entry_id"].(string)
		for i := range f.entries {
			if f.entries[i].ID == id {
				entry := f.entries[i]
				f.entries[i].Current = false
				entry.ID, entry.Supersedes = "ctx-refreshed", []string{id}
				entry.Verification, entry.Freshness, entry.RefreshError = "unavailable", "unavailable", "Fixture source unavailable; previous snapshot retained."
				f.entries = append(f.entries, entry)
				f.revision++
				return json.Marshal(entry)
			}
		}
		return nil, errors.New("unknown source")
	case "projects.asset":
		entry := model.ProjectEntry{ID: "ctx-added-asset", Kind: "asset", Title: "Reference fixture", Text: "Reference asset added through the scoped CLI API.", Source: model.ProjectSource{Kind: "user", Label: "Fixture owner"}, Verification: "agreed", Freshness: "agreed", UpdatedAt: 1791447300, Current: true, Asset: &model.WireAttachment{ID: "att-added", Name: "reference.txt", Mime: "text/plain", Size: 40}}
		f.entries = append(f.entries, entry)
		f.revision++
		return json.Marshal(entry)
	case "projects.asset_path":
		return nil, errors.New("Reference asset unavailable: fixture Runner offline. Retry when it is reachable.")
	default:
		return json.RawMessage(`null`), nil
	}
}

func projectUIFixture(t *testing.T) (*mainWindow, *ui.Tester, *projectUITransport, chan func()) {
	t.Helper()
	oldWindow := demoWindow(t)
	old := store
	when := int64(1791447300)
	backend := &projectUITransport{revision: 1, entries: []model.ProjectEntry{
		{ID: "ctx-decision", Kind: "decision", Title: "Pilot release decision", Text: "Release the Harbor pilot on 16 October.\n\nKeep the scope to project briefs and reference assets. Require a successful smoke test before announcing availability.", Source: model.ProjectSource{Kind: "output", Label: "Project owner · fixture", Output: &model.ProjectOutputReference{ChatID: "chat-launch", MessageID: "output-version-2", OutputID: "output-pilot", Version: 2, TaskID: "task-fixture"}}, Verification: "agreed", Freshness: "agreed", UpdatedAt: when, VerifiedAt: &when, Current: true},
		{ID: "ctx-source", Kind: "fact", Title: "Release status reference", Text: "Last retrieved status: awaiting owner review.\nBuild: pilot-2026.10\nThis earlier snapshot remains available while its live source is unreachable.", Source: model.ProjectSource{Kind: "url", Label: "Release status · fixture", URL: "https://example.com/harbor/status"}, Verification: "fetched", Freshness: "stale", UpdatedAt: when - 86400, FetchedAt: &when, Current: true},
		{ID: "ctx-asset", Kind: "asset", Title: "Launch checklist", Text: "Use this project reference when checking the pilot build.", Source: model.ProjectSource{Kind: "user", Label: "Fixture owner"}, Verification: "agreed", Freshness: "agreed", UpdatedAt: when, Current: true, Asset: &model.WireAttachment{ID: "att-checklist", Name: "launch-checklist.pdf", Mime: "application/pdf", Size: 28416}},
		{ID: "ctx-history", Kind: "decision", Title: "Initial rollout plan", Text: "Original plan: release on 9 October.\nSuperseded by the owner's decision to allow time for smoke-test verification.", Source: model.ProjectSource{Kind: "user", Label: "Fixture owner"}, Verification: "agreed", Freshness: "agreed", UpdatedAt: when - 86400, Current: false},
	}}
	queue := make(chan func(), 64)
	store = model.NewStore(backend, func(fn func()) { queue <- fn }, false)
	store.Devices, store.Bots, store.Chats, store.Providers, store.Models = old.Devices, old.Bots, old.Chats, old.Providers, old.Models
	store.Routines, store.IsConnected, store.IsStarting, store.HasIdentity = old.Routines, true, false, old.HasIdentity
	m := newMainWindow()
	m.userWantsInspector = oldWindow.userWantsInspector
	app.main = m
	m.selectChat("chat-launch")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	projectUIStep(tt, queue)
	return m, tt, backend, queue
}
func projectUIStep(tt *ui.Tester, queue chan func()) {
	for {
		select {
		case fn := <-queue:
			fn()
		default:
			tt.Frame()
			return
		}
	}
}
func projectUIWait(t *testing.T, tt *ui.Tester, queue chan func(), done func() bool) {
	t.Helper()
	projectUIStep(tt, queue)
	deadline := time.Now().Add(4 * time.Second)
	for !done() {
		if time.Now().After(deadline) {
			t.Fatalf("project UI did not settle: %q", tt.Texts())
		}
		projectUIStep(tt, queue)
		time.Sleep(time.Millisecond)
	}
	projectUIStep(tt, queue)
}
func projectUIPick(t *testing.T, tt *ui.Tester, queue chan func(), title string) {
	t.Helper()
	if err := tt.Click("Project entry"); err != nil {
		t.Fatal(err)
	}
	projectUIStep(tt, queue)
	var option string
	for _, item := range tt.Menu() {
		if strings.Contains(item, title) {
			option = item
			break
		}
	}
	if option == "" {
		t.Fatalf("no %s in %v", title, tt.Menu())
	}
	if err := tt.ChooseMenuItem(option); err != nil {
		t.Fatal(err)
	}
	projectUIStep(tt, queue)
}

func TestProjectContextNativeEditingAndConflicts(t *testing.T) {
	m, tt, backend, queue := projectUIFixture(t)
	st := m.presentProjectContext("chat-launch")
	projectUIWait(t, tt, queue, func() bool { return !st.loading })
	projectUIPick(t, tt, queue, "Pilot release decision")
	if err := tt.Click("Entry title"); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("Corrected release decision")
	projectUIStep(tt, queue)
	if st.draft.title != "Corrected release decision" {
		t.Fatal("pending input lost", st.draft.title)
	}
	st.contextChanged()
	projectUIStep(tt, queue)
	if st.draft.title != "Corrected release decision" || !st.outdated {
		t.Fatal("context event clobbered draft")
	}
	st.outdated = false
	backend.mu.Lock()
	backend.failSave = true
	backend.mu.Unlock()
	projectUIStep(tt, queue)
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	projectUIWait(t, tt, queue, func() bool { return !st.busy })
	if !st.outdated || st.draft.title != "Corrected release decision" {
		t.Fatal("conflict discarded draft")
	}
	// Explicit correction keeps the CLI hash and immutable output provenance.
	backend.mu.Lock()
	backend.failSave = false
	backend.mu.Unlock()
	st.outdated = false
	projectUIStep(tt, queue)
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	projectUIWait(t, tt, queue, func() bool { return !st.busy && !st.loading && st.selected == "ctx-updated" })
	backend.mu.Lock()
	defer backend.mu.Unlock()
	var saved map[string]any
	for _, call := range backend.calls {
		if call.method == "projects.save" {
			saved = call.params
		}
	}
	if saved["expected_revision"] != "r1" || saved["chat_id"] != "chat-launch" {
		t.Fatal(saved)
	}
	ref := saved["source"].(map[string]any)["output"].(map[string]any)
	if ref["message_id"] != "output-version-2" || ref["version"] != float64(2) {
		t.Fatal("output version lost", ref)
	}
}

func TestProjectContextNativeHistoryRefreshAssetsAndClosedReplies(t *testing.T) {
	m, tt, backend, queue := projectUIFixture(t)
	st := m.presentProjectContext("chat-launch")
	projectUIWait(t, tt, queue, func() bool { return !st.loading })
	projectUIPick(t, tt, queue, "Release status reference")
	if err := tt.Click("Refresh source"); err != nil {
		t.Fatal(err)
	}
	projectUIWait(t, tt, queue, func() bool { return !st.busy && !st.loading && st.selected == "ctx-refreshed" })
	if !tt.HasText("Fixture source unavailable; previous snapshot retained.") || !strings.Contains(st.draft.text, "Last retrieved status") {
		t.Fatal("refresh lost retained material", tt.Texts())
	}
	projectUIPick(t, tt, queue, "Launch checklist")
	if err := tt.Click("Open asset"); err != nil {
		t.Fatal(err)
	}
	projectUIWait(t, tt, queue, func() bool { return !st.busy })
	if st.status == nil || !strings.Contains(st.status.text, "Reference asset unavailable") {
		t.Fatal("no asset retry state")
	}
	st.addAsset("/fixture/reference.txt")
	projectUIWait(t, tt, queue, func() bool { return !st.busy && !st.loading && st.selected == "ctx-added-asset" })
	if st.entry().Asset.Name != "reference.txt" {
		t.Fatal("asset reply missing")
	}
	if err := tt.Click("Show revision history"); err != nil {
		t.Fatal(err)
	}
	projectUIWait(t, tt, queue, func() bool { return !st.loading && st.loadedHistory })
	projectUIPick(t, tt, queue, "Initial rollout plan")
	if st.editable() {
		t.Fatal("historical entry editable")
	}
	backend.mu.Lock()
	before := len(backend.calls)
	backend.mu.Unlock()
	_ = tt.Click("Save")
	projectUIStep(tt, queue)
	backend.mu.Lock()
	after := len(backend.calls)
	backend.mu.Unlock()
	if before != after {
		t.Fatal("history sent a write")
	}
	// A response already in flight cannot repopulate a dismissed sheet.
	gate := make(chan struct{})
	backend.mu.Lock()
	backend.gate = gate
	backend.mu.Unlock()
	st.load(st.selected)
	m.sheets[len(m.sheets)-1].dismiss()
	close(gate)
	select {
	case fn := <-queue:
		fn()
	case <-time.After(3 * time.Second):
		t.Fatal("closed-sheet reply missing")
	}
	projectUIStep(tt, queue)
	if !st.closed {
		t.Fatal("sheet did not close")
	}
}

func TestRenderProjectContextNativeStates(t *testing.T) {
	m, tt, _, queue := projectUIFixture(t)
	st := m.presentProjectContext("chat-launch")
	projectUIWait(t, tt, queue, func() bool { return !st.loading })
	projectUIPick(t, tt, queue, "Pilot release decision")
	settleTransitions(tt)
	renderTo(t, tt, "desktop-project-decision")
	projectUIPick(t, tt, queue, "Release status reference")
	if err := tt.Click("Refresh source"); err != nil {
		t.Fatal(err)
	}
	projectUIWait(t, tt, queue, func() bool { return !st.busy && !st.loading && st.selected == "ctx-refreshed" })
	settleTransitions(tt)
	renderTo(t, tt, "desktop-project-unavailable")
	projectUIPick(t, tt, queue, "Launch checklist")
	settleTransitions(tt)
	renderTo(t, tt, "desktop-project-asset")
	if err := tt.Click("Show revision history"); err != nil {
		t.Fatal(err)
	}
	projectUIWait(t, tt, queue, func() bool { return !st.loading && st.loadedHistory })
	projectUIPick(t, tt, queue, "Initial rollout plan")
	settleTransitions(tt)
	renderTo(t, tt, "desktop-project-history")
}

func TestRenderProjectContextNavigation(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(func(c *ui.Context) {
		applyTheme(c)
		ui.Column(c).Width(320).Padding(16).Children(func() { chat := store.Chat("chat-launch"); m.inspectorGroup(c, chat, store.BotsIn(chat)) })
	}, 352, 300)
	settle(tt)
	name := "desktop-project-navigation-after"
	if os.Getenv("LORCA_PROJECT_NAV_BEFORE") == "1" {
		name = "desktop-project-navigation-before"
		if tt.HasText("Project context") {
			t.Fatal("baseline includes project row")
		}
	} else if !tt.HasText("Project context") {
		t.Fatal("group row missing")
	}
	renderTo(t, tt, name)
}

func TestProjectContextDraftGuardBudgetAndGroupDeletion(t *testing.T) {
	m, tt, backend, queue := projectUIFixture(t)
	st := m.presentProjectContext("chat-launch")
	projectUIWait(t, tt, queue, func() bool { return !st.loading })
	if err := tt.Click("Entry title"); err != nil {
		t.Fatal(err)
	}
	tt.Type("New project goal")
	projectUIStep(tt, queue)
	if err := tt.Click("Context type"); err != nil {
		t.Fatal(err)
	}
	projectUIStep(tt, queue)
	if err := tt.ChooseMenuItem("Goal"); err != nil {
		t.Fatal(err)
	}
	projectUIStep(tt, queue)
	if st.draft.kind != "goal" || st.draft.title != "New project goal" {
		t.Fatal("type switch replaced draft")
	}
	if err := tt.Click("Close"); err != nil {
		t.Fatal(err)
	}
	projectUIStep(tt, queue)
	if !tt.HasText("Discard unsaved project context changes?") {
		t.Fatal("no discard guard")
	}
	if err := tt.Click("Keep Editing"); err != nil {
		t.Fatal(err)
	}
	projectUIStep(tt, queue)
	if st.closed || st.draft.title != "New project goal" {
		t.Fatal("draft did not survive cancellation")
	}
	st.draft.text = strings.Repeat("界", projectEntryMaxBytes/3+1)
	backend.mu.Lock()
	before := len(backend.calls)
	backend.mu.Unlock()
	st.save(false)
	backend.mu.Lock()
	after := len(backend.calls)
	backend.mu.Unlock()
	if st.problem() == "" || before != after {
		t.Fatal("over-budget text sent a write")
	}
	st.draft.text = "A bounded project goal."
	projectUIStep(tt, queue)
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	projectUIWait(t, tt, queue, func() bool { return !st.busy && !st.loading && st.selected == "ctx-updated" })
	if st.entry().Kind != "goal" {
		t.Fatal("new entry did not retain selected kind")
	}
	store.Chats = slices.DeleteFunc(store.Chats, func(chat *model.Chat) bool { return chat.ID == "chat-launch" })
	projectUIStep(tt, queue)
	if st.editable() || !tt.HasText("This group is no longer available.") {
		t.Fatal("deleted group remains editable")
	}
}
