package main

import (
	"math"
	"slices"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The provider sheets, after the macOS app's ConnectProviderViewController,
// CustomProviderViewController, and APIKeyField. A built-in provider connects with a pasted key, or
// signs in through the browser, driven by the CLI. A custom provider is any server that speaks
// OpenAI's Chat Completions or Responses API, or Anthropic's Messages API, or a decision API
// (System One, OpenAI Decisions) whose models Auto-review can run: the sheet loads the models the
// server lists as the user fills it in, and the user picks the ones to offer, adding any the server
// does not list. Either credential reaches every paired Device encrypted with the account key.

// providerFetching is a sheet's saved key being fetched before it opens, so a second click waits.
var providerFetching bool

// MARK: - Shared

// providerStatus is the line under a sheet's form: what it is doing, with a spinner, or how it went.
type providerStatus struct {
	text     string
	tone     model.Tone
	spinning bool
}

// providerStatusLine draws a sheet's status line, when it has one.
func providerStatusLine(c *ui.Context, status *providerStatus) {
	if status == nil {
		return
	}
	p := colors(c)
	ui.Row(c).Gap(8).Children(func() {
		if status.spinning {
			spinner(c, 14)
		}
		ui.Text(c, status.text).Grow(1).Shrink(1).MinWidth(0).FontSize(textCaption).LineHeight(1.4).TextColor(p.tone(status.tone)).Selectable()
	})
}

// providerNote is a note under a field, or a sheet's note: small, in the tertiary color.
func providerNote(c *ui.Context, text string, tint *ui.Color) ui.Element {
	p := colors(c)
	color := p.Label3
	if tint != nil {
		color = *tint
	}
	return ui.Text(c, text).FontSize(textCaption).LineHeight(1.4).TextColor(color)
}

// mcpFormLabelWidth is a form's label column: as wide as the widest of its labels, whichever show,
// as the Mac sheets' fixed label width is, so switching fields moves none.
func mcpFormLabelWidth(c *ui.Context, labels ...string) float32 {
	width := float32(0)
	for _, label := range labels {
		w, _ := c.MeasureText(0, ui.Span{Text: label, Size: 12})
		width = max(width, w)
	}
	return float32(math.Ceil(float64(width))) + 1
}

// providerFormRow is a label in the form's label column and its control, which fills the rest.
func providerFormRow(c *ui.Context, labelWidth float32, label string, control func()) ui.Element {
	p := colors(c)
	return ui.Row(c).Gap(10).Children(func() {
		ui.Text(c, label).Width(labelWidth).Shrink(0).FontSize(12).TextColor(p.Label2).SingleLine()
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(control)
	})
}

// providerFormNote is a note in the form's control column, tucked under the field above it.
func providerFormNote(c *ui.Context, labelWidth float32, text string, tint *ui.Color) {
	ui.Row(c).Gap(10).Margin(-4, 0, 0, 0).AlignItems(ui.Start).Children(func() {
		ui.Box(c).Width(labelWidth).Shrink(0)
		providerNote(c, text, tint).Grow(1).Shrink(1).MinWidth(0)
	})
}

// MARK: - The API key field

type providerKeyState struct {
	revealed bool
	// focused and from…to are the field's focus and selection before the eye was pressed, which it
	// gets back once the key shows or hides.
	focused  bool
	from, to int
	refocus  bool
	// shown is the field having been on screen: only its first appearance takes the keyboard.
	shown bool
}

// providerKeyField is a provider's API key, after the macOS app's APIKeyField: masked by default,
// with an eye inside the field that shows or hides the key and keeps the selection while it
// switches. `autoFocus` makes it the field a sheet starts in.
func providerKeyField(c *ui.Context, value *string, placeholder string, disabled, autoFocus bool) ui.Element {
	var field ui.Element
	box := ui.Box(c).MinWidth(0)
	st := ui.Local(box, "key", func() providerKeyState { return providerKeyState{} })
	box.Children(func() {
		field = textField(c, value, fieldOptions{Placeholder: placeholder, Secure: !st.revealed, Mono: true, Disabled: disabled, AutoFocus: autoFocus && !st.shown, Label: L("API key")}).
			Padding(4, 32, 4, 8)
		st.shown = true
		if st.refocus {
			st.refocus = false
			field.Focus().SetTextSelection(st.from, st.to)
		}
		symbolName, tooltip := "eye", L("Show API key")
		if st.revealed {
			symbolName, tooltip = "eye.slash", L("Hide API key")
		}
		eye := hoverButton(c, hoverButtonOptions{Symbol: symbolName, Size: 14, Tooltip: tooltip, Disabled: disabled}).
			Size(24, 20).MinWidth(24).Attach(ui.AnchorRight, ui.AnchorRight).Right(3)
		switch {
		case eye.Clicked():
			st.revealed = !st.revealed
			st.refocus = st.focused
		case !eye.Pressed():
			st.focused = field.Focused()
			if st.focused {
				st.from, st.to = field.TextSelection()
			}
		}
	})
	return field
}

// MARK: - Add Provider…

// addProviderMenu fills Add Provider…'s menu: the servers people often add, hosted, on the user's
// network, and decision APIs, then any other. A preset the account has a provider for, by name, is
// checked and opens that provider.
func (w *appWindow) addProviderMenu(menu *ui.Menu) {
	presets := append(slices.Clone(model.CustomPresets), model.DecisionPresets...)
	for i, preset := range presets {
		if i > 0 && (preset.Local != presets[i-1].Local || preset.API.Decides() != presets[i-1].API.Decides()) {
			menu.Separator()
		}
		existing := model.PresetProvider(preset, store.Providers)
		if menu.Item(preset.Name).Checked(existing != nil).Chosen() {
			kind := model.ProviderKind("")
			if existing != nil && model.IsCustomKind(existing.Kind) {
				kind = existing.Kind
			}
			chosen := preset
			w.presentCustomProvider(kind, &chosen, nil)
		}
	}
	menu.Separator()
	if menu.Item(L("Other Server…")).Chosen() {
		w.presentCustomProvider("", nil, nil)
	}
}

// MARK: - Custom provider

// presentCustomProvider opens a custom provider the account has (its key fetched first, so the
// sheet opens filled in), a preset, or an empty sheet (kind "" and preset nil). A kind the account
// no longer has opens the sheet to add one. onSave gets the provider's kind once it is saved; it
// may be nil.
func (w *appWindow) presentCustomProvider(kind model.ProviderKind, preset *model.CustomPreset, onSave func(model.ProviderKind)) {
	w.presentProviderSheet(kind, preset, false, onSave)
}

// presentBotProvider opens the sheet to add a provider a bot will run with: its API picker leaves
// out the decision APIs.
func (w *appWindow) presentBotProvider(preset *model.CustomPreset, onSave func(model.ProviderKind)) {
	w.presentProviderSheet("", preset, true, onSave)
}

func (w *appWindow) presentProviderSheet(kind model.ProviderKind, preset *model.CustomPreset, forBots bool, onSave func(model.ProviderKind)) {
	if providerFetching {
		return
	}
	var existing *model.ProviderCredential
	if kind != "" {
		if found := store.Credential(kind); found != nil {
			saved := *found
			existing = &saved
		}
	}
	open := func(apiKey string) {
		if existing != nil {
			preset = nil
		}
		s := newProviderCustomSheet(existing, preset, apiKey, onSave)
		s.forBots = forBots
		w.present(s.view, func() { s.closed = true })
	}
	if existing == nil {
		open("")
		return
	}
	providerFetching = true
	store.ProviderAPIKey(existing.Kind, func(saved model.SavedKey, err error) {
		providerFetching = false
		if err != nil {
			w.showAlert(alertOptions{Message: model.ErrorText(err)}, nil)
			return
		}
		key := ""
		if saved.APIKey != nil {
			key = *saved.APIKey
		}
		open(key)
	})
}

// providerListing is where the server's model list stands.
type providerListing int

const (
	providerNeedsURL providerListing = iota
	providerLoading
	providerListed
	providerUnlisted
	providerFailed
)

// providerCustomSheet is the custom provider sheet's state.
type providerCustomSheet struct {
	existing *model.ProviderCredential
	preset   *model.CustomPreset
	// kind is the provider being edited; "" adds one.
	kind        model.ProviderKind
	initialName string
	onSave      func(model.ProviderKind)
	// forBots leaves the decision APIs out of the API picker, for a provider a bot will run with.
	forBots bool

	name, baseURL, key string
	api                model.CustomAPI
	checklist          model.ModelChecklist
	search             string
	listing            providerListing
	failure            string

	busy   bool
	status *providerStatus
	closed bool

	// generation is the newest request for the server's models: an older one's answer is dropped.
	generation int
	// loadAt is when the server is asked, once the fields stop changing; zero for no request.
	loadAt time.Time
	query  model.ModelQuery
	// doneAt is when the sheet closes after saving, the connected line read.
	doneAt    time.Time
	savedKind model.ProviderKind
}

func newProviderCustomSheet(existing *model.ProviderCredential, preset *model.CustomPreset, apiKey string, onSave func(model.ProviderKind)) *providerCustomSheet {
	s := &providerCustomSheet{existing: existing, preset: preset, key: apiKey, onSave: onSave, api: model.APIChatCompletions}
	if existing != nil {
		if model.IsCustomKind(existing.Kind) {
			s.kind = existing.Kind
		}
		s.name, s.baseURL = existing.Name, existing.BaseURL
		if existing.API != "" {
			s.api = existing.API
		}
	} else if preset != nil {
		s.name, s.baseURL, s.api = preset.Name, preset.BaseURL, preset.API
	}
	s.initialName = s.name
	// A provider being edited starts with its saved models, picked, the first the default.
	var models []model.CustomModel
	if existing != nil {
		models = existing.Models
	}
	s.checklist = model.SavedChecklist(models)
	// A preset's or a saved provider's server is asked as the sheet opens.
	s.loadModels(0)
	return s
}

// savedName is the name to save: the one typed, else the known server's name or the host.
func (s *providerCustomSheet) savedName() string {
	if name := strings.TrimSpace(s.name); name != "" {
		return name
	}
	return model.SuggestedProviderName(s.baseURL, s.api)
}

// keyPlaceholder is the key's hint: while adding, the one of the server the base URL names.
func (s *providerCustomSheet) keyPlaceholder() string {
	if s.existing == nil {
		if preset := model.MatchingPreset(s.baseURL, s.api); preset != nil {
			return preset.KeyPlaceholder()
		}
	}
	return L("Optional for a server on your network")
}

func (s *providerCustomSheet) picked() []model.CustomModel {
	var out []model.CustomModel
	for _, each := range s.checklist.Models {
		if s.checklist.Selected[each.ID] {
			out = append(out, each)
		}
	}
	return out
}

func (s *providerCustomSheet) canConfirm() bool {
	return !s.busy && s.savedName() != "" && strings.TrimSpace(s.baseURL) != "" && len(s.checklist.Selected) > 0
}

// loadModels asks the server for its models once the fields stop changing, `delay` from now; a
// newer request replaces an older one's answer.
func (s *providerCustomSheet) loadModels(delay time.Duration) {
	s.generation++
	s.loadAt = time.Time{}
	root := strings.TrimSpace(s.baseURL)
	if !model.IsUsableBaseURL(root) {
		s.listing = providerNeedsURL
		return
	}
	s.listing = providerLoading
	s.query = model.ModelQuery{Name: s.savedName(), API: s.api, BaseURL: root, APIKey: s.key}
	s.loadAt = time.Now().Add(delay)
}

func (s *providerCustomSheet) askServer() {
	generation := s.generation
	store.ListCustomModels(s.query, func(models []model.CustomModel, listed bool, err error) {
		if s.closed || generation != s.generation {
			return
		}
		if err != nil {
			s.listing, s.failure = providerFailed, model.ErrorText(err)
			return
		}
		// A listing replaces the last one, and a server with none leaves no other server's models
		// behind; picked and typed models stay.
		s.checklist = s.checklist.TakeListing(models)
		if !listed || len(models) == 0 {
			s.listing = providerUnlisted
			return
		}
		s.listing = providerListed
	})
}

// tick runs what waits on the clock: the server's models, and closing once saved.
func (s *providerCustomSheet) tick(c *ui.Context, sh *sheet) {
	now := c.Now()
	if !s.loadAt.IsZero() {
		if now.Before(s.loadAt) {
			c.After(s.loadAt.Sub(now))
		} else {
			s.loadAt = time.Time{}
			s.askServer()
		}
	}
	if !s.doneAt.IsZero() {
		if now.Before(s.doneAt) {
			c.After(s.doneAt.Sub(now))
		} else {
			s.doneAt = time.Time{}
			sh.dismiss()
			if s.onSave != nil {
				s.onSave(s.savedKind)
			}
		}
	}
}

// overlay is the message in place of the list when it has no rows: what the sheet needs, that it
// is loading, or what the server answered.
func (s *providerCustomSheet) overlay() string {
	switch s.listing {
	case providerNeedsURL:
		return L("Enter the base URL to load the server’s models.")
	case providerLoading:
		return L("Loading models…")
	case providerUnlisted:
		return L("This server doesn’t list its models. Add model IDs above.")
	case providerFailed:
		return s.failure + " " + L("You can still add model IDs above.")
	}
	return ""
}

func (s *providerCustomSheet) begin(message string) {
	s.busy = true
	s.status = &providerStatus{text: message, tone: model.ToneSecondary, spinning: true}
}

func (s *providerCustomSheet) fail(err error) {
	s.busy = false
	s.status = &providerStatus{text: model.ErrorText(err), tone: model.ToneRed}
}

func (s *providerCustomSheet) add(id string) {
	s.checklist = s.checklist.Add(id)
	s.search = ""
}

// searchReturn is Return in the search field: it adds the id it holds, or picks the one model it
// names; with nothing typed it falls through to the sheet's default button.
func (s *providerCustomSheet) searchReturn() {
	query := strings.TrimSpace(s.search)
	if query == "" {
		fieldSubmitted = true
		return
	}
	if s.busy {
		return
	}
	if id := model.AddCandidate(s.search, s.checklist.Models); id != "" {
		s.add(id)
		return
	}
	if !s.checklist.Selected[query] {
		s.checklist = s.checklist.Toggle(query)
	}
	s.search = ""
}

func (s *providerCustomSheet) confirm() {
	if !s.canConfirm() {
		return
	}
	saved := s.savedName()
	s.begin(L("Checking %@…", saved))
	store.SaveCustomProvider(model.CustomProvider{Kind: s.kind, Name: saved, API: s.api, BaseURL: s.baseURL, APIKey: s.key, Models: s.checklist.OrderedIDs()},
		func(kind model.ProviderKind, err error) {
			if s.closed {
				return
			}
			if err != nil {
				s.fail(err)
				return
			}
			s.status = &providerStatus{text: L("%@ connected.", saved), tone: model.ToneGreen}
			s.savedKind = kind
			s.doneAt = time.Now().Add(600 * time.Millisecond)
		})
}

// remove deletes the provider for the whole account, with no question first, as Disconnect does.
func (s *providerCustomSheet) remove(sh *sheet) {
	if s.kind == "" {
		return
	}
	s.begin(L("Deleting…"))
	store.DisconnectProvider(s.kind, func(err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.fail(err)
			return
		}
		sh.dismiss()
	})
}

func (s *providerCustomSheet) view(c *ui.Context, sh *sheet) {
	s.tick(c, sh)
	title := L("Add Custom Provider")
	switch {
	case s.existing != nil:
		title = model.ProviderName(s.existing.Kind, store.Providers)
	case s.preset != nil:
		title = L("Add %@", s.preset.Name)
	}
	confirm := L("Add")
	var leading func()
	if s.kind != "" {
		confirm = L("Save")
		leading = func() {
			if pushButton(c, L("Delete"), pushOptions{Kind: buttonDestructive, Disabled: s.busy}).Clicked() {
				s.remove(sh)
			}
		}
	}
	result := sheetFrame(c, sheetOptions{
		Title:           title,
		Subtitle:        L("Any server that speaks OpenAI’s or Anthropic’s API, such as a gateway or a model server on your network, or a decision API for Auto-review. Encrypted and shared with your paired Devices."),
		Width:           520,
		Confirm:         confirm,
		ConfirmDisabled: !s.canConfirm(),
		Leading:         leading,
	}, func() {
		s.formView(c)
		s.modelsView(c)
		providerStatusLine(c, s.status)
	})
	if result.Confirmed {
		s.confirm()
	}
	if result.Cancelled {
		s.closed = true
		sh.dismiss()
	}
}

// formView is the server's name, protocol, address, and key, with the labels in a column as wide
// as the longest.
func (s *providerCustomSheet) formView(c *ui.Context) {
	labelWidth := mcpFormLabelWidth(c, L("Name"), L("API"), L("API base URL"), L("API key"))
	ui.Column(c).Gap(8).Children(func() {
		providerFormRow(c, labelWidth, L("Name"), func() {
			textField(c, &s.name, fieldOptions{
				Placeholder: firstNonEmpty(model.SuggestedProviderName(s.baseURL, s.api), "OpenRouter"),
				Disabled:    s.busy,
				Label:       L("Name"),
				// A preset needs its key next; an empty sheet starts at the name.
				AutoFocus: s.existing != nil || s.initialName == "",
			})
		})
		providerFormRow(c, labelWidth, L("API"), func() {
			options := make([]popUpOption, 0, len(model.CustomAPIs))
			for _, api := range model.CustomAPIs {
				if !s.forBots || !api.Decides() {
					options = append(options, popUpOption{Value: string(api), Label: api.Title()})
				}
			}
			if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: string(s.api), Disabled: s.busy, Label: L("API")}); changed {
				s.api = model.CustomAPI(picked)
				s.loadModels(0)
			}
		})
		providerFormRow(c, labelWidth, L("API base URL"), func() {
			field := textField(c, &s.baseURL, fieldOptions{Placeholder: model.CustomBaseURLPlaceholder(s.api), Mono: true, Disabled: s.busy, Label: L("API base URL")})
			if field.Changed() {
				s.loadModels(500 * time.Millisecond)
			}
		})
		providerFormNote(c, labelWidth, model.CustomEndpointNote(s.api, s.baseURL), nil)
		providerFormRow(c, labelWidth, L("API key"), func() {
			field := providerKeyField(c, &s.key, s.keyPlaceholder(), s.busy, s.existing == nil && s.initialName != "")
			if field.Changed() {
				s.loadModels(500 * time.Millisecond)
			}
		})
	})
}

// modelsView is the models to offer: a search that adds what it names, the list to pick from, and
// how many are picked with the one bots run without a model of their own.
func (s *providerCustomSheet) modelsView(c *ui.Context) {
	p := colors(c)
	adding := model.AddCandidate(s.search, s.checklist.Models)
	rows := model.FilterModels(s.checklist.Models, s.search)
	isEmpty := adding == "" && len(rows) == 0
	ui.Column(c).Gap(8).Children(func() {
		ui.Row(c).Gap(6).Margin(0, 0, -2, 0).Children(func() {
			ui.Text(c, L("Models")).FontSize(12).FontWeight(600)
			// With rows on screen, a failure reads here instead of in the list.
			note := ""
			if s.listing == providerFailed && !isEmpty {
				note = s.failure
			}
			text := ui.Text(c, note).Grow(1).Shrink(1).MinWidth(0).FontSize(textCaption).TextColor(p.Label2).TextAlign(ui.End).SingleLine()
			if note != "" {
				text.Tooltip(note)
			}
			if s.listing == providerLoading {
				spinner(c, 12)
			}
		})
		if searchField(c, &s.search, L("Search or add a model ID")).Submitted() {
			s.searchReturn()
		}
		list := ui.Scroll(c).Height(196).Radius(9).Border(1, p.BotBubbleBorder).Background(p.BotBubble).Label(L("Models"))
		list.Children(func() {
			if adding != "" {
				ui.Box(c.Key("add")).Children(func() {
					title := L("Add “%@”", adding)
					b := ui.ButtonBase(c).Height(34).Padding(0, 10).Gap(6).Justify(ui.Start).TextColor(p.Accent).Label(title).Disabled(s.busy)
					b.Children(func() {
						symbol(c, "plus.circle.fill", 14, 2).Shrink(0)
						ui.Text(c, title).FontSize(13).SingleLine().Shrink(1).MinWidth(0)
					})
					if b.Clicked() {
						s.add(adding)
					}
				})
			}
			for _, each := range rows {
				providerModelRow(c, each, s.checklist.Selected[each.ID], s.busy,
					func() { s.checklist = s.checklist.Toggle(each.ID) })
			}
			if isEmpty {
				if message := s.overlay(); message != "" {
					ui.Column(c.Key("message")).MinHeight(194).Gap(8).Padding(0, 24).Justify(ui.Center).AlignItems(ui.Center).Children(func() {
						if s.listing == providerLoading {
							spinner(c, 14)
						}
						ui.Text(c, message).FontSize(textCaption).LineHeight(1.4).TextColor(p.Label2).TextAlign(ui.Center)
					})
				}
			}
		})
		// The row keeps its height while nothing is picked.
		picked := s.picked()
		ui.Row(c).Height(22).Gap(6).Justify(ui.SpaceBetween).Children(func() {
			count := ""
			if len(picked) > 0 {
				count = L("%d selected", len(picked))
			}
			ui.Text(c, count).FontSize(textCaption).TextColor(p.Label2)
			if len(picked) == 0 {
				return
			}
			ui.Row(c).Gap(6).MinWidth(0).Children(func() {
				ui.Text(c, L("Default")).FontSize(textCaption).TextColor(p.Label2)
				options := make([]popUpOption, 0, len(picked))
				for _, each := range picked {
					options = append(options, popUpOption{Value: each.ID, Label: each.DisplayName()})
				}
				if id, changed, _ := popUpButton(c, popUp{Options: options, Value: s.checklist.DefaultID, Disabled: s.busy, Label: L("Default model")}); changed {
					s.checklist.DefaultID = id
				}
			})
		})
	})
}

// providerModelRow is a model to pick: a click anywhere on it toggles it, as the Mac's checklist
// does. Change runs after the view is built with the new choice.
func providerModelRow(c *ui.Context, each model.CustomModel, checked, disabled bool, change func()) {
	p := colors(c)
	ui.Box(c.Key(each.ID)).Children(func() {
		on := checked
		name := each.DisplayName()
		row := ui.CheckboxBase(c, &on).Height(34).Padding(0, 10).Gap(6).Label(name).Disabled(disabled).OnChange(change)
		row.Children(func() {
			box := ui.Box(c).Size(14, 14).Radius(3.5).Shrink(0).Center()
			if checked {
				box.Background(p.Accent).TextColor(p.AccentText).Children(func() { symbol(c, "checkmark", 10, 3) })
			} else {
				box.Background(p.Field).Border(1, p.Label3)
			}
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() {
				ui.Text(c, name).FontSize(13).TextColor(p.Label).SingleLine()
				if name != each.ID {
					ui.Text(c, each.ID).FontSize(11).TextColor(p.Label2).SingleLine()
				}
			})
			if each.ContextWindow > 0 {
				tokens := model.Tokens(each.ContextWindow)
				ui.Text(c, tokens).Shrink(0).Margin(0, 0, 0, 8).FontSize(11).FontFeatures("tnum").TextColor(p.Label2).
					Tooltip(L("Context window: %@ tokens", tokens))
			}
			if each.Images != nil && *each.Images {
				ui.Row(c).Shrink(0).TextColor(p.Label2).Tooltip(L("Sees images")).Label(L("Sees images")).Children(func() {
					symbol(c, "eye", 13, 1.8)
				})
			}
		})
	})
}

// MARK: - Connect a built-in provider

// presentConnectProvider connects a built-in provider: its API key, or its sign-in. The saved key
// is fetched first, so the sheet opens filled in and masked. baseURL is the one the account's
// credential has; onDone runs once it is connected, and may be nil. A custom provider opens its
// own sheet.
func (w *appWindow) presentConnectProvider(kind model.ProviderKind, baseURL string, onDone func()) {
	if model.IsCustomKind(kind) {
		w.presentCustomProvider(kind, nil, func(model.ProviderKind) {
			if onDone != nil {
				onDone()
			}
		})
		return
	}
	if providerFetching {
		return
	}
	open := func(credential *model.SavedKey) {
		s := &providerConnectSheet{kind: kind, onDone: onDone, baseURL: baseURL}
		if credential != nil {
			s.editing = credential.APIKey != nil
			if credential.APIKey != nil {
				s.key = *credential.APIKey
			}
			s.baseURL = ""
			if credential.BaseURL != nil {
				s.baseURL = *credential.BaseURL
			}
		}
		w.present(s.view, func() { s.closed = true })
	}
	credential := store.Credential(kind)
	if !model.UsesAPIKey(kind) || credential == nil || !credential.IsConnected {
		open(nil)
		return
	}
	providerFetching = true
	store.ProviderAPIKey(kind, func(saved model.SavedKey, err error) {
		providerFetching = false
		if err != nil {
			w.showAlert(alertOptions{Message: model.ErrorText(err)}, nil)
			return
		}
		open(&saved)
	})
}

// providerConnectSheet is the state of the sheet that connects a built-in provider.
type providerConnectSheet struct {
	kind         model.ProviderKind
	editing      bool
	key, baseURL string
	onDone       func()

	busy, closed bool
	// signingIn is a sign-in waiting on the browser, which a cancel stops.
	signingIn bool
	status    *providerStatus
	// doneAt is when the sheet closes, the connected line read.
	doneAt time.Time
}

func (s *providerConnectSheet) canConfirm() bool {
	return !s.busy && (!model.UsesAPIKey(s.kind) || strings.TrimSpace(s.key) != "")
}

func (s *providerConnectSheet) begin(message string) {
	s.busy = true
	s.status = &providerStatus{text: message, tone: model.ToneSecondary, spinning: true}
}

func (s *providerConnectSheet) fail(err error) {
	s.busy = false
	s.status = &providerStatus{text: model.ErrorText(err), tone: model.ToneRed}
}

func (s *providerConnectSheet) finish(sh *sheet) {
	sh.dismiss()
	if s.onDone != nil {
		s.onDone()
	}
}

func (s *providerConnectSheet) confirm() {
	if !s.canConfirm() {
		return
	}
	name := model.ProviderName(s.kind, store.Providers)
	connected := func(err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.fail(err)
			return
		}
		s.status = &providerStatus{text: L("%@ connected.", name), tone: model.ToneGreen}
		s.doneAt = time.Now().Add(600 * time.Millisecond)
	}
	if model.UsesAPIKey(s.kind) {
		s.begin(L("Checking the key with %@…", name))
		store.ConnectAPIKey(s.kind, strings.TrimSpace(s.key), s.baseURL, connected)
		return
	}
	s.begin(L("Waiting for the browser…"))
	s.signingIn = true
	store.ConnectSignIn(s.kind, func(err error) {
		s.signingIn = false
		connected(err)
	})
}

func (s *providerConnectSheet) disconnect(sh *sheet) {
	s.begin(L("Disconnecting…"))
	store.DisconnectProvider(s.kind, func(err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.fail(err)
			return
		}
		s.finish(sh)
	})
}

func (s *providerConnectSheet) cancel(sh *sheet) {
	s.closed = true
	// A sign-in waiting on the browser stops with the sheet.
	if s.signingIn {
		store.CancelSignIn()
	}
	sh.dismiss()
}

func (s *providerConnectSheet) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	if !s.doneAt.IsZero() {
		if now := c.Now(); now.Before(s.doneAt) {
			c.After(s.doneAt.Sub(now))
		} else {
			s.doneAt = time.Time{}
			s.finish(sh)
			return
		}
	}
	kind := s.kind
	name := model.ProviderName(kind, store.Providers)
	usesKey := model.UsesAPIKey(kind)
	title, subtitle, confirm := L("Connect %@", name), L("Encrypted and shared with your paired Devices."), L("Connect")
	if s.editing {
		title, confirm = name, L("Save")
	}
	if !usesKey {
		subtitle = L("Your browser opens a %@ sign-in. The tokens are shared with your paired Devices, encrypted with your account key; the relay cannot read them.", name)
		confirm = L("Sign in with %@…", name)
	}
	var leading func()
	if s.editing && usesKey {
		leading = func() {
			if pushButton(c, L("Disconnect"), pushOptions{Kind: buttonDestructive, Disabled: s.busy}).Clicked() {
				s.disconnect(sh)
			}
		}
	}
	result := sheetFrame(c, sheetOptions{Title: title, Subtitle: subtitle, Width: 420, Confirm: confirm, ConfirmDisabled: !s.canConfirm(), Leading: leading}, func() {
		if !usesKey {
			flow := L("Sign-in uses the same OAuth flow as xAI's Grok CLI.")
			if kind == "chatgpt" {
				flow = L("Sign-in uses the same OAuth flow as the Codex CLI.")
			}
			providerNote(c, flow+" "+model.SignInRequirement(kind), nil)
		} else {
			ui.Column(c).Gap(5).Children(func() {
				ui.Text(c, L("API key")).FontSize(textCaption).TextColor(p.Label2)
				providerKeyField(c, &s.key, model.KeyPlaceholder(kind), s.busy, true)
			})
			ui.Column(c).Gap(4).Children(func() {
				ui.Column(c).Gap(5).Children(func() {
					ui.Text(c, L("API base URL")).FontSize(textCaption).TextColor(p.Label2)
					textField(c, &s.baseURL, fieldOptions{Placeholder: model.DefaultBaseURL(kind), Mono: true, Disabled: s.busy, Label: L("API base URL")})
				})
				providerNote(c, L("Leave empty to use %@’s API.", name), nil)
			})
		}
		providerStatusLine(c, s.status)
	})
	if result.Confirmed {
		s.confirm()
	}
	if result.Cancelled {
		s.cancel(sh)
	}
}
