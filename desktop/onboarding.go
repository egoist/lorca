package main

import (
	"strconv"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Onboarding, after the macOS app's OnboardingWindowController: a new identity with its backup
// phrase, a restore from that phrase, or pairing with a Device that has the account; then the first
// bot and a provider for it. Its own window, which the main window replaces when it finishes.

// onboardingStep is one page of onboarding.
type onboardingStep int

const (
	onboardingWelcome onboardingStep = iota
	onboardingCreate
	onboardingRestore
	onboardingPair
	onboardingBot
	onboardingProvider
	onboardingDone
)

// onboardingOrigin is how this computer came by its identity, which the last step words.
type onboardingOrigin int

const (
	// onboardingElsewhere is an identity that arrived from elsewhere (the CLI, or a paired
	// Device) while onboarding waited.
	onboardingElsewhere onboardingOrigin = iota
	onboardingCreated
	onboardingRestored
	onboardingPaired
)

// onboardingJoining is a restore or a pairing in flight. Back cancels a pairing, which waits on
// the other Device; a restore, and the sync after either, end on their own.
type onboardingJoining int

const (
	onboardingNotJoining onboardingJoining = iota
	onboardingRestoring
	onboardingPairing
	onboardingSyncing
)

// onboardingDefaultDescription is the first bot's description as the CLI writes it, which the bot
// step shows in the app's language.
const onboardingDefaultDescription = "Chief of staff. Plans the work and delegates each task to the right teammate, proposing a new one when none fits. Does hands-on work when necessary."

// onboardingStatus is the line under a step's field: what is under way, or what failed.
type onboardingStatus struct {
	text   string
	failed bool
}

// onboardingButton is one of a step's buttons at the bottom.
type onboardingButton struct {
	title    string
	disabled bool
	run      func()
}

// onboardingWindow is onboarding's window and where onboarding stands: the step, the phrase the
// user was shown, and what they typed and picked, which a language change keeps.
type onboardingWindow struct {
	appWindow
	step        onboardingStep
	phrase      []string
	savedPhrase bool
	status      *onboardingStatus
	// providerKind is the built-in provider picked, unless custom is set.
	providerKind model.ProviderKind
	// custom is a custom provider picked instead of a built-in one: customPreset, or none for
	// Other Server. It is set up in its own sheet, and the first bot moves to it once it is saved.
	custom       bool
	customPreset *model.CustomPreset
	apiKey       string
	// joinText is the phrase or the pairing code typed on the restore or the pair step.
	joinText string
	// botName and botDescription are the first bot's, as the bot step edits them.
	botName        string
	botDescription string
	origin         onboardingOrigin
	joining        onboardingJoining
	busy           bool
	// signingIn is a subscription sign-in waiting on the browser.
	signingIn bool
}

// onboardingStore is the store onboarding listens to: once, for whichever onboarding is up.
var onboardingStore *model.Store

func newOnboardingWindow() *onboardingWindow {
	if onboardingStore != store {
		onboardingStore = store
		store.Subscribe(func(event model.Event) {
			if app.onboarding != nil {
				app.onboarding.storeChanged(event)
			}
		})
	}
	return &onboardingWindow{providerKind: "deepseek"}
}

// storeChanged moves on to the last step when the identity arrives from elsewhere (the CLI, or a
// paired Device) while onboarding waits for the user to choose.
func (o *onboardingWindow) storeChanged(event model.Event) {
	if event.Kind != model.EventIdentityChanged || store.HasIdentity == nil || !*store.HasIdentity || o.busy {
		return
	}
	if o.step != onboardingWelcome && o.step != onboardingRestore && o.step != onboardingPair {
		return
	}
	o.origin = onboardingElsewhere
	o.show(onboardingDone)
}

// firstBot is the bot the CLI created with the identity, or the first bot in the roster.
func (o *onboardingWindow) firstBot() *model.Bot {
	for _, bot := range store.Bots {
		if bot.Name == "Chef" {
			return bot
		}
	}
	if len(store.Bots) > 0 {
		return store.Bots[0]
	}
	return nil
}

// show shows a step, without the last one's status or key. The bot step starts from the first bot's
// profile: the CLI's default one in the app's language, a name or description the user saved as
// it is.
func (o *onboardingWindow) show(next onboardingStep) {
	o.status = nil
	o.apiKey = ""
	o.joinText = ""
	if next == onboardingBot {
		bot := o.firstBot()
		o.botName = Lc("Chef", "first bot name")
		o.botDescription = L("Chief of staff. Plans the work and delegates each task to the right teammate, proposing a new one when none fits. Does hands-on work when necessary.")
		if bot != nil && bot.Name != "Chef" {
			o.botName = bot.Name
		}
		if bot != nil && bot.Description != onboardingDefaultDescription {
			o.botDescription = bot.Description
		}
	}
	o.step = next
	o.invalidate()
}

func (o *onboardingWindow) presentError(message string) {
	o.showAlert(alertOptions{Message: appName(), Informative: message, Style: alertWarning, Buttons: []alertButton{{Title: L("OK")}}}, nil)
}

// MARK: - Actions

func (o *onboardingWindow) createIdentity() {
	if o.busy {
		return
	}
	if store.IsMock {
		o.phrase = model.MockBackupPhrase
		o.origin = onboardingCreated
		o.show(onboardingCreate)
		return
	}
	if !store.IsConnected {
		o.presentError(strings.Replace(L("The Lorca CLI is not running. Start it with `lorca serve` and try again."), "lorca serve", cliCommand(), 1))
		return
	}
	o.busy = true
	store.CreateIdentity(func(phrase []string, err error) {
		o.busy = false
		if err != nil {
			o.presentError(model.ErrorText(err))
			return
		}
		o.phrase = phrase
		o.origin = onboardingCreated
		o.show(onboardingCreate)
	})
}

// join restores from the phrase or pairs with the code typed. A computer that joined an account
// gets its credentials with its first pull from the relay, so the provider step shows only when
// the account has none.
func (o *onboardingWindow) join(joined onboardingOrigin, waiting string, run func(text string, done func(error))) {
	text := strings.TrimSpace(o.joinText)
	if o.busy || text == "" {
		return
	}
	if store.IsMock {
		o.origin = joined
		o.show(onboardingProvider)
		return
	}
	o.busy = true
	o.joining = onboardingRestoring
	if joined == onboardingPaired {
		o.joining = onboardingPairing
	}
	o.status = &onboardingStatus{text: waiting}
	run(text, func(err error) {
		if err != nil {
			// Back cancelled the pairing and left the step.
			if o.step == onboardingRestore || o.step == onboardingPair {
				o.status = &onboardingStatus{text: model.ErrorText(err), failed: true}
			}
			o.busy = false
			o.joining = onboardingNotJoining
			return
		}
		o.origin = joined
		o.joining = onboardingSyncing
		o.status = &onboardingStatus{text: L("Syncing the account from the relay…")}
		store.AccountHasProvider(func(has bool) {
			o.busy = false
			o.joining = onboardingNotJoining
			if has {
				o.show(onboardingDone)
			} else {
				o.show(onboardingProvider)
			}
		})
	})
}

// leaveJoin is Back from a restore or a pairing; from a pairing that waits on the other Device,
// it stops the wait in the CLI too.
func (o *onboardingWindow) leaveJoin() {
	if o.joining == onboardingPairing {
		store.AbortPairing()
	}
	o.show(onboardingWelcome)
}

func (o *onboardingWindow) connectProvider() {
	if o.busy {
		return
	}
	if o.custom {
		o.setUpCustomProvider()
		return
	}
	if store.IsMock {
		o.show(onboardingDone)
		return
	}
	kind := o.providerKind
	name := model.ProviderName(kind, store.Providers)
	key := strings.TrimSpace(o.apiKey)
	if model.UsesAPIKey(kind) && key == "" {
		text := L("Paste a %@ API key to continue.", name)
		if kind == "anthropic" {
			text = L("Paste an %@ API key to continue.", name)
		}
		o.status = &onboardingStatus{text: text, failed: true}
		return
	}
	o.busy = true
	done := func(err error) {
		o.busy = false
		o.signingIn = false
		// Skipped while the browser waited, onboarding has moved on; the bot step and the provider
		// step both show the rows the error belongs under.
		if o.step == onboardingDone {
			return
		}
		if err != nil {
			o.status = &onboardingStatus{text: model.ErrorText(err), failed: true}
			return
		}
		o.show(onboardingDone)
	}
	if model.UsesAPIKey(kind) {
		o.status = &onboardingStatus{text: L("Checking the key with %@…", name)}
		store.ConnectAPIKey(kind, key, "", done)
	} else {
		o.status = &onboardingStatus{text: L("Waiting for the browser…")}
		o.signingIn = true
		store.ConnectSignIn(kind, done)
	}
}

// setUpCustomProvider opens the custom provider sheet over onboarding; once the provider is saved,
// the first bot runs with it and onboarding is done. Cancel leaves onboarding where it was.
func (o *onboardingWindow) setUpCustomProvider() {
	o.presentCustomProvider("", o.customPreset, func(kind model.ProviderKind) {
		if o.step == onboardingDone {
			return
		}
		if bot := o.firstBot(); bot != nil && bot.Provider != kind {
			store.SetBotRuntime(bot.ID, kind, bot.Model, bot.Thinking)
		}
		o.show(onboardingDone)
	})
}

// skipProvider leaves the provider for later, a sign-in still waiting on the browser included, so
// finishing it there afterwards connects nothing.
func (o *onboardingWindow) skipProvider() {
	if o.signingIn {
		store.CancelSignIn()
	}
	o.show(onboardingDone)
}

// saveFirstBot names the first bot and has it run with the provider chosen here, not the CLI's
// default (a custom one is set once its sheet saves it), then connects the provider.
func (o *onboardingWindow) saveFirstBot() {
	bot := o.firstBot()
	if o.busy || bot == nil {
		o.show(onboardingProvider)
		return
	}
	name, description := strings.TrimSpace(o.botName), strings.TrimSpace(o.botDescription)
	if name == "" {
		o.status = &onboardingStatus{text: L("Give the bot a name."), failed: true}
		return
	}
	provider := o.providerKind
	if o.custom {
		provider = bot.Provider
	}
	if name != bot.Name || description != bot.Description {
		store.UpdateBotProfile(bot.ID, name, &description)
	}
	if provider != bot.Provider {
		store.SetBotRuntime(bot.ID, provider, bot.Model, bot.Thinking)
	}
	o.connectProvider()
}

// pickProvider is a choice in the provider menu: a built-in provider, a preset, or Other Server.
// The note under the key is the new provider's, not the last one's error.
func (o *onboardingWindow) pickProvider(value string) {
	o.apiKey = ""
	o.status = nil
	switch {
	case value == "other":
		o.custom, o.customPreset = true, nil
	case strings.HasPrefix(value, "preset:"):
		o.custom, o.customPreset = true, nil
		for i := range model.CustomPresets {
			if "preset:"+model.CustomPresets[i].Name == value {
				o.customPreset = &model.CustomPresets[i]
			}
		}
	default:
		o.custom, o.providerKind = false, value
	}
}

// credentialButton is the Continue button of a step with a credential: signing in says so, a key
// must be typed, and a custom provider is set up next.
func (o *onboardingWindow) credentialButton(run func()) onboardingButton {
	if o.custom {
		title := L("Set Up…")
		if o.customPreset != nil {
			title = L("Set Up %@…", o.customPreset.Name)
		}
		return onboardingButton{title: title, run: run}
	}
	if model.UsesAPIKey(o.providerKind) {
		return onboardingButton{title: L("Continue"), disabled: strings.TrimSpace(o.apiKey) == "", run: run}
	}
	return onboardingButton{title: L("Sign in with %@", model.ProviderName(o.providerKind, store.Providers)), run: run}
}

// MARK: - View

// view is the whole window, its title bar hidden: white in the light appearance, as macOS 26 draws
// a window's background, and it drags the window by its background, as the Mac app's does. The
// content starts below the window controls.
func (o *onboardingWindow) view(c *ui.Context) {
	p := colors(c)
	o.setTitle(appName())
	background := p.Window
	if !c.Theme().Dark {
		background = p.White
	}
	// Return presses the step's default button wherever the keyboard is, as a window's default
	// button answers Return, unless what has the keyboard takes it: a button, or a pop-up button.
	var defaultButton *onboardingButton
	ui.Column(c).Fill().Background(background).Padding(max(28, c.TitleBar().Height), 44, 36, 44).DragWindow().Children(func() {
		page := ui.Column(c.Key(int(o.step))).Grow(1).MinHeight(0).
			Transition(ui.ElementTransition{Enter: &ui.Motion{}, Duration: 220 * time.Millisecond})
		page.Children(func() {
			switch o.step {
			case onboardingWelcome:
				defaultButton = o.welcome(c)
			case onboardingCreate:
				defaultButton = o.createStep(c)
			case onboardingRestore:
				note := L("Restoring never contacts a login server. The relay only answers a signature challenge.")
				if store.RelayURL == "" {
					note = L("Restoring unwraps the account key from the relay. Set a relay URL in Settings › Advanced first.")
				}
				defaultButton = o.joinStep(c, onboardingJoinStep{
					title:       L("Restore your identity"),
					subtitle:    L("Paste the twelve groups from your backup phrase. Everything else is re-derived and unwrapped from the relay."),
					placeholder: "k4mq 7rth 2bnz …",
					note:        note,
					action:      L("Restore"),
					canGoBack:   o.joining == onboardingNotJoining,
					submit: func() {
						o.join(onboardingRestored, L("Re-deriving keys and unwrapping the account key…"), store.RestoreIdentity)
					},
				})
			case onboardingPair:
				defaultButton = o.joinStep(c, onboardingJoinStep{
					title:       L("Pair this computer"),
					subtitle:    L("On a computer that already has your identity, choose File › Pair a Device in the desktop app or run lorca pair in a terminal, then paste the code here."),
					placeholder: "lorca://pair?relay=…",
					note:        L("The two Devices run a handshake; the relay only carries the ciphertext. This computer joins as a Runner."),
					action:      L("Pair"),
					canGoBack:   o.joining == onboardingNotJoining || o.joining == onboardingPairing,
					submit: func() {
						o.join(onboardingPaired, L("Waiting for the other Device to wrap the account key…"), store.AcceptPairing)
					},
				})
			case onboardingBot:
				defaultButton = o.botStep(c)
			case onboardingProvider:
				defaultButton = o.providerStep(c)
			case onboardingDone:
				defaultButton = o.doneStep(c)
			}
		})
	})
	if defaultButton != nil && !defaultButton.disabled && !o.hasSheet() && c.Shortcut(0, ui.KeyEnter) {
		defaultButton.run()
	}
}

// centered is a step of a column in the middle of the window: the welcome and the last step.
func (o *onboardingWindow) centered(c *ui.Context, content func()) {
	ui.Column(c).Grow(1).Center().Children(func() {
		ui.Column(c).AlignItems(ui.Center).Gap(14).MaxWidthPercent(100).Children(content)
	})
}

// onboardingLede is the centered paragraph under a centered step's title.
func onboardingLede(c *ui.Context, text string) {
	ui.Text(c, text).Width(420).MaxWidthPercent(100).FontSize(13).LineHeight(1.45).TextColor(colors(c).Label2).TextAlign(ui.Center)
}

func (o *onboardingWindow) welcome(c *ui.Context) *onboardingButton {
	create := onboardingButton{title: L("Create a New Identity"), run: o.createIdentity}
	o.centered(c, func() {
		ui.Box(c).Size(96, 96).Margin(0, 0, 6, 0).Children(func() {
			if icon := appIcon(); icon != nil {
				ui.Image(c, icon).Size(96, 96)
			}
		})
		ui.Text(c, appName()).FontSize(30).FontWeight(700)
		onboardingLede(c, L("Bots that run on computers you own. Your identity is a key pair on this computer — no account, no server that can read your chats."))
		ui.Column(c).Gap(10).Margin(16, 0, 0, 0).Children(func() {
			if pushButton(c, create.title, pushOptions{Kind: buttonPrimary, Large: true}).Width(280).Clicked() {
				create.run()
			}
			if pushButton(c, L("Restore from Backup Phrase"), pushOptions{Large: true}).Width(280).Clicked() {
				o.show(onboardingRestore)
			}
			if pushButton(c, L("Pair with Another Device"), pushOptions{Large: true}).Width(280).Clicked() {
				o.show(onboardingPair)
			}
		})
	})
	return &create
}

// onboardingStepOptions describe a step of the usual layout.
type onboardingStepOptions struct {
	title    string
	subtitle string
	back     *onboardingButton
	next     onboardingButton
	// busy turns a spinner beside the buttons.
	busy bool
}

// stepLayout is a step: its title and what it is for at the top, its body, and Back and Continue
// at the bottom.
func (o *onboardingWindow) stepLayout(c *ui.Context, s onboardingStepOptions, body func()) {
	p := colors(c)
	ui.Column(c).Gap(6).Padding(12, 0, 0, 0).Children(func() {
		ui.Text(c, s.title).FontSize(22).FontWeight(600)
		ui.Text(c, s.subtitle).FontSize(12.5).LineHeight(1.45).TextColor(p.Label2)
	})
	ui.Column(c).Grow(1).MinHeight(0).Margin(26, 0, 0, 0).AlignItems(ui.Start).Children(body)
	ui.Row(c).Justify(ui.End).Gap(10).Margin(20, 0, 0, 0).Children(func() {
		if s.busy {
			spinner(c, 16).Margin(0, 2, 0, 0)
		}
		if s.back != nil {
			if pushButton(c, s.back.title, pushOptions{Large: true, Disabled: s.back.disabled}).Clicked() {
				s.back.run()
			}
		}
		if pushButton(c, s.next.title, pushOptions{Kind: buttonPrimary, Large: true, Disabled: s.next.disabled}).MinWidth(120).Clicked() {
			s.next.run()
		}
	})
}

func (o *onboardingWindow) createStep(c *ui.Context) *onboardingButton {
	p := colors(c)
	next := onboardingButton{title: L("Continue"), disabled: !o.savedPhrase, run: func() {
		if o.firstBot() != nil {
			o.show(onboardingBot)
		} else {
			o.show(onboardingProvider)
		}
	}}
	o.stepLayout(c, onboardingStepOptions{
		title:    L("Your backup phrase"),
		subtitle: L("This phrase is your master secret. It re-derives every key and unwraps everything on the relay. Write it down — nobody can reset it for you."),
		next:     next,
	}, func() {
		ui.Column(c).AlignItems(ui.Start).Gap(16).Children(func() {
			// The words are one selection, which a drag runs across instead of moving the window;
			// their numbers stay out of it.
			ui.Grid(c).Selectable().ColumnTracks(ui.Fixed(128), ui.Fixed(128), ui.Fixed(128), ui.Fixed(128)).Gap(8).Children(func() {
				for i, word := range o.phrase {
					ui.Row(c).Height(34).Gap(2).Padding(0, 0, 0, 9).Radius(7).Background(p.Code).Children(func() {
						ui.Text(c, strconv.Itoa(i+1)).Width(16).FontSize(10).TextColor(p.Label3).FontFeatures("tnum").SingleLine().Unselectable()
						ui.Text(c, word).Font(monoFont).FontSize(13).FontWeight(500).SingleLine()
					})
				}
			})
			copyButton(c, strings.Join(o.phrase, " "), copyOptions{Title: L("Copy Phrase"), Bordered: true})
			ui.Checkbox(c, &o.savedPhrase, L("I wrote this phrase down somewhere safe")).Gap(7).FontSize(13)
		})
	})
	return &next
}

// onboardingJoinStep describes the restore and the pair step: a field for the phrase or the code,
// and a note under it that the status replaces.
type onboardingJoinStep struct {
	title       string
	subtitle    string
	placeholder string
	note        string
	action      string
	canGoBack   bool
	submit      func()
}

// joinStep is the restore or the pair step. While the restore or the pairing is under way, the
// field and the action wait beside the spinner.
func (o *onboardingWindow) joinStep(c *ui.Context, s onboardingJoinStep) *onboardingButton {
	p := colors(c)
	busy := o.joining != onboardingNotJoining
	next := onboardingButton{title: s.action, disabled: busy, run: s.submit}
	o.stepLayout(c, onboardingStepOptions{
		title:    s.title,
		subtitle: s.subtitle,
		back:     &onboardingButton{title: L("Back"), disabled: !s.canGoBack, run: o.leaveJoin},
		next:     next,
		busy:     busy,
	}, func() {
		ui.Column(c).Gap(10).Width(560).MaxWidthPercent(100).Children(func() {
			ui.Box(c.Key("field")).Children(func() {
				if textField(c, &o.joinText, fieldOptions{Placeholder: s.placeholder, Mono: true, Disabled: busy, AutoFocus: true, Label: s.title}).Submitted() && !busy {
					s.submit()
				}
			})
			note, color := s.note, p.Label3
			if o.status != nil {
				note, color = o.status.text, o.statusColor(c)
			}
			ui.Text(c, note).FontSize(textCaption).LineHeight(1.4).TextColor(color)
		})
	})
	return &next
}

// statusColor is the status line's: red for what failed, secondary for what is under way.
func (o *onboardingWindow) statusColor(c *ui.Context) ui.Color {
	if o.status != nil && o.status.failed {
		return colors(c).Red
	}
	return colors(c).Label2
}

// card is the rounded fill around a step's form.
func (o *onboardingWindow) card(c *ui.Context, content func()) {
	ui.Column(c).Padding(18).Radius(12).Background(colors(c).Code).Children(content)
}

// formRow is a row of a step's form: the label column on the left, the control on the right,
// every control the same width. A label `top` sits by the control's first line.
func (o *onboardingWindow) formRow(c *ui.Context, label string, top bool, control func()) {
	p := colors(c)
	l := ui.Text(c, label).FontSize(12).FontWeight(500).TextColor(p.Label2).TextAlign(ui.End)
	if top {
		l.AlignSelf(ui.Start).Padding(6, 0, 0, 0)
	}
	ui.Row(c).MinWidth(0).Children(control)
}

// form lays out the rows built in `rows`.
func (o *onboardingWindow) form(c *ui.Context, rows func()) {
	ui.Grid(c).ColumnTracks(ui.Fixed(76), ui.Fixed(400)).GapX(14).GapY(12).AlignItems(ui.Center).Children(rows)
}

func (o *onboardingWindow) botStep(c *ui.Context) *onboardingButton {
	next := o.credentialButton(o.saveFirstBot)
	o.stepLayout(c, onboardingStepOptions{
		title:    L("Your first bot"),
		subtitle: L("It runs on this computer, plans your work, and builds the rest of the team when you ask. Give it a name and the credentials it runs with."),
		next:     next,
	}, func() {
		o.card(c, func() {
			o.form(c, func() {
				o.formRow(c, "", false, func() {
					content := avatarContent{Kind: avatarBot, SymbolName: "sparkles", Accent: "indigo"}
					if bot := o.firstBot(); bot != nil {
						content = botAvatar(bot)
					}
					avatar(c, content, 40, false)
				})
				o.formRow(c, L("Name"), false, func() {
					ui.Box(c.Key("name")).Grow(1).Children(func() {
						if textField(c, &o.botName, fieldOptions{Placeholder: L("Name"), Label: L("Name")}).Submitted() && !next.disabled {
							next.run()
						}
					})
				})
				o.formRow(c, L("Description"), true, func() {
					ui.Box(c.Key("description")).Grow(1).Children(func() {
						field := textArea(c, &o.botDescription, 3, fieldOptions{Placeholder: L("What it does and how it should work"), Label: L("Description")})
						composing := field.Composing()
						field.HandleInput(func(ev ui.InputEvent) bool {
							// Return continues, as in a wrapping text field; Alt or Shift with it
							// starts a line.
							if ev.Kind != ui.InputKeyDown || ev.Key != ui.KeyEnter || ev.Mods != 0 || composing {
								return false
							}
							if !next.disabled {
								next.run()
							}
							return true
						})
					})
				})
				o.providerRows(c, next)
			})
		})
	})
	return &next
}

func (o *onboardingWindow) providerStep(c *ui.Context) *onboardingButton {
	next := o.credentialButton(o.connectProvider)
	o.stepLayout(c, onboardingStepOptions{
		title:    L("Connect a provider"),
		subtitle: L("Credentials belong to your account: your bots use them on every Runner you pair. They sync encrypted with your account key; the relay cannot read them."),
		back:     &onboardingButton{title: L("Skip for Now"), run: o.skipProvider},
		next:     next,
	}, func() {
		o.card(c, func() {
			o.form(c, func() { o.providerRows(c, next) })
		})
	})
	return &next
}

// providerRows are the provider picker and the credential for the chosen provider: a key field,
// what signing in does, or that a custom provider's server is set up next. Return in the key field
// presses `next`.
func (o *onboardingWindow) providerRows(c *ui.Context, next onboardingButton) {
	p := colors(c)
	// The built-in providers, then any other server: for an account whose models run on a
	// gateway or its own computers.
	var options []popUpOption
	for _, kind := range model.ProviderKinds {
		options = append(options, popUpOption{Value: kind, Label: model.ProviderName(kind, store.Providers)})
	}
	for i, preset := range model.CustomPresets {
		options = append(options, popUpOption{Value: "preset:" + preset.Name, Label: preset.Name, Separated: i == 0})
	}
	options = append(options, popUpOption{Value: "other", Label: L("Other Server…")})
	value := o.providerKind
	if o.custom {
		value = "other"
		if o.customPreset != nil {
			value = "preset:" + o.customPreset.Name
		}
	}
	kind, apiKey := o.providerKind, model.UsesAPIKey(o.providerKind)
	name := model.ProviderName(kind, store.Providers)
	o.formRow(c, L("Provider"), false, func() {
		if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: value, Width: 400, Label: L("Provider")}); changed {
			o.pickProvider(picked)
		}
	})
	label := L("Account")
	switch {
	case o.custom:
		label = L("Server")
	case apiKey:
		label = L("API key")
	}
	o.formRow(c, label, o.custom || !apiKey, func() {
		switch {
		case o.custom:
			ui.Text(c, L("Any server that speaks OpenAI’s or Anthropic’s API, such as a gateway or a model server on your network. Set up its address, key, and models next.")).
				FontSize(12).LineHeight(1.4).TextColor(p.Label2)
		case apiKey:
			ui.Box(c.Key("key:" + kind)).Grow(1).Children(func() {
				if textField(c, &o.apiKey, fieldOptions{Placeholder: model.KeyPlaceholder(kind), Secure: true, Mono: true, Label: L("API key")}).Submitted() && !next.disabled {
					next.run()
				}
			})
		default:
			ui.Text(c, L("Your browser opens a %@ sign-in when you continue. %@", name, model.SignInRequirement(kind))).FontSize(12).LineHeight(1.4).TextColor(p.Label2)
		}
	})
	note, color := L("Tokens from the sign-in are shared with your paired Devices, encrypted."), p.Label3
	switch {
	case o.status != nil:
		note, color = o.status.text, o.statusColor(c)
	case o.custom:
		note = L("Lorca checks the server, then shares it with your paired Devices, encrypted.")
	case apiKey:
		note = L("The key is checked against %@ and shared with your paired Devices, encrypted.", name)
	}
	o.formRow(c, "", false, func() {
		ui.Text(c, note).FontSize(textCaption).LineHeight(1.4).TextColor(color)
	})
}

func (o *onboardingWindow) doneStep(c *ui.Context) *onboardingButton {
	p := colors(c)
	origin := o.origin
	if origin == onboardingElsewhere {
		// One that arrived from elsewhere was paired, unless this computer holds the identity.
		origin = onboardingPaired
		if store.IsIdentityDevice {
			origin = onboardingCreated
		}
	}
	title := L("This computer is paired")
	lede := L("Your bots and chats sync to this computer, and it can run bots too. Pair another Device any time from the File menu.")
	switch origin {
	case onboardingCreated:
		title = L("This computer is your first Device")
		lede = L("Bots you create here run on this computer with your account's provider credentials. Pair another Device any time from the File menu.")
		if bot := o.firstBot(); bot != nil {
			lede = L("%@ is ready to talk to. Pair another Device any time from the File menu.", bot.Name)
		}
	case onboardingRestored:
		title = L("Your identity is restored")
	}
	open := onboardingButton{title: L("Open Lorca"), run: app.endOnboarding}
	o.centered(c, func() {
		ui.Row(c).Margin(0, 0, 6, 0).TextColor(p.Green).Children(func() { symbol(c, "checkmark.circle.fill", 56, 1.6) })
		ui.Text(c, title).FontSize(22).FontWeight(600).TextAlign(ui.Center)
		onboardingLede(c, lede)
		if pushButton(c, open.title, pushOptions{Kind: buttonPrimary, Large: true}).Width(200).Margin(12, 0, 0, 0).AutoFocus().Clicked() {
			open.run()
		}
	})
	return &open
}
