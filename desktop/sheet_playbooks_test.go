package main

import (
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A DM shows its bot's skills and a group its own, drafts first; a chat with none shows no section.
// A row opens the skill's sheet, which says who can use it.
func TestInspectorSkills(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	wantText(t, tt, L("Skills"), "flaky-tests", "release-notes", "ship-checklist", L("Draft"))
	renderBoth(t, tt, "skills-inspector")

	m.selectChat("chat-scout")
	settle(tt)
	if tt.HasText(L("Skills")) {
		t.Errorf("a bot without skills shows the section: %q", tt.Texts())
	}
	m.selectChat("chat-relay")
	settle(tt)
	wantText(t, tt, "launch-post")
	if tt.HasText("release-notes") {
		t.Error("the group shows a member's own skills")
	}

	m.selectChat("chat-patch")
	settle(tt)
	if err := tt.Click("release-notes"); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	wantText(t, tt, L("%@ can use it in every chat.", "Developer"), L("Instructions"), L("History"), L("Delete…"))
	renderBoth(t, tt, "skills-sheet")
	for i, name := range []string{"references", "scripts", "history"} {
		if err := tt.Click([]string{L("References"), L("Scripts"), L("History")}[i]); err != nil {
			t.Fatal(err)
		}
		settle(tt)
		renderTo(t, tt, "skills-sheet-"+name)
	}
	wantText(t, tt, L("Edited"), L("Created"))
}

// A new skill saves only once it has a name the CLI takes, a description, and instructions; a name
// typed the way people write it goes as a slug.
func TestSkillSheetNeedsAName(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	s := m.presentPlaybook(model.BotScope("bot-patch"), nil)
	settleTransitions(tt)
	if s.canSave() {
		t.Fatal("an empty skill can be saved")
	}
	s.name, s.description, s.instructions = "Weekly Report", "For the Friday report", "Compare the numbers"
	settle(tt)
	if !s.canSave() || s.content().Name != "weekly-report" {
		t.Fatalf("name %q, can save %v", s.content().Name, s.canSave())
	}
	s.name = "weekly/report!"
	settle(tt)
	wantText(t, tt, L("Use lowercase letters, numbers, and hyphens."))
	renderBoth(t, tt, "skills-new-bad-name")
	if s.canSave() {
		t.Error("a name the CLI refuses can be saved")
	}
	if !safeFileName("checklist.md") || safeFileName("../outside.md") || safeFileName(".env") {
		t.Error("bundled file names")
	}
}

// Save as Skill on a reply offers the messages up to it with the request and the reply picked, and
// opens the draft for review; Save as Standing Instruction needs two corrections.
func TestSaveAsSkill(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-patch")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	chat := store.Chat("chat-patch")
	reply := chat.Messages[len(chat.Messages)-1]
	s := m.presentPlaybookCapture(chat.ID, reply)
	settleTransitions(tt)
	wantText(t, tt, L("Save as Skill"), L("You"), "Developer")
	if !s.enough() {
		t.Fatal("the reply and its request are not enough")
	}
	renderBoth(t, tt, "skills-capture")
	before := len(store.Skills(model.BotScope("bot-patch")))
	if err := tt.Click(L("Continue")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	wantText(t, tt, L("Writing a draft…"))
	renderTo(t, tt, "skills-capture-drafting-light")
	time.Sleep(1300 * time.Millisecond)
	settleTransitions(tt)
	wantText(t, tt, "relay-deploy", L("A draft. Once you save it, %@ can use it in every chat.", "Developer"))
	renderBoth(t, tt, "skills-captured-draft")
	if after := store.Skills(model.BotScope("bot-patch")); len(after) != before+1 || !after[0].IsDraft() {
		t.Errorf("the draft is not listed first: %+v", after)
	}
	if err := tt.Click(L("Cancel")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)

	m.selectChat("chat-relay")
	settle(tt)
	group := store.Chat("chat-relay")
	var last *model.Message
	for _, message := range group.Messages {
		if message.Author.Kind == model.AuthorYou {
			last = message
		}
	}
	corrections := m.presentPlaybookCapture(group.ID, last)
	settleTransitions(tt)
	wantText(t, tt, L("Save as Standing Instruction"), L("Used by"), L("The bots in %@", "Launch room"))
	if corrections.enough() {
		t.Error("one correction is enough")
	}
	for _, source := range corrections.sources {
		corrections.picked[source.ID] = true
	}
	settle(tt)
	if !corrections.enough() {
		t.Error("two corrections are not enough")
	}
	renderBoth(t, tt, "skills-corrections")
}
