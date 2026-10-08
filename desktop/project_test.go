package main

import (
	"encoding/json"
	"errors"
	"os"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// projectBackend answers the projects.* methods as the CLI does, over a few revisions of one group.
type projectBackend struct {
	mu      sync.Mutex
	entries []model.ProjectEntry
	saves   []map[string]any
	next    int
}

func (b *projectBackend) Reconnect() {}

func (b *projectBackend) Request(method string, params any) (json.RawMessage, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	raw, _ := json.Marshal(params)
	var input map[string]any
	_ = json.Unmarshal(raw, &input)
	switch method {
	case "projects.get":
		var listed []model.ProjectEntry
		for _, entry := range b.entries {
			if input["history"] == true || entry.Current {
				listed = append(listed, entry)
			}
		}
		return json.Marshal(map[string]any{"entries": listed, "has_more": false, "conflicts": map[string]any{}})
	case "projects.save":
		b.saves = append(b.saves, input)
		var replaced []string
		for _, id := range input["supersedes"].([]any) {
			replaced = append(replaced, id.(string))
		}
		for _, id := range replaced {
			if !slices.ContainsFunc(b.entries, func(e model.ProjectEntry) bool { return e.ID == id && e.Current }) {
				return nil, errors.New(model.StaleProjectEntry)
			}
		}
		b.next++
		saved := model.ProjectEntry{ID: "ctx-saved-" + string(rune('a'+b.next)), Kind: input["kind"].(string), Title: input["title"].(string), Supersedes: replaced, Current: true, Verification: "agreed", Freshness: "agreed", UpdatedAt: 1_791_447_300}
		b.supersede(replaced, saved)
		return json.Marshal(saved)
	}
	return json.RawMessage(`null`), nil
}

// supersede puts `entry` in place of the revisions it replaces, as a correction from another
// Device does.
func (b *projectBackend) supersede(ids []string, entry model.ProjectEntry) {
	for i := range b.entries {
		if slices.Contains(ids, b.entries[i].ID) {
			b.entries[i].Current = false
		}
	}
	b.entries = append(b.entries, entry)
}

// projectFixture is the main window on the demo's Launch room, over a backend that answers the
// project methods.
func projectFixture(t *testing.T) (*mainWindow, *ui.Tester, *projectBackend, func(func() bool)) {
	t.Helper()
	demoWindow(t)
	demo := store
	backend := &projectBackend{entries: []model.ProjectEntry{
		{ID: "ctx-brief", Kind: "brief", Title: "Relay launch", Text: "Move every Device to the TLS relay.", Source: model.ProjectSource{Kind: "user", Label: "User"}, Verification: "agreed", Freshness: "agreed", UpdatedAt: 1_791_400_000, Current: true},
		{ID: "ctx-link", Kind: "document", Title: "Release checklist", Source: model.ProjectSource{Kind: "url", Label: "docs.example.com", URL: "https://docs.example.com/checklist"}, Verification: "unavailable", Freshness: "unavailable", UpdatedAt: 1_791_400_000, Current: true},
	}}
	queue := make(chan func(), 64)
	store = model.NewStore(backend, func(fn func()) { queue <- fn }, false)
	store.Devices, store.Bots, store.Chats, store.Providers, store.Models = demo.Devices, demo.Bots, demo.Chats, demo.Providers, demo.Models
	store.IsConnected, store.IsStarting, store.HasIdentity = true, false, demo.HasIdentity
	m := newMainWindow()
	m.userWantsInspector = true
	app.main = m
	m.selectChat("chat-relay")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	step := func() {
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
	wait := func(done func() bool) {
		t.Helper()
		deadline := time.Now().Add(4 * time.Second)
		for step(); !done(); step() {
			if time.Now().After(deadline) {
				t.Fatalf("did not settle: %q", tt.Texts())
			}
			time.Sleep(time.Millisecond)
		}
		step()
	}
	wait(func() bool { return tt.HasText("Relay launch") })
	return m, tt, backend, wait
}

func TestTheProjectSectionListsEntriesAndSaysWhatNeedsTheUser(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-relay")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	for _, text := range []string{"PROJECT", "Relay launch", "Brief", "Fact · Suggested by Scout", "Show 2 More"} {
		if !tt.HasText(text) {
			t.Fatalf("no %q in %q", text, tt.Texts())
		}
	}
	if tt.HasText("Release checklist") {
		t.Fatal("past five entries, four show")
	}
	if err := tt.Click("Show 2 More"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText("Release checklist") || !tt.HasText("Link · docs.example.com") || tt.HasText("Show 2 More") {
		t.Fatalf("every entry shows: %q", tt.Texts())
	}
	if err := tt.Click("Relay p95 latency is 180 ms"); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	for _, text := range []string{"Fact", "Suggested by Scout on", "Accept", "Remove…"} {
		if !slices.ContainsFunc(tt.Texts(), func(each string) bool { return strings.HasPrefix(each, text) }) {
			t.Fatalf("no %q in the sheet: %q", text, tt.Texts())
		}
	}

	// A group with nothing in it shows the title and its + alone.
	m.selectChat("chat-launch")
	settle(tt)
	if !tt.HasText("PROJECT") || tt.HasText("Relay launch") {
		t.Fatalf("an empty group's section: %q", tt.Texts())
	}
}

func TestAnEntrySavesACorrectionAndOffersTheVersionAnotherDeviceSaved(t *testing.T) {
	m, tt, backend, wait := projectFixture(t)
	if !tt.HasText("Link · Unavailable") {
		t.Fatalf("a link that could not be opened says so: %q", tt.Texts())
	}
	m.openProjectEntry("chat-relay", "ctx-brief")
	wait(func() bool { return tt.HasText("Saved by you on " + model.ProjectDay(time.Unix(1_791_400_000, 0))) })
	if err := tt.Click("Title"); err != nil {
		t.Fatal(err)
	}
	tt.Type(" on Friday")
	wait(func() bool { return true })
	// Another Device corrects the brief first.
	backend.mu.Lock()
	backend.supersede([]string{"ctx-brief"}, model.ProjectEntry{ID: "ctx-elsewhere", Kind: "brief", Title: "Relay launch, moved", Source: model.ProjectSource{Kind: "user", Label: "User"}, Supersedes: []string{"ctx-brief"}, Current: true, Verification: "agreed", Freshness: "agreed", UpdatedAt: 1_791_447_000})
	backend.mu.Unlock()
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	wait(func() bool { return tt.HasText("This entry changed on another Device") })
	if err := tt.Click("Overwrite with mine"); err != nil {
		t.Fatal(err)
	}
	wait(func() bool { return !tt.HasText("Saved by you on " + model.ProjectDay(time.Unix(1_791_400_000, 0))) })
	backend.mu.Lock()
	defer backend.mu.Unlock()
	if len(backend.saves) != 2 {
		t.Fatalf("saves: %v", backend.saves)
	}
	last := backend.saves[1]
	if !strings.HasSuffix(last["title"].(string), " on Friday") || last["supersedes"].([]any)[0] != "ctx-elsewhere" || last["verification"] != "agreed" {
		t.Fatalf("the edits go over the other Device's version: %v", last)
	}
	if source := last["source"].(map[string]any); source["kind"] != "user" {
		t.Fatalf("a correction keeps its source: %v", source)
	}
}

func TestANewLinkNeedsItsAddress(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-launch")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	st := m.presentProjectEntry("chat-launch", nil, "document", nil)
	st.title = "Pricing page"
	if st.canSave() {
		t.Fatal("a link without its address")
	}
	st.link = "example.com/pricing"
	if st.canSave() || st.whatStopsSaving() != "Links start with https://." {
		t.Fatalf("a link without https: %q", st.whatStopsSaving())
	}
	st.link = "https://example.com/pricing"
	if !st.canSave() || st.confirmTitle() != "Add" {
		t.Fatal("a new link with its address")
	}
}

func TestRenderProject(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-relay")
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	settle(tt)
	// Without LORCA_RENDER the states are only built, which is quick.
	shoot := func(name string) {
		if os.Getenv("LORCA_RENDER") == "" {
			settle(tt)
			return
		}
		renderBoth(t, tt, name)
	}
	shoot("project-inspector")
	if err := tt.Click("Show 2 More"); err != nil {
		t.Fatal(err)
	}
	shoot("project-inspector-all")
	context := m.inspector.project.contexts["chat-relay"]
	entry := func(id string) *model.ProjectEntry {
		for _, each := range context.Entries {
			if each.ID == id {
				return &each
			}
		}
		t.Fatalf("no %s", id)
		return nil
	}
	unavailable := entry("ctx-link")
	unavailable.Freshness, unavailable.Verification, unavailable.RefreshError = "unavailable", "unavailable", "error sending request"
	sheets := []struct {
		name          string
		entry         *model.ProjectEntry
		kind          string
		otherVersions []string
	}{
		{"decision", entry("ctx-decision"), "", nil},
		{"suggestion", entry("ctx-fact"), "", nil},
		{"link", entry("ctx-link"), "", nil},
		{"unavailable", unavailable, "", nil},
		{"file", entry("ctx-file"), "", nil},
		{"conflict", entry("ctx-brief"), "", []string{"ctx-other"}},
		{"new", nil, "decision", nil},
	}
	for _, each := range sheets {
		m.presentProjectEntry("chat-relay", each.entry, each.kind, each.otherVersions)
		shoot("project-sheet-" + each.name)
		m.sheets[len(m.sheets)-1].dismiss()
		settle(tt)
	}
}
