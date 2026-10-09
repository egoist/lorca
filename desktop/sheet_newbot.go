package main

import (
	"slices"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A new bot, after the macOS app's NewBotViewController: its name, what it does, a look, the Runner
// it runs on, and the provider, model, and thinking level. Every bot gets a direct chat, which opens
// when it is made.

// newBotLook is one of the looks a new bot starts with.
type newBotLook struct {
	symbolName string
	accent     model.Accent
}

var newBotLooks = []newBotLook{
	{"sparkles", "indigo"},
	{"chevron.left.forwardslash.chevron.right", "blue"},
	{"binoculars.fill", "teal"},
	{"pencil.and.scribble", "pink"},
	{"bolt.horizontal.fill", "orange"},
	{"leaf.fill", "green"},
	{"wand.and.stars", "purple"},
	{"flame.fill", "red"},
}

type newBotState struct {
	name        string
	description string
	look        int
	runnerID    string
	provider    model.ProviderKind
	// model and thinking are empty for the provider's and the model's defaults.
	model    string
	thinking string
	// kinds are the providers as the sheet opened, the built-in ones and then the custom ones, so
	// the pop-up keeps its order while the account's list changes.
	kinds []model.ProviderKind
	// returned is Return in the description, which creates the bot as a wrapping field's does.
	returned bool
}

// presentNewBot makes a bot; onCreate gets its id.
func (w *appWindow) presentNewBot(onCreate func(botID string)) {
	st := &newBotState{kinds: store.ProviderKinds()}
	if runners := store.Runners(); len(runners) > 0 {
		st.runnerID = runners[0].ID
	}
	st.provider = model.ProviderKinds[0]
	w.present(func(c *ui.Context, s *sheet) { st.view(c, s, onCreate) }, nil)
}

// runner is the Runner picked in the pop-up: the first one until another is picked, nil while
// none is paired.
func (st *newBotState) runner() *model.Device {
	runners := store.Runners()
	for _, device := range runners {
		if device.ID == st.runnerID {
			return device
		}
	}
	if len(runners) > 0 {
		return runners[0]
	}
	return nil
}

// catalog is the CLI's catalog, which comes with each snapshot, and the custom providers' saved
// models.
func (st *newBotState) catalog() []model.ProviderModel {
	return model.WithCustomModels(store.Models, store.Providers)
}

// note says where the bot's turns run, or what keeps them from running, and whether that is a
// warning.
func (st *newBotState) note() (string, bool) {
	host := st.runner()
	if host == nil {
		return L("No Runner is paired. Bots run on a Device with macOS, Linux, or Windows."), true
	}
	name := model.ProviderName(st.provider, store.Providers)
	if credential := store.Credential(st.provider); credential != nil && credential.IsConnected {
		return L("%@ is connected. Turns run on %@.", name, host.Name), false
	}
	return L("%@ is not connected yet. The bot is created now and its first turn waits until you connect it in Settings.", name), true
}

// canCreate is a name and a Runner to run the bot on.
func (st *newBotState) canCreate() bool {
	return strings.TrimSpace(st.name) != "" && st.runner() != nil
}

func (st *newBotState) create(s *sheet, onCreate func(botID string)) {
	host := st.runner()
	name := strings.TrimSpace(st.name)
	if host == nil || name == "" {
		return
	}
	look := newBotLooks[st.look]
	botID := store.CreateBot(model.NewBot{
		Name:        name,
		Description: strings.TrimSpace(st.description),
		SymbolName:  look.symbolName,
		Accent:      look.accent,
		RunnerID:    host.ID,
		Provider:    st.provider,
		Model:       st.model,
		Thinking:    st.thinking,
	})
	s.dismiss()
	if onCreate != nil {
		onCreate(botID)
	}
}

func (st *newBotState) view(c *ui.Context, s *sheet, onCreate func(botID string)) {
	p := colors(c)
	const width = 440
	// The pop-ups fill the row after the label, as wide as the sheet's content allows.
	fill := float32(width - 40 - formLabelWidth - 10)
	catalog := st.catalog()
	models := model.ProviderModels(catalog, st.provider)
	levels := model.ThinkingLevels(catalog, st.provider, st.model)
	result := sheetFrame(c, sheetOptions{
		Title:           L("New Bot"),
		Subtitle:        L("A bot runs on one Runner and uses that machine's credentials. Phones and tablets are not Runners."),
		Width:           width,
		Confirm:         L("Create Bot"),
		ConfirmDisabled: !st.canCreate(),
	}, func() {
		formRow(c, L("Name"), false, func() {
			textField(c, &st.name, fieldOptions{Placeholder: L("Name"), AutoFocus: true, Label: L("Name")}).Grow(1).MinWidth(0)
		})
		formRow(c, L("Description"), true, func() {
			field := textArea(c, &st.description, 0, fieldOptions{Placeholder: L("What it does and how it should work"), Label: L("Description")}).
				Lines(3, 12).Grow(1).MinWidth(0).MinHeight(54)
			composing := field.Composing()
			field.HandleInput(func(ev ui.InputEvent) bool {
				// Return creates the bot, as in a wrapping text field; Alt or Shift with it starts a line.
				if ev.Kind != ui.InputKeyDown || ev.Key != ui.KeyEnter || ev.Mods != 0 || composing {
					return false
				}
				st.returned = true
				return true
			})
		})
		formRow(c, L("Look"), false, func() {
			ui.Row(c).Gap(8).Children(func() {
				for i, look := range newBotLooks {
					b := ui.ButtonBase(c).Size(26, 26).Radius(13).Label(look.symbolName).Role(ui.RoleToggleButton).Checked(i == st.look).Cursor(ui.CursorPointer)
					if i != st.look {
						b.Opacity(0.35)
					}
					b.Children(func() {
						avatar(c, avatarContent{Kind: avatarBot, SymbolName: look.symbolName, Accent: look.accent}, 26, false)
					})
					if b.Clicked() {
						st.look = i
					}
				}
			})
		})
		formRow(c, L("Runner"), false, func() {
			var options []popUpOption
			for _, device := range store.Runners() {
				label := device.Name
				if device.IsThisDevice {
					label = L("%@ (this computer)", device.Name)
				}
				options = append(options, popUpOption{Value: device.ID, Label: label})
			}
			value := ""
			if host := st.runner(); host != nil {
				value = host.ID
			}
			if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: value, Width: fill, Label: L("Runner")}); changed {
				st.runnerID = picked
			}
		})
		formRow(c, L("Provider"), false, func() {
			options := make([]popUpOption, 0, len(st.kinds))
			for _, kind := range st.kinds {
				options = append(options, popUpOption{Value: kind, Label: model.ProviderName(kind, store.Providers) + " (" + model.ProviderSubtitle(kind) + ")"})
			}
			if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: st.provider, Width: fill, Label: L("Provider")}); changed {
				// A new provider starts on its default model and thinking level.
				st.provider, st.model, st.thinking = picked, "", ""
			}
		})
		formRow(c, L("Model"), false, func() {
			first := ""
			if len(models) > 0 {
				first = models[0].Label
			}
			options := []popUpOption{{Value: "", Label: L("Default (%@)", first)}}
			for _, each := range models {
				options = append(options, popUpOption{Value: each.ID, Label: each.Label})
			}
			if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: st.model, Width: fill, Label: L("Model")}); changed {
				st.model = picked
				// A level the new model does not take goes back to the default.
				taken := model.ThinkingLevels(st.catalog(), st.provider, st.model)
				if !slices.ContainsFunc(taken, func(level model.Choice) bool { return level.ID == st.thinking }) {
					st.thinking = ""
				}
			}
		})
		// Only the levels this model takes; a model without any has no choice to make.
		if len(levels) > 0 {
			formRow(c, L("Thinking"), false, func() {
				options := []popUpOption{{Value: "", Label: L("Default")}}
				for _, level := range levels {
					options = append(options, popUpOption{Value: level.ID, Label: level.Label})
				}
				if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: st.thinking, Width: fill, Label: L("Thinking")}); changed {
					st.thinking = picked
				}
			})
		}
		text, warning := st.note()
		tint := p.Label3
		if warning {
			tint = p.Orange
		}
		ui.Text(c, text).FontSize(textCaption).LineHeight(1.4).TextColor(tint)
	})
	returned := st.returned
	st.returned = false
	switch {
	case result.Cancelled:
		s.dismiss()
	case result.Confirmed, returned && st.canCreate():
		st.create(s, onCreate)
	}
}
