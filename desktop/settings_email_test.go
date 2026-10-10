package main

import (
	"errors"
	"os"
	"slices"
	"testing"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The Email pane over the demo's address, and against answers in the CLI's shape. `LORCA_RENDER`
// saves each state at 2x in the light and dark appearances.

func emailTester(t *testing.T) (*mainWindow, *ui.Tester, *string) {
	t.Helper()
	m, tt := settingsWindowTester(t, 640)
	if os.Getenv("LORCA_RENDER") != "" {
		tt.SetScale(2)
	}
	copied := new(string)
	oldCopy := copyText
	t.Cleanup(func() { copyText = oldCopy })
	copyText = func(text string) { *copied = text }
	m.showSettings(model.PaneEmail)
	settle(tt)
	return m, tt, copied
}

func TestEmailSettingsDemo(t *testing.T) {
	m, tt, copied := emailTester(t)
	address := store.Mail.Address.Email
	if !tt.HasText(address) {
		t.Fatalf("no address: %q", tt.Texts())
	}
	if note := L("Each bot writes from an address of its own, such as %@, and mail to it goes to that bot. Other mail goes to %@.", "k7f3m9q2+developer@bots.lorca.app", "Project Manager"); !tt.HasText(note) {
		t.Errorf("no routing note: %q", tt.Texts())
	}
	renderBoth(t, tt, "desktop-email-address")

	// Copy copies the address and says so for a moment.
	if err := tt.Click(L("Copy")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if *copied != address || !tt.HasText(L("Copied")) {
		t.Fatalf("copied %q, texts %q", *copied, tt.Texts())
	}
	renderTo(t, tt, "desktop-email-copied-light")

	// Its menu holds the rows' actions.
	if err := tt.RightClick(L("Address")); err != nil {
		t.Fatal(err)
	}
	if menu := tt.Menu(); !slices.Equal(menu, []string{L("Copy Address"), L("Change Address…"), "-", L("Give Up Address…")}) {
		t.Fatalf("menu %q", menu)
	}
	tt.CloseMenu()

	// Change… names the address that stops working.
	if err := tt.Click(L("Change…")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText(L("%@ stops working: mail to it bounces, and nobody else can take the name.", address)) {
		t.Fatalf("not the change sheet: %q", tt.Texts())
	}
	renderBoth(t, tt, "desktop-email-sheet-change")
	// A reserved name stays in the sheet with the reason; a free one changes the address.
	if err := tt.Click(L("Your own name")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	tt.Type("Admin")
	tt.Key(0, ui.KeyEnter)
	settle(tt)
	if !tt.HasText(L("That name is reserved.")) || !m.hasSheet() {
		t.Fatalf("no problem shown: %q", tt.Texts())
	}
	renderBoth(t, tt, "desktop-email-sheet-problem")
	for range 5 {
		tt.Key(0, ui.KeyBackspace)
	}
	tt.Type("lorca-scout")
	tt.Key(0, ui.KeyEnter)
	settle(tt)
	if m.hasSheet() || store.Mail.Address.Name != "lorca-scout" || !tt.HasText("lorca-scout@bots.lorca.app") {
		t.Fatalf("address %+v, texts %q", store.Mail.Address, tt.Texts())
	}

	// Suspended: the row says so, and the footnote says why.
	store.Mail.Address.State = "suspended"
	settle(tt)
	if !tt.HasText(L("Suspended")) || !tt.HasText(L("Mail to this address bounces for now, because mail sent from it kept bouncing.")+" "+L("Each bot writes from an address of its own, such as %@, and mail to it goes to that bot. Other mail goes to %@.", "lorca-scout+developer@bots.lorca.app", "Project Manager")) {
		t.Fatalf("not suspended: %q", tt.Texts())
	}
	renderBoth(t, tt, "desktop-email-suspended")

	// Give Up asks first, then leaves the pane with its one row.
	if err := tt.Click(L("Give Up Address…")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText(L("Give up %@?", "lorca-scout@bots.lorca.app")) {
		t.Fatalf("no confirmation: %q", tt.Texts())
	}
	renderBoth(t, tt, "desktop-email-give-up")
	// Give Up is the alert's first button; the page's Give Up row is named the same.
	tt.Key(0, ui.KeyEnter)
	settle(tt)
	if store.Mail.Address != nil || !tt.HasText(L("Get an Address…")) {
		t.Fatalf("still has %+v: %q", store.Mail.Address, tt.Texts())
	}
	renderBoth(t, tt, "desktop-email-none")

	// A random name is the sheet's first choice.
	if err := tt.Click(L("Get an Address…")); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !tt.HasText(L("Get an Email Address")) {
		t.Fatalf("not the get sheet: %q", tt.Texts())
	}
	renderBoth(t, tt, "desktop-email-sheet-get")
	tt.Key(0, ui.KeyEnter)
	settle(tt)
	if m.hasSheet() || store.Mail.Address == nil || len(store.Mail.Address.Name) != 8 {
		t.Fatalf("no random address: %+v", store.Mail.Address)
	}
}

func TestEmailPaneOnlyWhenTheRelayOffersIt(t *testing.T) {
	m, tt, _ := emailTester(t)
	if !slices.Contains(store.ListedPanes(), model.PaneEmail) {
		t.Fatal("Email is not listed")
	}
	store.Mail.Available = false
	m.showSettings(model.PaneGeneral)
	settle(tt)
	if slices.Contains(store.ListedPanes(), model.PaneEmail) || tt.HasText(model.PaneEmail.Title()) {
		t.Fatalf("Email still listed: %q", tt.Texts())
	}
	store.Mail = nil
	if slices.Contains(store.ListedPanes(), model.PaneEmail) {
		t.Fatal("Email listed before the CLI said")
	}
}

func TestEmailAgainstTheCLI(t *testing.T) {
	f := newTemplateUIFixture(t)
	taken := true
	f.transport.handler = func(call templateUICall) (any, error) {
		status := map[string]any{"available": true, "domain": "bots.lorca.app", "address": nil, "lead_bot_id": "bot-nova", "bots": []any{}}
		switch call.method {
		case "mail.get":
			return status, nil
		case "mail.apply":
			if taken {
				return map[string]any{"problem": "taken"}, nil
			}
			status["address"] = map[string]any{"name": call.params["name"], "email": call.params["name"].(string) + "@bots.lorca.app", "state": "active"}
			return map[string]any{"mail": status}, nil
		case "mail.release":
			return nil, errors.New("Relay unreachable")
		case "chats.mark_read", "bots.memory":
			return map[string]any{}, nil
		}
		return nil, errors.New("unexpected " + call.method)
	}
	// Before the CLI has asked the relay the pane is not offered; the snapshot's status brings it.
	if slices.Contains(store.ListedPanes(), model.PaneEmail) {
		t.Fatal("Email listed with no status")
	}
	store.HandleEvent("mail.changed", []byte(`{"available":true,"domain":"bots.lorca.app","address":null,"lead_bot_id":"bot-nova","bots":[]}`))
	f.step()
	f.m.showSettings(model.PaneEmail)
	f.wait(t, func() bool { return f.tt.HasText(L("Get an Address…")) })
	if len(f.transport.methodCalls("mail.get")) != 1 {
		t.Fatal("the pane did not ask the relay")
	}
	f.click(t, L("Get an Address…"))
	f.wait(t, func() bool { return f.tt.HasText(L("Get an Email Address")) })
	f.click(t, L("Your own name"))
	f.tt.Type("Egoist")
	f.tt.Key(0, ui.KeyEnter)
	f.wait(t, func() bool { return f.tt.HasText(L("That name is taken.")) })
	if calls := f.transport.methodCalls("mail.apply"); len(calls) != 1 || calls[0].params["name"] != "egoist" {
		t.Fatalf("apply %+v", calls)
	}
	taken = false
	f.tt.Key(0, ui.KeyEnter)
	f.wait(t, func() bool { return f.tt.HasText("egoist@bots.lorca.app") })
	if f.m.hasSheet() {
		t.Fatal("the sheet stayed up")
	}
	// A failure to give it up says so and keeps the address.
	f.click(t, L("Give Up Address…"))
	f.wait(t, func() bool { return f.tt.HasText(L("Give up %@?", "egoist@bots.lorca.app")) })
	f.tt.Key(0, ui.KeyEnter)
	f.wait(t, func() bool { return f.tt.HasText(L("Couldn't give up the address")) })
	if store.Mail.Address == nil {
		t.Fatal("the address went")
	}
}

func TestEmailSettingsInChinese(t *testing.T) {
	_, tt, _ := emailTester(t)
	l10n.Set("zh-Hans", "zh-CN")
	t.Cleanup(func() { l10n.Set("en", "en-US") })
	settle(tt)
	if !tt.HasText("邮箱地址") || !tt.HasText("放弃地址…") {
		t.Fatalf("not in Chinese: %q", tt.Texts())
	}
	renderTo(t, tt, "desktop-email-address-zh")
	if err := tt.Click("更改…"); err != nil {
		t.Fatal(err)
	}
	settleTransitions(tt)
	renderTo(t, tt, "desktop-email-sheet-change-zh")
}

func TestEmailInTheAccessSheet(t *testing.T) {
	_, _, tt := accessSheet(t, "bot-nova")
	if os.Getenv("LORCA_RENDER") != "" {
		tt.SetScale(2)
	}
	click(t, tt, L("%@ tools", "Email"))
	if !tt.HasText("Read and wait for email") || !tt.HasText("Send email") {
		t.Fatalf("Email's tools did not show: %q", tt.Texts())
	}
	tt.Move(2, 2)
	renderBoth(t, tt, "desktop-email-access")
}
