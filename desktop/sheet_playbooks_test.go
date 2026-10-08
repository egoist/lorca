package main

import (
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The fixture transport stands in for the existing CLI APIs, never for production storage.
type playbookUITransport struct {
	sync.Mutex
	record   model.PlaybookRecord
	methods  []string
	params   []map[string]json.RawMessage
	conflict bool
	wait     chan struct{}
}

func (f *playbookUITransport) Reconnect() {}
func (f *playbookUITransport) Request(method string, params any) (json.RawMessage, error) {
	f.Lock()
	data, _ := json.Marshal(params)
	var p map[string]json.RawMessage
	_ = json.Unmarshal(data, &p)
	f.methods = append(f.methods, method)
	f.params = append(f.params, p)
	wait := f.wait
	if wait != nil {
		f.Unlock()
		<-wait
		f.Lock()
	}
	defer f.Unlock()
	var scope model.PlaybookScope
	_ = json.Unmarshal(p["scope"], &scope)
	switch method {
	case "playbooks.list":
		return json.Marshal(map[string]any{"items": []model.PlaybookSummary{f.record.PlaybookSummary}})
	case "playbooks.get":
		return json.Marshal(f.record)
	case "playbooks.draft":
		f.record.Scope, f.record.Status = scope, "draft"
		return json.Marshal(f.record)
	case "playbooks.save":
		if f.conflict {
			return nil, errors.New("Playbook changed since you opened it; reload before saving")
		}
		var content model.PlaybookContent
		_ = json.Unmarshal(p["content"], &content)
		f.record.Content = &content
		f.record.Status = "saved"
		f.record.Revision++
		return json.Marshal(f.record)
	case "playbooks.remove":
		return json.Marshal(map[string]any{"status": "deleted"})
	case "playbooks.export":
		return json.Marshal(map[string]any{"format": "lorca-playbook", "version": 1, "content": f.record.Content, "files": []any{}})
	}
	return nil, fmt.Errorf("Unexpected fixture request %s", method)
}

func (f *playbookUITransport) calls() []string {
	f.Lock()
	defer f.Unlock()
	return slices.Clone(f.methods)
}

func playbookUIRecord() model.PlaybookRecord {
	content := &model.PlaybookContent{Name: "weekly-review", Description: "Review a completed public demo and suggest next steps.",
		Instructions: "1. Read the public progress notes.\n2. Compare the work with the weekly goals.\n3. Include a concrete example before each recommendation.",
		Examples:     "The public demo shipped on Tuesday; accessibility is the next goal.",
		References:   []model.PlaybookResource{{Path: "references/checklist.md", Text: "# Review questions\n- What changed?\n- Which public examples support the recommendations?"}},
		Scripts:      []model.PlaybookResource{{Path: "scripts/example.sh", Text: "#!/bin/sh\n# Saved as text; never run by this sheet.\nprintf '%s\\n' 'Review the public checklist.'"}}}
	chatID := "chat-relay"
	provenance := model.PlaybookProvenance{Kind: "corrections", ChatID: &chatID, MessageIDs: []string{"fixture-one", "fixture-two"}, Note: "Repeated corrections request public examples"}
	return model.PlaybookRecord{PlaybookSummary: model.PlaybookSummary{ID: "playbook-fixture", Scope: model.PlaybookScope{Kind: "bot", ID: "bot-patch"}, Name: content.Name, Description: content.Description, Revision: 2, Hash: "fixture-hash-two", Status: "draft", Path: "playbook://playbook-fixture/SKILL.md"},
		Content: content, Provenance: provenance, Revisions: []model.PlaybookRevision{
			{Revision: 1, Status: "draft", CreatedAt: 100, Content: &model.PlaybookContent{Instructions: "Summarize the public report."}, Provenance: model.PlaybookProvenance{Kind: "workflow", Note: "Completed public workflow", MessageIDs: []string{"fixture-request", "fixture-reply"}}},
			{Revision: 2, Status: "draft", CreatedAt: 200, Content: content, Provenance: provenance}}}
}

type playbookUIHarness struct {
	m       *mainWindow
	tt      *ui.Tester
	rpc     *playbookUITransport
	posts   chan func()
	applied int
}

func playbookTester(t *testing.T) *playbookUIHarness {
	t.Helper()
	m := demoWindow(t)
	f := &playbookUITransport{record: playbookUIRecord()}
	queue := make(chan func(), 128)
	store = model.NewStore(f, func(fn func()) { queue <- fn }, true)
	store.Start()
	m.selectChat("chat-relay")
	h := &playbookUIHarness{m: m, rpc: f, posts: queue}
	h.tt = ui.NewTester(m.frame(m.view), 1180, 820)
	h.settle(t, func() bool { return true })
	return h
}

func (h *playbookUIHarness) settle(t *testing.T, ready func() bool) {
	t.Helper()
	deadline := time.Now().Add(3 * time.Second)
	for {
		for len(h.posts) > 0 {
			(<-h.posts)()
			h.applied++
		}
		h.tt.Frame()
		if ready() {
			for range 3 {
				for len(h.posts) > 0 {
					(<-h.posts)()
					h.applied++
				}
				h.tt.Frame()
			}
			return
		}
		if time.Now().After(deadline) {
			t.Fatal("Reply did not settle", h.tt.Texts())
		}
		time.Sleep(time.Millisecond)
	}
}

func playbookClick(t *testing.T, tt *ui.Tester, text string) {
	t.Helper()
	if err := tt.Click(text); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
}
func playbookType(t *testing.T, tt *ui.Tester, label, value string) {
	t.Helper()
	playbookClick(t, tt, label)
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type(value)
	tt.Frame()
}

func TestPlaybookNativeEditorRetainsFieldsResourcesAndReviewGuard(t *testing.T) {
	h := playbookTester(t)
	record := playbookUIRecord()
	editor := h.m.presentPlaybookEditor(record.Scope, &record, nil)
	h.settle(t, func() bool { return true })
	if !h.tt.HasText("Review Skill Draft") || !h.tt.HasText("Draft · unavailable to the bot until you save") {
		t.Fatal(h.tt.Texts())
	}
	renderBoth(t, h.tt, "desktop-playbook-draft")
	playbookType(t, h.tt, "Skill instructions", "Corrected public procedure")
	playbookClick(t, h.tt, "Examples")
	playbookType(t, h.tt, "Skill examples", "A revised public example")
	playbookClick(t, h.tt, "References")
	playbookType(t, h.tt, "Bundled file text", "Reference edit stays with this file")
	playbookType(t, h.tt, "Relative file name (for example, checklist.md)", "review.md")
	renderBoth(t, h.tt, "desktop-playbook-references")
	playbookClick(t, h.tt, "Scripts")
	playbookType(t, h.tt, "Bundled file text", "printf 'Synthetic example only'")
	renderBoth(t, h.tt, "desktop-playbook-scripts")
	playbookClick(t, h.tt, "Instructions")
	if editor.content.Instructions != "Corrected public procedure" || editor.content.Examples != "A revised public example" {
		t.Fatal(editor.content)
	}
	value := editor.value()
	if value.References[0].Path != "references/review.md" || value.References[0].Text != "Reference edit stays with this file" || value.Scripts[0].Text != "printf 'Synthetic example only'" {
		t.Fatal(value)
	}
	playbookClick(t, h.tt, "History")
	if !strings.Contains(editor.history, "Repeated corrections request public examples") || !strings.Contains(editor.history, "Source messages: fixture-one, fixture-two") || !h.tt.HasText("Revision history") {
		t.Fatal(editor.history, h.tt.Texts())
	}
	beforeHistory := editor.history
	playbookType(t, h.tt, "Revision history", "This must remain read-only")
	if editor.history != beforeHistory {
		t.Fatal("History accepted an edit")
	}
	renderBoth(t, h.tt, "desktop-playbook-history")
	playbookClick(t, h.tt, "Save Skill")
	h.settle(t, func() bool { return !editor.busy })
	if h.m.hasSheet() || !slices.Equal(h.rpc.calls(), []string{"playbooks.save"}) {
		t.Fatal("reviewed save", h.rpc.calls(), h.m.hasSheet())
	}
	h.rpc.Lock()
	defer h.rpc.Unlock()
	p := h.rpc.params[0]
	if string(p["expected_revision"]) != "2" || string(p["expected_hash"]) != `"fixture-hash-two"` {
		t.Fatal(p)
	}
}

func TestPlaybookNativeManagerLoadsMetadataLazilyAndGuardsRemoval(t *testing.T) {
	h := playbookTester(t)
	manager := h.m.presentPlaybooks("chat-patch")
	h.settle(t, func() bool { return !manager.loading })
	if !slices.Equal(h.rpc.calls(), []string{"playbooks.list"}) {
		t.Fatal(h.rpc.calls())
	}
	if !h.tt.HasText("weekly-review · Draft") || !h.tt.HasText("Bot · Developer") {
		t.Fatal(h.tt.Texts())
	}
	renderBoth(t, h.tt, "desktop-playbook-manager")
	playbookClick(t, h.tt, "weekly-review · Draft")
	playbookClick(t, h.tt, "Edit / Review")
	h.settle(t, func() bool { return !manager.busy })
	if !slices.Equal(h.rpc.calls(), []string{"playbooks.list", "playbooks.get"}) {
		t.Fatal(h.rpc.calls())
	}
	playbookClick(t, h.tt, "Cancel")
	playbookClick(t, h.tt, "Remove")
	if !h.tt.HasText("Remove weekly-review?") {
		t.Fatal(h.tt.Texts())
	}
	h.tt.Key(0, ui.KeyEnter)
	h.tt.Frame()
	h.settle(t, func() bool { return !manager.busy && !manager.loading })
	h.rpc.Lock()
	defer h.rpc.Unlock()
	for i, method := range h.rpc.methods {
		if method == "playbooks.remove" {
			if string(h.rpc.params[i]["expected_revision"]) != "2" || string(h.rpc.params[i]["expected_hash"]) != `"fixture-hash-two"` {
				t.Fatal(h.rpc.params[i])
			}
			return
		}
	}
	t.Fatal("No guarded removal request")
}

func TestPlaybookNativeCorrectionSelectionDraftAndExplicitProjectScope(t *testing.T) {
	h := playbookTester(t)
	chat := store.Chat("chat-relay")
	start := time.Unix(100, 0)
	one := &model.Message{ID: "fixture-one", Author: model.You, Body: model.Body{Kind: model.BodyText, Text: "Include a concrete public example."}, CreatedAt: start}
	two := &model.Message{ID: "fixture-two", Author: model.You, Body: model.Body{Kind: model.BodyText, Text: "Again, ground the recommendation in a public example."}, CreatedAt: start.Add(time.Minute)}
	chat.Messages = []*model.Message{one, two}
	capture := h.m.presentPlaybookCapture(chat.ID, two)
	h.settle(t, func() bool { return true })
	playbookClick(t, h.tt, "Draft for Review")
	if !h.tt.HasText("Select 2–20 related user corrections") || len(h.rpc.calls()) != 0 {
		t.Fatal("one correction admitted", h.rpc.calls(), h.tt.Texts())
	}
	renderBoth(t, h.tt, "desktop-playbook-correction-validation")
	playbookClick(t, h.tt, "You: Include a concrete public example.")
	playbookClick(t, h.tt, "Scope")
	if err := h.tt.ChooseMenuItem("Project · Launch room"); err != nil {
		t.Fatal(err, h.tt.Menu())
	}
	h.tt.Frame()
	playbookClick(t, h.tt, "Draft for Review")
	h.settle(t, func() bool { return !capture.busy })
	if !slices.Equal(h.rpc.calls(), []string{"playbooks.draft"}) || !h.tt.HasText("Review Skill Draft") {
		t.Fatal("draft activated silently", h.rpc.calls(), h.tt.Texts())
	}
	h.rpc.Lock()
	defer h.rpc.Unlock()
	p := h.rpc.params[0]
	if string(p["scope"]) != `{"kind":"project","id":"chat-relay"}` || string(p["message_ids"]) != `["fixture-one","fixture-two"]` || string(p["kind"]) != `"corrections"` {
		t.Fatal(p)
	}
}

func TestPlaybookNativeLateReplyDoesNotReopenDismissedSheet(t *testing.T) {
	h := playbookTester(t)
	h.rpc.wait = make(chan struct{})
	manager := h.m.presentPlaybooks("chat-patch")
	h.settle(t, func() bool { return len(h.rpc.calls()) > 0 })
	playbookClick(t, h.tt, "Done")
	if !manager.closed {
		t.Fatal("manager did not close")
	}
	close(h.rpc.wait)
	before := h.applied
	h.settle(t, func() bool { return h.applied > before })
	if h.m.hasSheet() || len(manager.items) > 0 {
		t.Fatal("late metadata reply reached a dismissed sheet")
	}
}

func TestPlaybookNativeResourceAddRemoveUsesStableFileKeys(t *testing.T) {
	h := playbookTester(t)
	record := playbookUIRecord()
	editor := h.m.presentPlaybookEditor(record.Scope, &record, nil)
	h.settle(t, func() bool { return true })
	playbookClick(t, h.tt, "References")
	playbookType(t, h.tt, "Bundled file text", "First edited reference")
	playbookClick(t, h.tt, "Add File")
	playbookType(t, h.tt, "Relative file name (for example, checklist.md)", "second.md")
	playbookType(t, h.tt, "Bundled file text", "Second edited reference")
	playbookClick(t, h.tt, "Bundled file")
	if err := h.tt.ChooseMenuItem("references/checklist.md"); err != nil {
		t.Fatal(err, h.tt.Menu())
	}
	h.tt.Frame()
	if editor.value().References[0].Text != "First edited reference" {
		t.Fatal(editor.value())
	}
	playbookClick(t, h.tt, "Remove File")
	if value := editor.value(); len(value.References) != 1 || value.References[0].Path != "references/second.md" || value.References[0].Text != "Second edited reference" {
		t.Fatal(value)
	}
}

func TestPlaybookNativeTranscriptWorkflowMenuDraftsBeforeReview(t *testing.T) {
	h := playbookTester(t)
	chat := store.Chat("chat-relay")
	start := time.Now()
	request := &model.Message{ID: "fixture-request", Author: model.You, Body: model.Body{Kind: model.BodyText, Text: "Review the public demo and suggest next steps."}, CreatedAt: start}
	reply := &model.Message{ID: "fixture-reply", Author: model.BotAuthor("bot-patch"), Body: model.Body{Kind: model.BodyText, Text: "Completed the public demo review; accessibility is next."}, CreatedAt: start.Add(time.Second)}
	chat.Messages = []*model.Message{request, reply}
	h.m.selectChat(chat.ID)
	h.settle(t, func() bool { return true })
	if err := h.tt.RightClick(reply.Body.Text); err != nil {
		t.Fatal(err, h.tt.Texts())
	}
	if !slices.Contains(h.tt.Menu(), "Save Workflow as Skill…") {
		t.Fatal(h.tt.Menu())
	}
	if err := h.tt.ChooseMenuItem("Save Workflow as Skill…"); err != nil {
		t.Fatal(err)
	}
	h.tt.Frame()
	if !h.tt.HasText("Save Workflow as Skill") {
		t.Fatal(h.tt.Texts())
	}
	renderBoth(t, h.tt, "desktop-playbook-workflow-capture")
	playbookClick(t, h.tt, "Draft for Review")
	h.settle(t, func() bool { return len(h.m.sheets) == 2 })
	if !slices.Equal(h.rpc.calls(), []string{"playbooks.draft"}) || !h.tt.HasText("Review Skill Draft") {
		t.Fatal(h.rpc.calls(), h.tt.Texts())
	}
	h.rpc.Lock()
	defer h.rpc.Unlock()
	p := h.rpc.params[0]
	if string(p["kind"]) != `"workflow"` || string(p["message_ids"]) != `["fixture-request","fixture-reply"]` {
		t.Fatal(p)
	}
}

func TestPlaybookNativeConflictKeepsDraftAndReloadsWithoutBlindOverwrite(t *testing.T) {
	h := playbookTester(t)
	record := playbookUIRecord()
	h.rpc.conflict = true
	editor := h.m.presentPlaybookEditor(record.Scope, &record, nil)
	h.settle(t, func() bool { return true })
	playbookType(t, h.tt, "Skill instructions", "My unsaved correction")
	playbookClick(t, h.tt, "Save Skill")
	h.settle(t, func() bool { return !editor.busy })
	if !h.tt.HasText("This skill changed while you were editing") {
		t.Fatal(h.tt.Texts())
	}
	renderBoth(t, h.tt, "desktop-playbook-conflict")
	h.tt.Key(0, ui.KeyEnter)
	h.tt.Frame() // Keep Draft is the default, not an overwrite.
	if editor.content.Instructions != "My unsaved correction" || !slices.Equal(h.rpc.calls(), []string{"playbooks.save"}) {
		t.Fatal(editor.content, h.rpc.calls())
	}
	playbookClick(t, h.tt, "Save Skill")
	h.settle(t, func() bool { return !editor.busy })
	playbookClick(t, h.tt, "Reload")
	h.settle(t, func() bool { return !editor.busy })
	if editor.content.Instructions != record.Content.Instructions || !slices.Equal(h.rpc.calls(), []string{"playbooks.save", "playbooks.save", "playbooks.get"}) {
		t.Fatal(editor.content, h.rpc.calls())
	}
}
