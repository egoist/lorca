package main

import (
	"strings"
	"testing"

	"github.com/egoist/lorca/desktop/l10n"
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// onboardingDemo is onboarding over the demo's store, as `LORCA_MOCK=1` shows it from the Debug
// menu's Show Onboarding, without a window of its own.
func onboardingDemo(t *testing.T) (*onboardingWindow, *ui.Tester) {
	t.Helper()
	demoWindow(t)
	o := newOnboardingWindow()
	app.onboarding = o
	tt := ui.NewTester(o.frame(o.view), 660, 560)
	// Steps show at once, without their fade, for the pictures.
	tt.SetPreferences(ui.Preferences{ReduceMotion: true, TextScale: 1})
	onboardingSettle(tt)
	return o, tt
}

// onboardingSettle runs what the store posted and draws until the step's fade is done.
func onboardingSettle(tt *ui.Tester) {
	for range 3 {
		runPosts()
		tt.Frame()
	}
}

// onboardingRender writes the step light and dark.
func onboardingRender(t *testing.T, tt *ui.Tester, name string) {
	t.Helper()
	tt.SetDark(false)
	tt.Frame()
	renderTo(t, tt, "onboarding-"+name+"-light")
	tt.SetDark(true)
	tt.Frame()
	renderTo(t, tt, "onboarding-"+name+"-dark")
	tt.SetDark(false)
	tt.Frame()
}

func onboardingClick(t *testing.T, tt *ui.Tester, text string) {
	t.Helper()
	if err := tt.Click(text); err != nil {
		t.Fatalf("click %q: %v (texts %q)", text, err, tt.Texts())
	}
	onboardingSettle(tt)
}

func onboardingExpect(t *testing.T, tt *ui.Tester, texts ...string) {
	t.Helper()
	for _, text := range texts {
		if !tt.HasText(text) {
			t.Fatalf("no %q in %q", text, tt.Texts())
		}
	}
}

// A new identity: the phrase, the first bot, and the last step.
func TestOnboardingCreate(t *testing.T) {
	o, tt := onboardingDemo(t)
	onboardingExpect(t, tt, appName(), L("Create a New Identity"), L("Restore from Backup Phrase"), L("Pair with Another Device"))
	onboardingRender(t, tt, "welcome")

	// Return presses Create a New Identity, the welcome's default button.
	tt.Key(0, ui.KeyEnter)
	onboardingSettle(tt)
	if o.step != onboardingCreate || len(o.phrase) != 12 {
		t.Fatalf("step %d, phrase %q", o.step, o.phrase)
	}
	onboardingExpect(t, tt, L("Your backup phrase"), "k4mq", "m5yq", "12", L("Copy Phrase"), L("I wrote this phrase down somewhere safe"))
	onboardingRender(t, tt, "create")

	// Continue waits for the check box.
	onboardingClick(t, tt, L("Continue"))
	if o.step != onboardingCreate {
		t.Fatalf("Continue went on before the phrase was saved")
	}
	onboardingClick(t, tt, L("I wrote this phrase down somewhere safe"))
	if !o.savedPhrase {
		t.Fatalf("check box did not check")
	}
	onboardingRender(t, tt, "create-saved")
	onboardingClick(t, tt, L("Continue"))
	if o.step != onboardingBot {
		t.Fatalf("step %d after Continue", o.step)
	}
	bot := o.firstBot()
	onboardingExpect(t, tt, L("Your first bot"), L("Name"), L("Description"), L("Provider"), "DeepSeek", L("API key"))
	if o.botName != bot.Name {
		t.Errorf("name %q, bot %q", o.botName, bot.Name)
	}
	onboardingRender(t, tt, "bot")

	// The key is needed before Continue.
	onboardingClick(t, tt, L("Continue"))
	if o.step != onboardingBot {
		t.Fatalf("Continue went on without a key")
	}
	// The field is in the control column, right of its label.
	label, _ := tt.Find(L("API key"))
	tt.ClickAt(label.X+label.W+14+100, label.Y+label.H/2)
	onboardingSettle(tt)
	tt.Type("sk-test")
	onboardingSettle(tt)
	if o.apiKey != "sk-test" {
		t.Fatalf("key %q", o.apiKey)
	}
	onboardingRender(t, tt, "bot-key")
	// Return in the key field continues.
	tt.Key(0, ui.KeyEnter)
	onboardingSettle(tt)
	if o.step != onboardingDone {
		t.Fatalf("step %d after Return", o.step)
	}
	onboardingExpect(t, tt, L("This computer is your first Device"), L("%@ is ready to talk to. Pair another Device any time from the File menu.", bot.Name), L("Open Lorca"))
	if !tt.Focused(L("Open Lorca")) {
		t.Errorf("Open Lorca is not focused")
	}
	onboardingRender(t, tt, "done")
}

// The bot step's name must not be empty, and Return in the description continues.
func TestOnboardingBotName(t *testing.T) {
	o, tt := onboardingDemo(t)
	o.show(onboardingBot)
	onboardingSettle(tt)
	o.botName = "  "
	o.apiKey = "sk-test"
	onboardingClick(t, tt, L("Continue"))
	onboardingExpect(t, tt, L("Give the bot a name."))
	onboardingRender(t, tt, "bot-no-name")
	o.botName = "Planner"
	onboardingClick(t, tt, L("Description"))
	tt.Key(0, ui.KeyEnter)
	onboardingSettle(tt)
	if o.step != onboardingDone || o.firstBot().Name != "Planner" {
		t.Fatalf("step %d, bot %q", o.step, o.firstBot().Name)
	}
}

// A restore, then the provider step with each kind of provider, and Skip for Now.
func TestOnboardingRestore(t *testing.T) {
	o, tt := onboardingDemo(t)
	onboardingClick(t, tt, L("Restore from Backup Phrase"))
	if o.step != onboardingRestore {
		t.Fatalf("step %d", o.step)
	}
	onboardingExpect(t, tt, L("Restore your identity"), L("Back"), L("Restore"))
	if !tt.Focused(L("Restore your identity")) {
		t.Errorf("the phrase field is not focused")
	}
	onboardingRender(t, tt, "restore")
	// Nothing typed, Restore waits.
	tt.Key(0, ui.KeyEnter)
	onboardingSettle(tt)
	if o.step != onboardingRestore {
		t.Fatalf("Restore ran without a phrase")
	}
	tt.Type("k4mq 7rth 2bnz wq5f j3xd pv82 ct6m 9hsa e7lw 4knr zb3u m5yq")
	tt.Key(0, ui.KeyEnter)
	onboardingSettle(tt)
	if o.step != onboardingProvider || o.origin != onboardingRestored {
		t.Fatalf("step %d, origin %d", o.step, o.origin)
	}
	onboardingExpect(t, tt, L("Connect a provider"), L("Skip for Now"), L("Continue"), L("The key is checked against %@ and shared with your paired Devices, encrypted.", "DeepSeek"))
	onboardingRender(t, tt, "provider")

	// A subscription signs in through the browser.
	onboardingClick(t, tt, "DeepSeek")
	if err := tt.ChooseMenuItem("ChatGPT"); err != nil {
		t.Fatalf("menu %q: %v", tt.Menu(), err)
	}
	onboardingSettle(tt)
	if o.providerKind != "chatgpt" {
		t.Fatalf("provider %q", o.providerKind)
	}
	onboardingExpect(t, tt, L("Account"), L("Sign in with %@", "ChatGPT"), L("Tokens from the sign-in are shared with your paired Devices, encrypted."))
	onboardingRender(t, tt, "provider-signin")

	// A preset is set up in its own sheet.
	onboardingClick(t, tt, "ChatGPT")
	if err := tt.ChooseMenuItem("OpenRouter"); err != nil {
		t.Fatalf("menu %q: %v", tt.Menu(), err)
	}
	onboardingSettle(tt)
	if !o.custom || o.customPreset == nil || o.customPreset.Name != "OpenRouter" {
		t.Fatalf("custom %v %+v", o.custom, o.customPreset)
	}
	onboardingExpect(t, tt, L("Server"), L("Set Up %@…", "OpenRouter"), L("Lorca checks the server, then shares it with your paired Devices, encrypted."))
	onboardingRender(t, tt, "provider-custom")

	onboardingClick(t, tt, L("Skip for Now"))
	if o.step != onboardingDone {
		t.Fatalf("step %d after Skip", o.step)
	}
	onboardingExpect(t, tt, L("Your identity is restored"), L("Your bots and chats sync to this computer, and it can run bots too. Pair another Device any time from the File menu."))
	onboardingRender(t, tt, "done-restored")
}

// Pairing: the code, the wait on the other Device with its spinner, Back, and a failure.
func TestOnboardingPair(t *testing.T) {
	o, tt := onboardingDemo(t)
	onboardingClick(t, tt, L("Pair with Another Device"))
	onboardingExpect(t, tt, L("Pair this computer"), L("The two Devices run a handshake; the relay only carries the ciphertext. This computer joins as a Runner."))
	onboardingRender(t, tt, "pair")

	// Waiting on the other Device: the field and Pair wait, Back cancels.
	o.joinText = "lorca://pair?relay=x"
	o.busy, o.joining = true, onboardingPairing
	o.status = &onboardingStatus{text: L("Waiting for the other Device to wrap the account key…")}
	onboardingSettle(tt)
	onboardingExpect(t, tt, L("Waiting for the other Device to wrap the account key…"))
	onboardingRender(t, tt, "pair-waiting")
	onboardingClick(t, tt, L("Pair"))
	if o.step != onboardingPair {
		t.Fatalf("Pair ran while it waited")
	}

	// A restore waits on its own: Back waits too.
	o.joining = onboardingRestoring
	onboardingSettle(tt)
	onboardingClick(t, tt, L("Back"))
	if o.step != onboardingPair {
		t.Fatalf("Back left a restore")
	}

	o.busy, o.joining = false, onboardingNotJoining
	o.status = &onboardingStatus{text: "Pairing cancelled", failed: true}
	onboardingSettle(tt)
	onboardingRender(t, tt, "pair-failed")
	onboardingClick(t, tt, L("Back"))
	if o.step != onboardingWelcome || o.status != nil {
		t.Fatalf("step %d, status %+v after Back", o.step, o.status)
	}
}

// Without the CLI, Create says how to start it.
func TestOnboardingOffline(t *testing.T) {
	o, tt := onboardingDemo(t)
	store.IsMock = false
	store.IsConnected = false
	onboardingClick(t, tt, L("Create a New Identity"))
	if o.step != onboardingWelcome || !o.hasSheet() {
		t.Fatalf("step %d, sheet %v", o.step, o.hasSheet())
	}
	onboardingExpect(t, tt, appName(), L("OK"))
	onboardingRender(t, tt, "offline")
	// Return answers the alert, not the page.
	tt.Key(0, ui.KeyEnter)
	onboardingSettle(tt)
	if o.hasSheet() || o.step != onboardingWelcome {
		t.Fatalf("sheet %v, step %d", o.hasSheet(), o.step)
	}
}

// An identity that arrives from elsewhere while onboarding waits moves on to the last step.
func TestOnboardingIdentityFromElsewhere(t *testing.T) {
	o, tt := onboardingDemo(t)
	store.IsIdentityDevice = false
	o.show(onboardingRestore)
	onboardingSettle(tt)
	yes := true
	store.HasIdentity = &yes
	o.storeChanged(model.Event{Kind: model.EventIdentityChanged})
	onboardingSettle(tt)
	if o.step != onboardingDone || o.origin != onboardingElsewhere {
		t.Fatalf("step %d, origin %d", o.step, o.origin)
	}
	onboardingExpect(t, tt, L("This computer is paired"))

	// Not while a step after the identity is up.
	o.show(onboardingProvider)
	o.storeChanged(model.Event{Kind: model.EventIdentityChanged})
	onboardingSettle(tt)
	if o.step != onboardingProvider {
		t.Fatalf("step %d", o.step)
	}
}

// The window controls of Windows and Linux sit over the top of the page.
func TestOnboardingTitleBar(t *testing.T) {
	o, tt := onboardingDemo(t)
	tt.SetTitleBar(ui.TitleBar{Height: 32, Right: 46})
	o.show(onboardingProvider)
	onboardingSettle(tt)
	r, ok := tt.Find(L("Connect a provider"))
	if !ok || r.Y < 32 {
		t.Fatalf("title at %+v", r)
	}
	onboardingRender(t, tt, "titlebar")
}

// The steps in Chinese, whose words run to other lengths. The bot step reads the CLI's default
// profile in the app's language.
func TestOnboardingChinese(t *testing.T) {
	o, tt := onboardingDemo(t)
	l10n.Set("zh-Hans", "zh-CN")
	defer l10n.Set("en", "en-US")
	tt.Frame()
	onboardingRender(t, tt, "zh-welcome")
	o.phrase = model.MockBackupPhrase
	o.show(onboardingCreate)
	onboardingSettle(tt)
	onboardingRender(t, tt, "zh-create")
	bot := o.firstBot()
	bot.Name, bot.Description = "Chef", onboardingDefaultDescription
	o.show(onboardingBot)
	onboardingSettle(tt)
	if o.botName != "幕僚长" || o.botDescription == onboardingDefaultDescription {
		t.Errorf("name %q, description %q", o.botName, o.botDescription)
	}
	onboardingRender(t, tt, "zh-bot")
	o.show(onboardingRestore)
	onboardingSettle(tt)
	onboardingRender(t, tt, "zh-restore")
	o.origin = onboardingCreated
	o.show(onboardingDone)
	onboardingSettle(tt)
	onboardingRender(t, tt, "zh-done")
}

// A drag runs across the phrase's words, and Copy takes them without their numbers.
func TestOnboardingPhraseSelects(t *testing.T) {
	o, tt := onboardingDemo(t)
	tt.Key(0, ui.KeyEnter)
	onboardingSettle(tt)
	if o.step != onboardingCreate {
		t.Fatalf("step %d", o.step)
	}
	first, ok := tt.Find(o.phrase[0])
	last, ok2 := tt.Find(o.phrase[5])
	if !ok || !ok2 {
		t.Fatalf("no words: %q", tt.Texts())
	}
	tt.Press(first.X+1, first.Y+first.H/2)
	tt.Move(last.X+last.W/2, last.Y+last.H/2)
	tt.Release(last.X+last.W-1, last.Y+last.H/2)
	tt.Key(ui.Cmd, ui.KeyC)
	if got, want := tt.Clipboard(), strings.Join(o.phrase[:6], "\n"); got != want {
		t.Errorf("copied %q, want %q", got, want)
	}
}
