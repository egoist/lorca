package main

import (
	"slices"
	"testing"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func accessSheet(t *testing.T, botID string) (*mainWindow, *botAccessSheet, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	m.userWantsInspector = false
	st := m.presentBotAccess(botID)
	tt := ui.NewTester(m.frame(m.view), 1080, 900)
	runPosts()
	settleTransitions(tt)
	return m, st, tt
}

func click(t *testing.T, tt *ui.Tester, label string) {
	t.Helper()
	if err := tt.Click(label); err != nil {
		t.Fatal(err)
	}
	settle(tt)
}

func choose(t *testing.T, tt *ui.Tester, popUp, item string) {
	t.Helper()
	if err := tt.Click(popUp); err != nil {
		t.Fatal(err)
	}
	if err := tt.ChooseMenuItem(item); err != nil {
		t.Fatal(err)
	}
	settle(tt)
}

func TestAccessSheetSavesLevelsToolsFilesAndShell(t *testing.T) {
	m, _, tt := accessSheet(t, "bot-nova")
	click(t, tt, L("%@ tools", "GitHub"))
	if !tt.HasText("Merge a pull request") || !tt.HasText(L("All tools")) {
		t.Fatal("GitHub's tools did not show")
	}
	click(t, tt, "Merge a pull request")
	if !tt.HasText(L("%d of %d tools", 4, 5)) {
		t.Fatal("the tools summary did not follow the checkbox")
	}
	choose(t, tt, L("Access to %@", "Linear"), L("No access"))
	choose(t, tt, L("Access to %@", "deepwiki"), L("Read only"))
	// The Files and Shell commands rows share their controls' names: each control is at its
	// row's end.
	files, _ := tt.Find(L("Files"))
	tt.ClickAt(files.X+files.W-40, files.Y+files.H/2)
	if err := tt.ChooseMenuItem(L("Read only")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	shell, _ := tt.Find(L("Shell commands"))
	tt.ClickAt(shell.X+shell.W-18, shell.Y+shell.H/2)
	settle(tt)
	click(t, tt, L("Save"))
	if m.hasSheet() {
		t.Fatal("Save did not close the sheet")
	}
	policy := store.Bot("bot-nova").Permissions
	if policy == nil || policy.Connections == nil || policy.Shell || policy.Filesystem != model.AccessRead {
		t.Fatalf("saved policy: %+v", policy)
	}
	grants := *policy.Connections
	if _, ok := grants["linear"]; ok || policy.Level("deepwiki") != model.AccessRead || policy.Level("filesystem") != model.AccessWrite {
		t.Fatalf("plugin levels: %+v", grants)
	}
	if tools := grants["github"].Tools; tools == nil || len(*tools) != 4 || slices.Contains(*tools, "merge_pull_request") {
		t.Fatalf("GitHub's tools: %+v", grants["github"])
	}
}

func TestAccessSheetLeavesFullAccessOpenToNewPlugins(t *testing.T) {
	_, st, tt := accessSheet(t, "bot-nova")
	click(t, tt, L("%@ tools", "GitHub"))
	click(t, tt, "Merge a pull request")
	click(t, tt, "Merge a pull request")
	if policy := st.policy(); policy.Connections != nil {
		t.Fatalf("every plugin open should stay every plugin: %+v", policy)
	}
	click(t, tt, L("Save"))
	if store.Bot("bot-nova").Permissions != nil {
		t.Fatal("an unchanged sheet wrote a policy")
	}
}

func TestAccessSheetKeepsChoicesWhenTheToolsArrive(t *testing.T) {
	m := demoWindow(t)
	st := m.presentBotAccess("bot-quill")
	if len(st.plugins) == 0 || len(st.plugins[0].Tools) != 0 {
		t.Fatal("the sheet opens on the Runner's own list")
	}
	st.levels["linear"] = model.AccessRead
	runPosts()
	if st.levels["linear"] != model.AccessRead || len(st.plugins[0].Tools) == 0 {
		t.Fatal("the tools replaced what the user chose")
	}
	closed := m.presentBotAccess("bot-quill")
	m.sheets[len(m.sheets)-1].dismiss()
	runPosts()
	if !closed.closed || len(closed.plugins[0].Tools) != 0 {
		t.Fatal("a closed sheet took the tools")
	}
}

func TestAccessRequestOpensTheSheetOrIsDismissed(t *testing.T) {
	m := demoWindow(t)
	m.userWantsInspector = false
	m.selectChat("chat-quill")
	tt := ui.NewTester(m.frame(m.view), 1080, 880)
	settleTransitions(tt)
	if tt.HasText(L("Always allow")) || tt.HasText(L("Allow once")) || !tt.HasText(L("Not allowed in this bot's Access settings.")) {
		t.Fatal("the access request offers to allow the call")
	}
	click(t, tt, L("Edit Access…"))
	settleTransitions(tt)
	if !m.hasSheet() || !tt.HasText(L("Access for %@", "Writer")) {
		t.Fatal("Edit Access did not open the sheet")
	}
	click(t, tt, L("Cancel"))
	settleTransitions(tt)
	click(t, tt, L("Dismiss"))
	for _, message := range store.Chat("chat-quill").Messages {
		if request := message.Body.Request; request != nil && request.IsAccess() && request.Decision != model.DecisionDismissed {
			t.Fatal("Dismiss did not dismiss the request")
		}
	}
}

func TestRenderDesktopAccess(t *testing.T) {
	m := demoWindow(t)
	writer := store.Bot("bot-quill")
	inspector := ui.NewTester(func(c *ui.Context) {
		applyTheme(c)
		ui.Column(c).Padding(16).Gap(20).Children(func() {
			m.inspectorProfile(c, writer)
			m.inspectorPlugins(c, writer)
		})
	}, 300, 520)
	inspector.SetScale(2)
	settle(inspector)
	if !inspector.HasText(L("Limited")) || !inspector.HasText(L("No access")) {
		t.Fatal("the inspector does not show the Writer's Access")
	}
	renderBoth(t, inspector, "access-inspector")

	m.userWantsInspector = false
	m.selectChat("chat-quill")
	chat := ui.NewTester(m.frame(m.view), 1000, 620)
	chat.SetScale(2)
	settleTransitions(chat)
	renderBoth(t, chat, "access-card")

	m.presentBotAccess("bot-quill")
	runPosts()
	sheet := ui.NewTester(m.frame(m.view), 1000, 760)
	sheet.SetScale(2)
	settleTransitions(sheet)
	renderBoth(t, sheet, "access-sheet")
	click(t, sheet, L("%@ tools", "GitHub"))
	sheet.Move(2, 2)
	renderBoth(t, sheet, "access-sheet-tools")
}
