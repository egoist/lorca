package main

import (
	"slices"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func accessFixture(t *testing.T) (*mainWindow, *botAccessSheet, *ui.Tester) {
	t.Helper()
	m := demoWindow(t)
	m.userWantsInspector = false
	// Only synthetic named instances are advertised in this fixture.
	store.Device("dev-workbench").Plugins = nil
	st := m.presentBotAccess("bot-nova")
	tt := ui.NewTester(m.frame(m.view), 1080, 880)
	settleTransitions(tt)
	return m, st, tt
}

func accessScroll(tt *ui.Tester, dy float32) { tt.Scroll(530, 400, 0, dy); settle(tt) }

func TestNativeAccessChoicesPersistAndSave(t *testing.T) {
	m, st, tt := accessFixture(t)
	if err := tt.Click(L("All connections")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	for _, capability := range []string{L("Read"), L("Draft"), L("Write")} {
		if err := tt.Click(L("Allow %@ for %@", capability, "Gmail · Personal")); err != nil {
			t.Fatal(err)
		}
		settle(tt)
	}
	if err := tt.Click(L("Allow %@ for %@", L("Write"), "Gmail · Work")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.Click(L("All tools for %@", "Gmail · Work")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.Click(L("Allow %@ on %@", "send_message", "Gmail · Work")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	for range 5 {
		tt.Frame()
	}
	work := st.draft.Connections["gmail-11111111111111111111111111111111"]
	if !work.Read || !work.Draft || work.Write || work.AllTools || work.Tools["send_message"].Selected {
		t.Fatalf("instance/tool/capability controls: all=%v work=%+v send=%+v", st.draft.AllConnections, work, work.Tools["send_message"])
	}
	if err := tt.Click(L("All local tools")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	renderBoth(t, tt, "desktop-access-connections")
	accessScroll(tt, 1600)
	if err := tt.Click(L("Allow local tool %@", "write")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.Click(L("Allow shell commands")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.Click(L("Filesystem access")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.ChooseMenuItem(L("Read")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if st.draft.Shell || st.draft.Filesystem != "read" {
		t.Fatal("local controls did not change")
	}
	renderBoth(t, tt, "desktop-access-local-controls")
	if err := tt.Click(L("Save")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if m.hasSheet() || store.Bot("bot-nova").Permissions.Shell {
		t.Fatal("save did not update profile and close")
	}
	if tools := store.Bot("bot-nova").Permissions.Tools; tools == nil || slices.Contains(*tools, "write") {
		t.Fatal("local tool selection was not saved")
	}
	if grants := *store.Bot("bot-nova").Permissions.Connections; len(grants["gmail-22222222222222222222222222222222"].Capabilities) != 0 {
		t.Fatal("Personal gained capabilities")
	}
}

func TestAccessCardOpensEditorWithoutGrantingTheCall(t *testing.T) {
	m := demoWindow(t)
	m.userWantsInspector = false
	m.selectChat("chat-nova")
	store.Append(&model.Message{ID: "access-fixture", Author: model.BotAuthor("bot-nova"), Body: model.Body{Kind: model.BodyPermission,
		Request: &model.PermissionRequest{PluginID: "computer", PluginName: "Bot access", Tool: "access", Summary: "Analyst needs access to send_message", Decision: model.DecisionPending,
			Reason: "Write access is excluded. Edit this bot's Access settings in its profile."}}, State: model.MessageState{Kind: model.StateComplete}, CreatedAt: time.Now()}, "chat-nova")
	tt := ui.NewTester(m.frame(m.view), 1080, 880)
	settleTransitions(tt)
	if tt.HasText(L("Always allow")) || tt.HasText(L("Allow once")) {
		t.Fatal("refusal offers action approval")
	}
	renderBoth(t, tt, "desktop-access-refusal")
	if err := tt.Click(L("Edit Access…")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if !m.hasSheet() || !tt.HasText(L("All connections")) {
		t.Fatal("access request did not open editor")
	}
	if err := tt.Click(L("Cancel")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if err := tt.Click(L("Dismiss")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	chat := store.Chat("chat-nova")
	if chat.Messages[len(chat.Messages)-1].Body.Request.Decision != model.DecisionDenied {
		t.Fatal("dismiss did not answer refusal")
	}
}

func TestAccessEditorRetainsDraftOnSaveConflict(t *testing.T) {
	_, st, tt := accessFixture(t)
	st.draft.Shell = false
	store.Bot("bot-nova").Permissions = &model.BotPermissions{Shell: true, Filesystem: "read"}
	if err := tt.Click(L("Save")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if st.closed || st.draft.Shell || st.problem == "" {
		t.Fatal("conflict discarded draft or accepted stale settings")
	}
}

func TestAccessDraftSurvivesCatalogReloadAndLanguageBuilds(t *testing.T) {
	_, st, tt := accessFixture(t)
	if err := tt.Click(L("All connections")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	work := st.draft.Connections["gmail-11111111111111111111111111111111"]
	work.Write = false
	st.load()
	settle(tt)
	l10n.Set("zh-Hans", "zh-CN")
	settle(tt)
	defer l10n.Set("en", "en-US")
	if st.draft.AllConnections || work.Write || !work.Read || !tt.HasText(L("Access")) {
		t.Fatal("rebuild/reload reset edited values")
	}
}

func TestDismissedAccessSheetIgnoresPendingCatalogReply(t *testing.T) {
	m := demoWindow(t)
	st := m.presentBotAccess("bot-nova")
	m.sheets[len(m.sheets)-1].dismiss()
	runPosts()
	if !st.closed || !st.loading || st.draft.Connections["gmail-11111111111111111111111111111111"] != nil {
		t.Fatal("closed sheet applied catalog reply")
	}
}

func TestRenderDesktopAccessProfile(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(func(c *ui.Context) { applyTheme(c); m.inspectorProfile(c, store.Bot("bot-nova")) }, 380, 250)
	settle(tt)
	renderBoth(t, tt, "desktop-access-profile-before")
	draft := model.NewAccessDraft(nil)
	draft.AllConnections, draft.AllTools, draft.Shell, draft.Filesystem = false, false, false, "read"
	store.SetBotPermissions("bot-nova", draft.Policy(), func(err error) {
		if err != nil {
			t.Fatal(err)
		}
	})
	settle(tt)
	if !tt.HasText(L("Shell denied")) && !tt.HasText(store.Bot("bot-nova").Permissions.Summary()) {
		t.Fatal("profile summary missing")
	}
	renderBoth(t, tt, "desktop-access-profile-after")
}
