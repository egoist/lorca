package main

import (
	"slices"
	"testing"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

func TestRenderChannelsInInspector(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-tally")
	tt := ui.NewTester(m.frame(m.view), 1180, 1500)
	settle(tt)
	if !tt.HasText(L("Channels")) && !tt.HasText("CHANNELS") {
		t.Errorf("no Channels section: %q", tt.Texts())
	}
	if !tt.HasText("Community feedback") || !tt.HasText("Mentions, replies, #feedback") {
		t.Errorf("texts %q", tt.Texts())
	}
	renderBoth(t, tt, "inspector-channels")

	store.SetChannelPaused("ev-feedback", true)
	settle(tt)
	if !store.Channel("ev-feedback").IsPaused() {
		t.Error("the switch did not pause the channel")
	}
	store.SetChannelPaused("ev-feedback", false)

	// A held message reads in orange before what the channel takes.
	store.Channel("ev-feedback").State, store.Channel("ev-feedback").HeldDelivery = model.ChannelHeld, "d1"
	settle(tt)
	if !tt.HasText(L("On hold")) {
		t.Errorf("no On hold: %q", tt.Texts())
	}
	renderBoth(t, tt, "inspector-channels-held")

	// A bot without channels has no section.
	m.selectChat("chat-nova")
	settle(tt)
	if tt.HasText("Community feedback") {
		t.Error("another bot shows the channel")
	}
}

func TestChannelSheet(t *testing.T) {
	m, tt := sheetTester(t, func(m *mainWindow) {})
	m.presentChannel("ev-feedback", store.Bot("bot-tally"), m.open)
	settle(tt)
	for _, text := range []string{L("Listening"), "Telegram · Community", "Mentions, replies, #feedback", L("Asking to Chat"), "Acme Beta Testers", L("Group · %@", "Priya Nair"), L("Conversations"), "Acme Community"} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "channel-sheet")

	// Allowed, the chat that asked joins the channel's chats.
	if err := tt.Click(L("Allow")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if channel := store.Channel("ev-feedback"); len(channel.Requests) != 0 || len(channel.Chats) != 2 || channel.Chats[1].Title != "Acme Beta Testers" {
		t.Errorf("Allow left %+v", channel)
	}
	if tt.HasText(L("Asking to Chat")) {
		t.Error("the section stays with nothing asking")
	}

	// Add… asks for a chat's id, for a chat that can't ask itself.
	if err := tt.Click(L("Add…")); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	if !tt.HasText(L("Add a Chat")) {
		t.Errorf("no prompt: %q", tt.Texts())
	}
	renderBoth(t, tt, "channel-sheet-add-chat")
	tt.Key(0, ui.KeyEscape)
	settleTransitions(tt)
	store.AddChat("ev-feedback", "-1003", func(err error) {
		if err != nil {
			t.Error(err)
		}
	})
	settle(tt)
	if chats := store.Channel("ev-feedback").Chats; len(chats) != 3 || chats[2].ID != "-1003" {
		t.Errorf("AddChat left %+v", chats)
	}
	closeSheet(t, m, tt)

	store.Channel("ev-feedback").State, store.Channel("ev-feedback").HeldDelivery = model.ChannelHeld, "d1"
	m.presentChannel("ev-feedback", store.Bot("bot-tally"), m.open)
	settle(tt)
	renderBoth(t, tt, "channel-sheet-held")
	if err := tt.Click(L("Skip")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if channel := store.Channel("ev-feedback"); channel.State != model.ChannelListening || channel.HeldDelivery != "" {
		t.Errorf("Skip left %+v", channel)
	}
	closeSheet(t, m, tt)

	// A conversation's row opens it.
	m.presentChannel("ev-feedback", store.Bot("bot-tally"), m.open)
	settle(tt)
	// The section sits below the sheet's other sections, so the window makes room for it.
	tt.SetSize(1000, 1500)
	settle(tt)
	// The sidebar's row of the same name comes first in build order: click the sheet's, the last
	// row of its Conversations section.
	section, ok := tt.Find(L("Conversations"))
	if !ok {
		t.Fatal("no Conversations section")
	}
	tt.ClickAt(section.X+60, section.Y+section.H-16)
	settleTransitions(tt)
	if m.hasSheet() || m.selection.ChatID != "chat-community" {
		t.Errorf("the row left the sheet up or opened %q", m.selection.ChatID)
	}
}

func TestRenderChannelConversation(t *testing.T) {
	m := demoWindow(t)
	m.selectChat("chat-community")
	tt := ui.NewTester(m.frame(m.view), 1180, 900)
	settle(tt)
	for _, text := range []string{"Acme Community", "Alice Chen", "Ben Ortiz", "Thanks Alice, tracked in #142."} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "channel-conversation")

	// The conversation is only a transcript: deleting it keeps the bot and its DM.
	if store.DM("bot-tally") != "chat-tally" {
		t.Errorf("the bot's DM is %q", store.DM("bot-tally"))
	}
	store.DeleteChat("chat-community")
	if store.Bot("bot-tally") == nil || store.Chat("chat-tally") == nil {
		t.Error("deleting a channel's conversation deleted its bot")
	}
}

// intakeSheet opens the channel's sheet in a window tall enough for all of it.
func intakeSheet(t *testing.T) (*mainWindow, *ui.Tester) {
	t.Helper()
	m, tt := sheetTester(t, func(m *mainWindow) {})
	tt.SetSize(1000, 1500)
	m.presentChannel("ev-feedback", store.Bot("bot-tally"), m.open)
	settleTransitions(tt)
	return m, tt
}

func clickText(t *testing.T, tt *ui.Tester, text string) {
	t.Helper()
	if err := tt.Click(text); err != nil {
		t.Fatalf("%v in %q", err, tt.Texts())
	}
	settleTransitions(tt)
}

func TestChannelSheetIntakeMode(t *testing.T) {
	_, tt := intakeSheet(t)
	record := L("The bot reads each message against your rules and records what it would take on. Someone you list still gets what they ask for.")
	for _, text := range []string{L("Intake"), L("Only Record"), record, L("Can Ask Directly"), L("Rules"), "Bug reports", L("Decisions")} {
		if !tt.HasText(text) {
			t.Errorf("no %q in %q", text, tt.Texts())
		}
	}
	renderBoth(t, tt, "channel-sheet-intake")

	pick := func(from, to string) {
		t.Helper()
		clickText(t, tt, from)
		if err := tt.ChooseMenuItem(to); err != nil {
			t.Fatalf("%v: %q", err, tt.Menu())
		}
		settleTransitions(tt)
	}
	pick(L("Only Record"), L("On"))
	if intake := store.Channel("ev-feedback").Intake; intake == nil || intake.Mode != model.IntakeAuto || len(intake.Rules) != 1 {
		t.Fatalf("On left %+v", intake)
	}
	if !tt.HasText(L("The bot takes on what your rules cover, or what someone you list asks for, as a task in its own chat.")) {
		t.Errorf("no note for On: %q", tt.Texts())
	}

	// Off hides everything under it, and keeps what the owner wrote.
	pick(L("On"), L("Off"))
	if intake := store.Channel("ev-feedback").Intake; intake == nil || intake.Mode != model.IntakeOff || len(intake.Rules) != 1 {
		t.Fatalf("Off left %+v", intake)
	}
	if !tt.HasText(L("Each message the channel takes starts a turn.")) {
		t.Errorf("no note for Off: %q", tt.Texts())
	}
	for _, text := range []string{L("Can Ask Directly"), L("Rules"), L("Decisions")} {
		if tt.HasText(text) {
			t.Errorf("%q shows while Off", text)
		}
	}

	// Choosing again brings it all back, at any time.
	pick(L("Off"), L("Only Record"))
	if intake := store.Channel("ev-feedback").Intake; intake == nil || intake.Mode != model.IntakeObserve || len(intake.Rules) != 1 {
		t.Fatalf("Only Record left %+v", intake)
	}
}

func TestChannelSheetCanAskDirectly(t *testing.T) {
	_, tt := intakeSheet(t)
	section, ok := tt.Find(L("Can Ask Directly"))
	if !ok {
		t.Fatalf("no section in %q", tt.Texts())
	}
	// Its Add… sits at the end of its title's line; the Chats row's comes first in build order.
	tt.ClickAt(section.X+section.W-12, section.Y+7)
	settleTransitions(tt)
	if !tt.HasText(L("Add Someone")) {
		t.Fatalf("no prompt: %q", tt.Texts())
	}
	renderBoth(t, tt, "channel-sheet-add-person")
	tt.Type(" 5120099 ")
	settle(tt)
	clickText(t, tt, L("Add"))
	intake := store.Channel("ev-feedback").Intake
	if len(intake.People) != 1 || intake.People[0] != (model.IntakePerson{ID: "5120099"}) {
		t.Fatalf("Add left %+v", intake.People)
	}
	if !tt.HasText("5120099") {
		t.Errorf("no row by id: %q", tt.Texts())
	}
	clickText(t, tt, L("Remove"))
	if intake := store.Channel("ev-feedback").Intake; len(intake.People) != 0 {
		t.Errorf("Remove left %+v", intake.People)
	}
}

func TestChannelSheetRules(t *testing.T) {
	m, tt := intakeSheet(t)
	row, ok := tt.Find("Bug reports")
	if !ok {
		t.Fatalf("no rule in %q", tt.Texts())
	}
	// Its switch turns it off without opening it.
	tt.ClickAt(row.X+row.W-20, row.Y+row.H/2)
	settle(tt)
	if rule := store.Channel("ev-feedback").Intake.Rule("rule-bugs"); rule == nil || rule.Enabled {
		t.Fatalf("the switch left %+v", rule)
	}
	if len(m.sheets) != 1 {
		t.Fatalf("the switch opened a sheet")
	}

	clickText(t, tt, "Bug reports")
	if !tt.HasText(L("Instruction")) || !tt.HasText(L("Delete…")) {
		t.Fatalf("no rule sheet: %q", tt.Texts())
	}
	renderBoth(t, tt, "channel-sheet-rule")
	tt.Type(" and crashes")
	settle(tt)
	clickText(t, tt, L("Save"))
	rule := store.Channel("ev-feedback").Intake.Rule("rule-bugs")
	if rule == nil || rule.Name != "Bug reports and crashes" || rule.Enabled {
		t.Fatalf("Save left %+v", rule)
	}
	if len(m.sheets) != 1 {
		t.Errorf("Save left the rule sheet up")
	}

	// A new rule needs both fields, and takes an id at once so its row stays itself.
	clickText(t, tt, L("Add Rule"))
	if !tt.HasText(L("New Rule")) {
		t.Fatalf("no new rule sheet: %q", tt.Texts())
	}
	tt.Type("Billing")
	settle(tt)
	cancel, ok := tt.Find(L("Cancel"))
	if !ok {
		t.Fatalf("no Cancel: %q", tt.Texts())
	}
	// The default button sits after Cancel; the + behind the sheet has its name too.
	tt.ClickAt(cancel.X+cancel.W+40, cancel.Y+cancel.H/2)
	settle(tt)
	if rules := store.Channel("ev-feedback").Intake.Rules; len(rules) != 1 || len(m.sheets) != 2 {
		t.Fatalf("a rule saved without its instruction: %+v", rules)
	}
	label, ok := tt.Find(L("Instruction"))
	if !ok {
		t.Fatalf("no Instruction: %q", tt.Texts())
	}
	tt.ClickAt(label.X+200, label.Y+10)
	settle(tt)
	tt.Type("Refund questions")
	settle(tt)
	tt.ClickAt(cancel.X+cancel.W+40, cancel.Y+cancel.H/2)
	settleTransitions(tt)
	rules := store.Channel("ev-feedback").Intake.Rules
	if len(rules) != 2 || len(m.sheets) != 1 || rules[1].Name != "Billing" || rules[1].Instruction != "Refund questions" || !rules[1].Enabled || rules[1].ID == "" {
		t.Fatalf("Add Rule left %+v with %d sheets", rules, len(m.sheets))
	}
	store.SetChannelIntake("ev-feedback", new(store.Channel("ev-feedback").Intake.WithoutRule(rules[1].ID)), nil)
	settle(tt)

	clickText(t, tt, "Bug reports and crashes")
	clickText(t, tt, L("Cancel"))
	if len(m.sheets) != 1 {
		t.Fatal("Cancel left the rule sheet up")
	}

	clickText(t, tt, "Bug reports and crashes")
	clickText(t, tt, L("Delete…"))
	if !tt.HasText(L("Delete “%@”?", "Bug reports and crashes")) {
		t.Fatalf("no confirmation: %q", tt.Texts())
	}
	clickText(t, tt, L("Delete"))
	if intake := store.Channel("ev-feedback").Intake; len(intake.Rules) != 0 {
		t.Fatalf("Delete left %+v", intake.Rules)
	}
	if !tt.HasText(L("No rules yet: it only takes on what someone you list asks for.")) {
		t.Errorf("no empty state: %q", tt.Texts())
	}
	if len(m.sheets) != 1 {
		t.Errorf("Delete left the rule sheet up")
	}
}

func TestChannelSheetDecisions(t *testing.T) {
	m, tt := intakeSheet(t)
	const shown, left = "Dashboard forgets the last filter", "Agrees with an earlier report and asks for nothing new."
	if !tt.HasText(shown) {
		t.Fatalf("no decision: %q", tt.Texts())
	}
	if want := L("Would take on") + " · Maya · " + model.Ago(time.Now().Add(-12*time.Minute)); !tt.HasText(want) {
		t.Errorf("no %q in %q", want, tt.Texts())
	}
	if tt.HasText(left) {
		t.Error("a message left alone shows before Show Ignored")
	}
	row, ok := tt.Find(L("Show Ignored"))
	if !ok {
		t.Fatalf("no Show Ignored: %q", tt.Texts())
	}
	tt.ClickAt(row.X+row.W-20, row.Y+row.H/2)
	settle(tt)
	if !tt.HasText(left) {
		t.Errorf("Show Ignored shows nothing more: %q", tt.Texts())
	}

	// The menu lets the sender ask directly, and leaves once they can.
	if err := tt.RightClick(shown); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if err := tt.ChooseMenuItem(L("Let %@ Ask Directly", "Maya")); err != nil {
		t.Fatalf("%v: %q", err, tt.Menu())
	}
	settle(tt)
	if intake := store.Channel("ev-feedback").Intake; !intake.Lists("5120033") || intake.People[0].Name != "Maya" {
		t.Fatalf("the menu left %+v", intake.People)
	}
	if err := tt.RightClick(shown); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if slices.Contains(tt.Menu(), L("Let %@ Ask Directly", "Maya")) {
		t.Errorf("the menu still offers it: %q", tt.Menu())
	}
	tt.CloseMenu()
	settle(tt)

	// A row without a task opens its conversation.
	clickText(t, tt, shown)
	if m.hasSheet() || m.selection.ChatID != "chat-community" {
		t.Errorf("the row left the sheet up or opened %q", m.selection.ChatID)
	}
}

func TestChannelSheetDecisionOpensItsTask(t *testing.T) {
	m, tt := sheetTester(t, func(m *mainWindow) {})
	tt.SetSize(1000, 1500)
	store.AcceptDurableTask(taskFixture("task-1", model.TaskWorking, "bot-tally", "File the crash", 1))
	st := &channelSheet{w: &m.appWindow, channelID: "ev-feedback", asked: true, decisions: []model.IntakeDecision{
		{ID: "d1", Kind: model.DecisionAccepted, Title: "File the crash report", Sender: "Maya", ConversationID: "chat-community", TaskID: "task-1"},
		{ID: "d2", Kind: model.DecisionUnavailable, Reason: "Its model failed.", SenderID: "5120033"},
	}}
	m.present(func(c *ui.Context, s *sheet) {
		m.channelView(c, s, st, "Community feedback", "ev-feedback", store.Bot("bot-tally"), m.open)
	}, nil)
	settleTransitions(tt)
	// Without a title the reason reads, and without a name the id.
	if !tt.HasText("Its model failed.") {
		t.Errorf("no reason: %q", tt.Texts())
	}
	// A row with nowhere to go does nothing when clicked.
	clickText(t, tt, "Its model failed.")
	if len(m.sheets) != 1 {
		t.Fatalf("the row changed the sheets: %d", len(m.sheets))
	}
	clickText(t, tt, "File the crash report")
	if len(m.sheets) != 2 || !tt.HasText(L("Goal")) {
		t.Fatalf("no task sheet over the channel's: %d, %q", len(m.sheets), tt.Texts())
	}
}

func TestChannelSheetDecisionsOffline(t *testing.T) {
	m, tt := sheetTester(t, func(m *mainWindow) {})
	tt.SetSize(1000, 1500)
	runner := store.Device(store.Bot("bot-tally").RunnerID)
	runner.Status = model.StatusOffline
	m.presentChannel("ev-feedback", store.Bot("bot-tally"), m.open)
	settleTransitions(tt)
	if !tt.HasText(runner.Name + " is offline.") {
		t.Fatalf("no offline state: %q", tt.Texts())
	}
	renderBoth(t, tt, "channel-sheet-decisions-offline")
	runner.Status = model.StatusOnline
	clickText(t, tt, L("Retry"))
	if !tt.HasText("Dashboard forgets the last filter") || tt.HasText(L("Retry")) {
		t.Errorf("Retry left %q", tt.Texts())
	}
}

func TestWorkChatsStayOutOfLists(t *testing.T) {
	m := demoWindow(t)
	runPosts()
	work := &model.Chat{ID: "chat-work", Kind: model.ChatDM, BotIDs: []string{"bot-tally"}, IsHidden: true, UnreadCount: 5, CreatedAt: time.Now(),
		Channel: &model.ChatChannel{ChannelID: "ev-feedback", Service: "telegram", TaskID: "task-1"}}
	before := unreadBadge()
	store.Chats = append([]*model.Chat{work}, store.Chats...)
	if unreadBadge() != before {
		t.Errorf("a work chat counts on the badge: %d, was %d", unreadBadge(), before)
	}
	if first := store.FirstChat(); first == nil || first.ID == "chat-work" {
		t.Errorf("the first chat is %+v", first)
	}
	if slices.ContainsFunc(store.Conversations("ev-feedback"), func(chat *model.Chat) bool { return chat.ID == "chat-work" }) {
		t.Error("a work chat is among the channel's conversations")
	}
	// In every group, hidden or not; and the palette only finds it by searching.
	for _, hidden := range []bool{true, false} {
		work.IsHidden = hidden
		for _, sections := range [][]*model.Section{nil, {{ID: "s1", Name: "Pipeline"}}} {
			for _, row := range sidebarRows(store.Chats, sections, true, false) {
				if row.chat != nil && row.chat.ID == "chat-work" {
					t.Errorf("a work chat lists with hidden=%v sections=%d", hidden, len(sections))
				}
			}
		}
	}
	if _, ok := groupOf(work); ok {
		t.Error("a work chat has a group to unfold")
	}
	found := false
	for _, item := range paletteChats(m) {
		if item.chatID == "chat-work" {
			found = true
			if !item.searchOnly {
				t.Error("a work chat lists in the palette before a search")
			}
		}
	}
	if !found {
		t.Error("the palette can't find a work chat")
	}
}
