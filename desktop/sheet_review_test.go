package main

import (
	"encoding/json"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// reviewWindow is the demo on Project Manager's chat, where its routine left a command, a GitHub
// call, and a draft for review.
func reviewWindow(t *testing.T) (*mainWindow, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	m.selectChat("chat-nova")
	tt := ui.NewTester(m.frame(m.view), 1180, 820)
	settleTransitions(tt)
	return m, tt
}

func TestReviewApprovesTheEditedCommand(t *testing.T) {
	m, tt := reviewWindow(t)
	renderBoth(t, tt, "review-inspector")
	if err := tt.Click("git tag v1.4.0 && git push origin v1.4.0"); err != nil {
		t.Fatal(err, tt.Texts())
	}
	settleTransitions(tt)
	if !m.hasSheet() || !tt.HasText("Project Manager wants to run a command on Workbench") || tt.HasText("Save Changes") || tt.HasText("Reload") {
		t.Fatal(tt.Texts())
	}
	renderBoth(t, tt, "review-command")
	if err := tt.Click("Command"); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("git tag v1.4.1 && git push origin v1.4.1")
	if err := tt.Click("Approve"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	item := store.Review("review-tag")
	if item.State != "succeeded" || item.Version != 2 || item.Payload.EditorText() != "git tag v1.4.1 && git push origin v1.4.1" {
		t.Fatalf("approved %+v", item)
	}
	if m.hasSheet() {
		t.Fatal("the sheet stayed up after the decision")
	}
}

func TestReviewRejectsACall(t *testing.T) {
	m, tt := reviewWindow(t)
	// The row names the call by its plugin, as the permission card does, not by the tool's id.
	if err := tt.Click("GitHub"); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if !m.hasSheet() || !tt.HasText("Project Manager wants to use GitHub") {
		t.Fatal(tt.Texts())
	}
	renderBoth(t, tt, "review-call")
	if err := tt.Click("Reject"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Review("review-comment").State != "rejected" || m.hasSheet() {
		t.Fatal("reject not recorded")
	}
}

func TestReviewEditedElsewhereOffersTheNewVersion(t *testing.T) {
	m, tt := reviewWindow(t)
	st := m.presentReview(*store.Review("review-draft"))
	settleTransitions(tt)
	if err := tt.Click("Draft"); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Type("My edit")
	tt.Frame()
	// Another Device saves its own edit as version 2.
	next := store.Review("review-draft").Clone()
	next.Version, next.Revision = 2, 2
	next.Payload = model.ReviewPayload{Kind: "draft", Text: "Edited on another Device"}
	store.Reviews[2] = &next
	settle(tt)
	if st.item.Version != 1 || st.text != "My edit" {
		t.Fatal("a sync replaced the edit in progress")
	}
	if err := tt.Click("Approve"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Review("review-draft").State != "pending" || !tt.HasText("This changed on another Device") {
		t.Fatal("approved without showing the new version", tt.Texts())
	}
	renderBoth(t, tt, "review-changed-elsewhere")
	if err := tt.Click("Show Latest"); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if st.item.Version != 2 || st.text != "Edited on another Device" {
		t.Fatal("Show Latest kept the old version")
	}
	// The field shows the new text, not just the state behind it.
	if err := tt.Click("Draft"); err != nil {
		t.Fatal(err)
	}
	tt.Key(ui.Cmd, ui.KeyA)
	tt.Key(ui.Cmd, ui.KeyC)
	if tt.Clipboard() != "Edited on another Device" {
		t.Fatalf("editor shows %q", tt.Clipboard())
	}
}

func TestReviewDecidedShowsHowItWent(t *testing.T) {
	m, tt := reviewWindow(t)
	done := store.Review("review-tag").Clone()
	done.State, done.Revision = "succeeded", 3
	result, _ := json.Marshal(map[string]string{"text": "To github.com:lorca-app/relay.git\n * [new tag]         v1.4.0 -> v1.4.0"})
	done.Outcome = &model.ReviewOutcome{Summary: "Done", Result: result}
	store.Reviews[0] = &done
	m.presentReview(done)
	settleTransitions(tt)
	if !tt.HasText("Done") || !tt.HasText("Output") || tt.HasText("Approve") || tt.HasText("Reject") {
		t.Fatal(tt.Texts())
	}
	renderBoth(t, tt, "review-done")
	tt.Key(0, ui.KeyEnter)
	settle(tt)
	if m.hasSheet() {
		t.Fatal("Done left the sheet up")
	}
}

func TestReviewSectionShowsOnlyWhatIsOpen(t *testing.T) {
	_, tt := reviewWindow(t)
	if !tt.HasText(L("Waiting for review")) {
		t.Fatal(tt.Texts())
	}
	for _, item := range store.Reviews {
		store.RejectReview(item.Clone(), func(model.ReviewItem, error) {})
	}
	settle(tt)
	if tt.HasText(L("Waiting for review")) {
		t.Fatal("decided items stayed in the inspector", tt.Texts())
	}
}

func TestReviewClosedSheetIgnoresALateReply(t *testing.T) {
	m, tt := reviewWindow(t)
	st := m.presentReview(*store.Review("review-tag"))
	settleTransitions(tt)
	st.approve(&m.appWindow, m.sheets[len(m.sheets)-1])
	if !st.busy {
		t.Fatal("approve did not wait for the Runner")
	}
	tt.Frame()
	m.sheets[len(m.sheets)-1].dismiss()
	runPosts()
	tt.Frame()
	if !st.closed || m.hasSheet() {
		t.Fatal("a late reply revived a closed sheet")
	}
	if store.Review("review-tag").State != "succeeded" {
		t.Fatal("the approval the Runner acknowledged was lost")
	}
}
