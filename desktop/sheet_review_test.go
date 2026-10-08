package main

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func reviewFixture(t *testing.T, kind, state string) model.ReviewItem {
	t.Helper()
	payload := `{"kind":"draft","text":"Hi team,\n\nThe sandbox report is ready for review.\nPlease check the Friday rollout checklist."}`
	if kind == "plugin" {
		payload = `{"kind":"plugin","plugin_id":"mail-sandbox","server_name":"main","tool":"send_message","arguments":{"account_id":9223372036854775807,"draft_body":{"subject_line":"Weekly update"},"to":"team@example.test"}}`
	}
	raw := `{"id":"review-native-73","runner_id":"dev-workbench","bot_id":"bot-nova","origin":{"chat_id":"chat-nova"},"target":{"account":"Editorial sandbox","resource":"Weekly update draft"},"rationale":"Review before sending","payload":` + payload + `,"version":1,"revision":1,"preconditions":{"workdir":"/sandbox","files":[{"path":"/sandbox/weekly.md","hash":"fixture"}]},"state":"` + state + `"}`
	var item model.ReviewItem
	if err := json.Unmarshal([]byte(raw), &item); err != nil {
		t.Fatal(err)
	}
	if state == "uncertain" {
		item.Outcome = &model.ReviewOutcome{Summary: "Runner restarted during execution. The action may have completed; inspect the target before creating another proposal."}
	}
	return item
}

func reviewTester(t *testing.T, kind, state string) (*mainWindow, *reviewSheet, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	item := reviewFixture(t, kind, state)
	store.Reviews = []*model.ReviewItem{&item}
	m.selectChat(item.Origin.ChatID)
	st := m.presentReview(item)
	tt := ui.NewTester(m.frame(m.view), 1180, 820)
	settleTransitions(tt)
	return m, st, tt
}

func TestReviewDraftEditGuardAndSavedVersion(t *testing.T) {
	_, st, tt := reviewTester(t, "draft", "pending")
	if !tt.HasText("Version 1 · Needs review") {
		t.Fatal(tt.Texts())
	}
	renderBoth(t, tt, "review-draft")
	if err := tt.Click("Draft"); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("Edited draft for Friday")
	if err := tt.Click("Approve"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText("Save your changes, then review and approve the new version.") || store.Review(st.item.ID).State != "pending" {
		t.Fatal("unsaved edit was approved", tt.Texts())
	}
	for range 3 {
		tt.Frame()
	}
	if st.payload != "Edited draft for Friday" {
		t.Fatal("build passes replaced the draft")
	}
	renderBoth(t, tt, "review-unsaved-guard")
	if err := tt.Click("Save Changes"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if st.item.Version != 2 || st.payload != "Edited draft for Friday" || st.status != "" {
		t.Fatalf("save: %+v", st)
	}
	if err := tt.Click("Approve"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Review(st.item.ID).State != "approved" || !tt.HasText("Version 2 · Approved") {
		t.Fatal("approved wrong version", tt.Texts())
	}
}

func TestReviewCallEditorKeepsNumericIDsAndKeys(t *testing.T) {
	_, st, tt := reviewTester(t, "plugin", "pending")
	if err := tt.Click("Proposed call arguments"); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type(`{"account_id":9223372036854775807,"draft_body":{"subject_line":"Edited"},"unsigned_id":18446744073709551615}`)
	if err := tt.Click("Save Changes"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	for _, token := range []string{`"account_id": 9223372036854775807`, `"unsigned_id": 18446744073709551615`, `"subject_line": "Edited"`} {
		if !strings.Contains(st.payload, token) {
			t.Fatalf("lost %s: %s", token, st.payload)
		}
	}
	renderBoth(t, tt, "review-proposed-call")
}

func TestReviewSyncRequiresReloadAndKeepsLocalText(t *testing.T) {
	_, st, tt := reviewTester(t, "draft", "pending")
	if err := tt.Click("Draft"); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("Local unsaved draft")
	tt.Frame()
	next := store.Review(st.item.ID).Clone()
	next.Version, next.Revision = 2, 2
	next.Payload = model.ReviewPayload{Kind: "draft", Text: "Edited on another Device"}
	store.Reviews[0] = &next
	settle(tt)
	if !tt.HasText("This review changed. Reload it before deciding.") || st.item.Version != 1 || st.payload != "Local unsaved draft" {
		t.Fatal("sync replaced displayed edit", tt.Texts())
	}
	_ = tt.Click("Approve")
	settle(tt)
	if store.Review(st.item.ID).State != "pending" {
		t.Fatal("stale approval ran")
	}
	renderBoth(t, tt, "review-sync-conflict")
	if err := tt.Click("Reload"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if st.item.Version != 2 || st.payload != "Edited on another Device" || !tt.HasText("Edited on another Device") {
		t.Fatal("reload did not load saved version")
	}
}

func TestReviewRejectCancelAndUncertainStates(t *testing.T) {
	for _, action := range []string{"Reject", "Cancel Item"} {
		t.Run(action, func(t *testing.T) {
			_, st, tt := reviewTester(t, "draft", "pending")
			if err := tt.Click(action); err != nil {
				t.Fatal(err)
			}
			settle(tt)
			want := "rejected"
			if action == "Cancel Item" {
				want = "cancelled"
			}
			if st.item.State != want || st.item.Editable() {
				t.Fatal("decision not recorded")
			}
		})
	}
	_, st, tt := reviewTester(t, "plugin", "uncertain")
	if !tt.HasText("Check the outcome") && !tt.HasText("Version 1 · Check the outcome") {
		t.Fatal(tt.Texts())
	}
	_ = tt.Click("Approve")
	settle(tt)
	if st.item.State != "uncertain" {
		t.Fatal("uncertain outcome replayed")
	}
	renderBoth(t, tt, "review-uncertain")
}

func TestInspectorReviewQueueOpensEditor(t *testing.T) {
	m := demoWindow(t)
	item := reviewFixture(t, "draft", "pending")
	store.Reviews = []*model.ReviewItem{&item}
	tt := ui.NewTester(func(c *ui.Context) { applyTheme(c); m.inspectorReviews(c, store.Chat(item.Origin.ChatID)) }, 360, 320)
	settle(tt)
	if !tt.HasText("Weekly update draft") || !tt.HasText("Needs review") {
		t.Fatal(tt.Texts())
	}
	renderBoth(t, tt, "review-inspector")
	if err := tt.Click("Review…"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !m.hasSheet() {
		t.Fatal("inspector did not open the review editor")
	}
}

func TestReviewClosedSheetIgnoresAQueuedReply(t *testing.T) {
	m, st, tt := reviewTester(t, "draft", "pending")
	st.payload = "Save this exact local draft"
	st.act("edit")
	if !st.busy {
		t.Fatal("save did not enter request state")
	}
	// Build a busy frame without running the posted answer: the bound fields are disabled.
	tt.Frame()
	m.sheets[len(m.sheets)-1].dismiss()
	runPosts()
	tt.Frame()
	if !st.closed || m.hasSheet() || st.item.Version != 1 {
		t.Fatal("late reply revived or changed a closed editor")
	}
	if store.Review(st.item.ID).Version != 2 {
		t.Fatal("acknowledged save was lost from the store")
	}
}
