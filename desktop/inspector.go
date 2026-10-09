package main

import (
	"slices"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The pane beside a chat, after the macOS app's InspectorViewController: the bots in the chat, a
// group's name, description, and project context, what the chat's bots published, and for a DM the bot's profile, what it runs with (provider,
// model, thinking, the credential, and what the turns used), its memory, its routines, the plugins
// on its Runner, and where turns run.

// inspectorState is what each bot's Runner last said about its memory, refreshed when the pane
// opens on a chat and after every turn in it; why the last fetch failed (the Runner is offline, or
// did not answer); and the fetches on their way. Kept while the pane is closed, as the macOS
// inspector keeps them.
type inspectorState struct {
	memory   map[string]model.BotMemory
	errors   map[string]string
	fetching map[string]bool
	// feedback is what each bot's Runner last said about its workflow feedback, refreshed with
	// its memory and when the Runner says it changed; the feedback sheets show it too.
	feedback         map[string]model.BotFeedback
	fetchingFeedback map[string]bool
	// shownChat is the chat the pane last opened on.
	shownChat string
	scroll    ui.ScrollState
	// tasksShowingAll is the chat whose Tasks section shows every task rather than the first few.
	tasksShowingAll string
	project         projectInspectorState
	// skillsShowingAll is the bot or group whose Skills section shows every skill.
	skillsShowingAll model.PlaybookScope
}

// refreshMemory asks the CLI for the bot's memory; the section redraws when it answers.
func (s *inspectorState) refreshMemory(botID string) {
	if s.memory == nil {
		s.memory, s.errors, s.fetching = map[string]model.BotMemory{}, map[string]string{}, map[string]bool{}
	}
	if _, known := s.memory[botID]; (store.IsMock && known) || s.fetching[botID] {
		return
	}
	s.fetching[botID] = true
	store.BotMemory(botID, func(memory model.BotMemory, err error) {
		delete(s.fetching, botID)
		if err != nil {
			s.errors[botID] = model.ErrorText(err)
			return
		}
		s.memory[botID] = memory
		delete(s.errors, botID)
	})
}

// refreshFeedback asks the bot's Runner for its feedback. The section stays out while there is
// none, or while the Runner cannot be asked.
func (s *inspectorState) refreshFeedback(botID string) {
	if s.feedback == nil {
		s.feedback, s.fetchingFeedback = map[string]model.BotFeedback{}, map[string]bool{}
	}
	if s.fetchingFeedback[botID] {
		return
	}
	s.fetchingFeedback[botID] = true
	store.Feedback(botID, func(f model.BotFeedback, err error) {
		delete(s.fetchingFeedback, botID)
		if err == nil {
			s.feedback[botID] = f
		}
	})
}

// refreshShownMemory asks for the memory and the feedback of the bot whose DM is showing.
func (s *inspectorState) refreshShownMemory(chatID string) {
	chat := store.Chat(chatID)
	if chat == nil || !chat.IsDM() {
		return
	}
	if bots := store.BotsIn(chat); len(bots) > 0 {
		s.refreshMemory(bots[0].ID)
		s.refreshFeedback(bots[0].ID)
	}
}

// inspectorStoreChanged follows the turns: one that ended may have moved what the bot remembers.
// A bot's feedback is asked for again when its Runner says it changed, and a group's project
// context is listed again when it changes.
func (m *mainWindow) inspectorStoreChanged(event model.Event) {
	if event.Kind == model.EventRespondingChanged && event.ChatID == m.inspector.shownChat && !store.IsResponding(event.ChatID) {
		m.inspector.refreshShownMemory(event.ChatID)
	}
	if event.Kind == model.EventFeedbackChanged {
		if _, known := m.inspector.feedback[event.BotID]; known {
			m.inspector.refreshFeedback(event.BotID)
		}
	}
	if event.Kind == model.EventProjectContextChanged && event.ChatID == m.inspector.shownChat {
		m.inspector.project.refresh(event.ChatID)
	}
}

// joinPath is the Runner's path joined with a file name, in the Runner's own separators.
func joinPath(directory, name string) string {
	separator := "/"
	if strings.Contains(directory, "\\") && !strings.Contains(directory, "/") {
		separator = "\\"
	}
	if strings.HasSuffix(directory, separator) {
		return directory + name
	}
	return directory + separator + name
}

func (m *mainWindow) inspectorView(c *ui.Context, chatID string) {
	s := &m.inspector
	if s.shownChat != chatID {
		s.shownChat = chatID
		s.refreshShownMemory(chatID)
		if chat := store.Chat(chatID); chat != nil && chat.IsGroup() {
			s.project.refresh(chatID)
		}
	}
	chat := store.Chat(chatID)
	if chat == nil {
		return
	}
	members := store.BotsIn(chat)
	// A direct chat is one bot, so its profile, provider, and model are edited right here.
	var single *model.Bot
	if chat.IsDM() && len(members) == 1 {
		single = members[0]
	}
	ui.Scroll(c).Grow(1).MinHeight(0).Children(func() {
		ui.Column(c).Gap(20).Padding(18, 16, 20, 16).Children(func() {
			m.inspectorParticipants(c, chat, members)
			if !chat.IsDM() {
				add := pushButton(c, L("Add Bot…"), pushOptions{Disabled: !(chat.CanAddBot() && len(members) < len(store.Bots))}).AlignSelf(ui.Start).Margin(-10, 0, 0, 0)
				if add.Clicked() {
					m.addBotToChat(chat.ID)
				}
				m.inspectorGroup(c, chat, members)
				m.inspectorProject(c, chat)
			}
			m.inspectorReviews(c, chat)
			m.inspectorOutputs(c, chat)
			if single != nil {
				m.inspectorProfile(c, single)
				m.inspectorRuntime(c, single, chat)
				m.inspectorMemory(c, single)
			}
			m.inspectorSkills(c, chat, members)
			if single != nil {
				m.inspectorRoutines(c, single)
				m.inspectorFeedback(c, single)
			}
			m.inspectorDurableTasks(c, chat)
			if single != nil {
				m.inspectorPlugins(c, single)
			}
			m.inspectorRouting(c, members)
		})
	})
}

func (m *mainWindow) inspectorParticipants(c *ui.Context, chat *model.Chat, members []*model.Bot) {
	section(c, L("Bots in this chat"), sectionCaption, nil, func(k *card) {
		// In a group of several, a click or a right-click on a member offers to make it the owner.
		hasMenu := chat.IsGroup() && len(members) > 1
		for _, bot := range members {
			host := L("unassigned")
			if device := store.Device(bot.RunnerID); device != nil {
				host = device.Name
			}
			isOwner := chat.Owner() == bot.ID
			detail := model.ProviderName(bot.Provider, store.Providers) + " · " + host
			if isOwner {
				detail = L("Owner") + " · " + detail
			}
			o := botRowOptions{Detail: detail, Clickable: hasMenu, AvatarClickable: true}
			if chat.CanRemoveBot() {
				o.AccessorySymbol, o.AccessoryTooltip = "minus.circle", L("Remove from chat")
			}
			rowElement, row := botRow(c, k, bot, o)
			botID, chatID := bot.ID, chat.ID
			if row.Accessory {
				store.RemoveBot(botID, chatID)
			}
			// The avatar is the way to a bot's look: symbol, color, or an image.
			if row.Avatar {
				m.presentBotLook(botID)
			}
			if hasMenu {
				menu := func(menu *ui.Menu) {
					if menu.Item(L("Make Owner")).Checked(isOwner).Chosen() {
						store.SetOwner(botID, chatID)
					}
					if chat.CanRemoveBot() {
						menu.Separator()
						if menu.Item(L("Remove from Chat")).Chosen() {
							store.RemoveBot(botID, chatID)
						}
					}
				}
				rowElement.ContextMenu(menu)
				rowElement.Menu(menu)
			}
		}
	}).Label(L("Bots in this chat"))
}

// inspectorGroup is a group's name and what it is for. Without a name of its own, a group goes by
// its members' names.
func (m *mainWindow) inspectorGroup(c *ui.Context, chat *model.Chat, members []*model.Bot) {
	names := make([]string, 0, len(members))
	for _, bot := range members {
		names = append(names, bot.Name)
	}
	section(c, L("Group"), sectionCaption, nil, func(k *card) {
		if value, ok := editableRow(c, k, L("Name"), chat.CustomTitle, strings.Join(names, ", "), false, true); ok && value != chat.CustomTitle {
			store.Rename(chat.ID, value)
		}
		if summaryActionRow(c, k, L("Description"), chat.GroupDescription, L("Edit…")) {
			m.presentGroupDescription(chat.ID)
		}
	})
}

func (m *mainWindow) inspectorProfile(c *ui.Context, bot *model.Bot) {
	section(c, L("Profile"), sectionCaption, nil, func(k *card) {
		// An emptied value keeps the old name; Description has its own sheet.
		if value, ok := editableRow(c, k, L("Name"), bot.Name, L("Name"), false, true); ok {
			if value != "" && value != bot.Name {
				store.UpdateBotProfile(bot.ID, value, nil)
			}
		}
		if summaryActionRow(c, k, L("Description"), bot.Description, L("Edit…")) {
			m.presentBotDescription(bot.ID)
		}
		if disclosureRow(c.Key("bot-access"), k, L("Access"), bot.Permissions.Summary(), nil) {
			m.presentBotAccess(bot.ID)
		}
	})
}

func (m *mainWindow) inspectorRuntime(c *ui.Context, bot *model.Bot, chat *model.Chat) {
	p := colors(c)
	// The built-in providers, then the custom ones. A custom provider the account deleted stays
	// listed, under its slug, while the bot is still on it.
	kinds := store.ProviderKinds()
	if !slices.Contains(kinds, bot.Provider) {
		kinds = append(kinds, bot.Provider)
	}
	// The CLI's catalog, which comes with each snapshot, and the custom providers' saved models.
	catalog := model.WithCustomModels(store.Models, store.Providers)
	models := model.ProviderModels(catalog, bot.Provider)
	levels := model.ThinkingLevels(catalog, bot.Provider, bot.Model)
	credential := store.Credential(bot.Provider)
	connected := credential != nil && credential.IsConnected
	section(c, L("Runs with"), sectionCaption, nil, func(k *card) {
		var providers []popUpOption
		for _, kind := range kinds {
			providers = append(providers, popUpOption{Value: kind, Label: model.ProviderName(kind, store.Providers)})
		}
		// A new provider starts on its default model and thinking level.
		if kind, ok := popUpRow(c, k, L("Provider"), popUp{Options: providers, Value: bot.Provider}, false); ok {
			store.SetBotRuntime(bot.ID, kind, "", "")
		}
		defaultLabel := ""
		if len(models) > 0 {
			defaultLabel = models[0].Label
		}
		modelOptions := []popUpOption{{Value: "", Label: L("Default (%@)", defaultLabel)}}
		current := ""
		for _, each := range models {
			modelOptions = append(modelOptions, popUpOption{Value: each.ID, Label: each.Label})
			if each.ID == bot.Model {
				current = each.ID
			}
		}
		if id, ok := popUpRow(c, k, L("Model"), popUp{Options: modelOptions, Value: current}, false); ok && id != bot.Model {
			// A level the new model does not take goes back to the default.
			thinking := ""
			for _, level := range model.ThinkingLevels(catalog, bot.Provider, id) {
				if level.ID == bot.Thinking {
					thinking = bot.Thinking
				}
			}
			store.SetBotRuntime(bot.ID, bot.Provider, id, thinking)
		}
		// Only the levels this model takes; a model without any has no choice to make.
		if len(levels) > 0 {
			options := []popUpOption{{Value: "", Label: L("Default")}}
			current := ""
			for _, level := range levels {
				options = append(options, popUpOption{Value: level.ID, Label: level.Label})
				if level.ID == bot.Thinking {
					current = level.ID
				}
			}
			if id, ok := popUpRow(c, k, L("Thinking"), popUp{Options: options, Value: current}, false); ok && id != bot.Thinking {
				store.SetBotRuntime(bot.ID, bot.Provider, bot.Model, id)
			}
		}
		// Connected: the masked key and a Change link. Not connected: just the Connect link. A
		// custom provider's link opens its own sheet: to edit it, or to add it again once it is
		// deleted.
		value, tint, action := "", p.Label2, L("Connect")
		if connected {
			value, tint, action = firstNonEmpty(credential.Detail, L("Connected")), p.Label, L("Change")
		}
		if _, result := actionRow(c, k, L("Credential"), actionRowOptions{Value: value, Tint: &tint, Action: action}); result.Action {
			if model.IsCustomKind(bot.Provider) {
				m.presentCustomProvider(bot.Provider, nil, nil)
			} else {
				baseURL := ""
				if credential != nil {
					baseURL = credential.BaseURL
				}
				m.presentConnectProvider(bot.Provider, baseURL, nil)
			}
		}
		// What the turns here have used, and a way to shorten the context by hand.
		if usage := chat.Usage; usage != nil {
			label := p.Label
			if _, result := actionRow(c, k, L("Context"), actionRowOptions{Value: usage.ContextSummary(), Tint: &label, Action: L("Compact")}); result.Action {
				store.CompactChat(chat.ID)
			}
			if note := usage.SpendNote(); note != "" {
				keyValueRow(c, k, L("Spent"), usage.SpendSummary(), false, nil).Tooltip(note)
			} else {
				keyValueRow(c, k, L("Spent"), usage.SpendSummary(), false, nil)
			}
		}
		// What each turn may use, or the turn that stopped at a limit.
		limits, tint := Lc("None", "limits"), p.Label2
		if b := store.Budget("chat", chat.ID, bot.RunnerID); b != nil {
			limits = b.Limits.Summary()
		}
		if stopped := store.StoppedTurn(chat.ID, bot.RunnerID); stopped != nil {
			limits, tint = stopped.StoppedLabel(), p.Orange
		}
		if disclosureRow(c, k, L("Limits"), limits, &tint) {
			m.presentBudget(bot, chat.ID, "")
		}
	})
}

// inspectorMemory is what the bot remembers, as its Runner reports it: the index against its load
// budget with an editor, and the folder of topic files and daily logs. A bot on another Runner is
// read and edited through the relay; only the folder cannot be opened from here.
func (m *mainWindow) inspectorMemory(c *ui.Context, bot *model.Bot) {
	p := colors(c)
	s := &m.inspector
	memory, known := s.memory[bot.ID]
	section(c, L("Memory"), sectionCaption, nil, func(k *card) {
		if !known {
			if text := s.errors[bot.ID]; text != "" {
				if _, result := actionRow(c, k, L("Notes"), actionRowOptions{Value: text, Tint: &p.Label2, Action: L("Retry")}); result.Action {
					s.refreshMemory(bot.ID)
				}
				return
			}
			value := ""
			if s.fetching[bot.ID] {
				value = L("Loading…")
			}
			keyValueRow(c, k, L("Notes"), value, false, &p.Label2)
			return
		}
		tint := p.Label
		tooltip := L("MEMORY.md opens at the start of every turn.")
		if memory.Truncated {
			tint = p.Orange
			tooltip = L("Only the first %d lines or %@ open each turn; the rest is not read.", memory.MaxLines, model.Kilobytes(memory.MaxBytes))
		}
		if _, result := actionRow(c, k, L("Notes"), actionRowOptions{Value: memory.BudgetSummary(), Tint: &tint, Action: L("Edit…"), Tooltip: tooltip}); result.Action {
			// The rows outlive a rename, so the sheet takes the bot as it is now.
			if current := store.Bot(bot.ID); current != nil {
				botID := current.ID
				m.presentMemory(current, memory, func() { s.refreshMemory(botID) })
			}
		}
		folder := L("%@ · on %@", memory.FilesSummary(), memory.Runner)
		action := ""
		if memory.Here {
			folder, action = memory.FilesSummary(), L("Show")
		}
		_, row := actionRow(c, k, L("Folder"), actionRowOptions{Value: folder, Tint: &p.Label2, Action: action, Tooltip: memory.Path})
		if row.Action {
			showInFolder(joinPath(memory.Path, "MEMORY.md"))
		}
	})
}

// inspectorFeedback is what the user's feedback led to: the changes the bot suggests, each a click
// away from its diff, and the rest of its feedback. It is left out until there is any.
func (m *mainWindow) inspectorFeedback(c *ui.Context, bot *model.Bot) {
	f, known := m.inspector.feedback[bot.ID]
	if !known || f.IsEmpty() {
		return
	}
	section(c, L("Feedback"), sectionCaption, nil, func(k *card) {
		for i, suggestion := range f.Suggestions {
			if i == 3 {
				break
			}
			if feedbackSuggestionRow(c, k, f, suggestion).Clicked() {
				m.presentFeedbackSuggestion(bot.ID, f, suggestion, nil)
			}
		}
		counts := L("%d notes", f.NoteCount)
		if f.NoteCount == 1 {
			counts = L("1 note")
		}
		switch len(f.Changes) {
		case 0:
		case 1:
			counts += " · " + L("1 change")
		default:
			counts += " · " + L("%d changes", len(f.Changes))
		}
		if feedbackRow(c.Key("all"), k, "bubble.left.and.bubble.right", L("All feedback"), counts, "").Clicked() {
			m.presentFeedbackList(bot.ID)
		}
	})
}

// inspectorRoutines are the bot's routines: a row per routine with a pause switch, and the details
// in a sheet. With none, the sentence that says how to get one.
func (m *mainWindow) inspectorRoutines(c *ui.Context, bot *model.Bot) {
	p := colors(c)
	routines := store.RoutinesFor(bot.ID)
	section(c, L("Routines"), sectionCaption, nil, func(k *card) {
		if len(routines) == 0 {
			noteRow(c, k, L("Routines are recurring tasks this bot runs on a schedule. Ask it in chat to set one up."), nil)
			return
		}
		for _, routine := range routines {
			symbolName, tint := "pause.circle", p.Label3
			switch {
			case routine.IsRunning:
				symbolName, tint = "arrow.triangle.2.circlepath", p.Accent
			case routine.IsEnabled:
				symbolName, tint = "clock", p.Label2
			}
			toggle := L("Pause %@", routine.Name)
			if !routine.IsEnabled {
				toggle = L("Resume %@", routine.Name)
			}
			// What went wrong leads, in orange while the user has to do something about it; a
			// routine stopped at its limits says so first, since it runs again only once resumed.
			detail := []ui.Span{{Text: routine.Detail(), Color: p.Label2}}
			if budget := store.Budget("routine", routine.ID, bot.RunnerID); budget != nil && budget.IsStopped() {
				detail = []ui.Span{{Text: budget.StoppedLabel(), Color: p.Orange}, {Text: " · " + routine.ScheduleText, Color: p.Label2}}
			} else if problem := routine.Problem(); problem.NeedsUser() {
				detail = []ui.Span{{Text: problem.Text(), Color: p.Orange}, {Text: " · " + routine.ScheduleText, Color: p.Label2}}
			}
			on := routine.IsEnabled
			id, botID := routine.ID, bot.ID
			ui.Box(c.Key(routine.ID)).Children(func() {
				if switchRow(c, k, symbolName, tint, routine.Name, detail, &on, toggle, routine.Prompt,
					func(on bool) { store.SetRoutineEnabled(id, on) }) {
					if current := store.Bot(botID); current != nil {
						m.presentRoutine(id, current, m.prefill)
					}
				}
			})
		}
	})
}

// prefill puts a text in the composer of the chat on screen, with the keyboard there.
func (m *mainWindow) prefill(text string) {
	if m.chat == nil {
		return
	}
	m.chat.composer.draft = text
	m.chat.composer.caret = len([]rune(text))
	m.chat.composer.focus = true
	m.invalidate()
}

// inspectorPlugins are the plugins the bot's Runner has, and a way to the marketplace. A plugin that
// needs setup says so, and one the bot's Access leaves out says it has none; clicking opens it.
func (m *mainWindow) inspectorPlugins(c *ui.Context, bot *model.Bot) {
	p := colors(c)
	runner := store.Device(bot.RunnerID)
	section(c, L("Plugins"), sectionCaption, nil, func(k *card) {
		var plugins []model.InstalledPlugin
		if runner != nil {
			plugins = runner.Plugins
		}
		for _, plugin := range plugins {
			pluginID := plugin.ID
			var row statusRowResult
			ui.Box(c.Key(plugin.ID)).Children(func() {
				// A plugin the bot's Access leaves out says it has none.
				if bot.Permissions.Level(plugin.ID) == model.AccessNone {
					subtitle := plugin.Description
					if plugin.AccountName != "" {
						subtitle = ""
					}
					_, row = statusRow(c, k, statusRowOptions{Symbol: plugin.Symbol(), PluginID: plugin.MarketplaceID(), Title: plugin.Name, Subtitle: subtitle,
						State: L("No access"), Clickable: true, Tooltip: L("Open %@", plugin.Name)})
				} else {
					_, row = pluginRow(c, k, plugin, true, L("Open %@", plugin.Name))
				}
			})
			if row.Clicked && runner != nil {
				m.presentPlugin(pluginID, runner, bot.ID, m.selectedChatID())
			}
		}
		if len(plugins) == 0 {
			name := L("its Runner")
			if runner != nil {
				name = runner.Name
			}
			noteRow(c, k, L("No plugins on %@ yet. Add one from the marketplace, or ask %@ to find one.", name, bot.Name), nil)
		}
		if _, result := actionRow(c, k, L("Marketplace"), actionRowOptions{Tint: &p.Label2, Action: L("Add from Plugins…")}); result.Action {
			m.presentMarketplace(bot.RunnerID)
		}
	})
}

func (m *mainWindow) inspectorRouting(c *ui.Context, members []*model.Bot) {
	p := colors(c)
	var ids []string
	for _, bot := range members {
		if !slices.Contains(ids, bot.RunnerID) {
			ids = append(ids, bot.RunnerID)
		}
	}
	slices.Sort(ids)
	section(c, L("Where turns run"), sectionCaption, nil, func(k *card) {
		for _, id := range ids {
			runner := store.Device(id)
			if runner == nil {
				continue
			}
			var names []string
			for _, bot := range members {
				if bot.RunnerID == id {
					names = append(names, bot.Name)
				}
			}
			online := runner.Status == model.StatusOnline
			state, color := model.LastSeen(runner.LastSeen), p.Label2
			if online {
				state, color = L("Online"), p.Green
			}
			var row statusRowResult
			ui.Box(c.Key(runner.ID)).Children(func() {
				_, row = statusRow(c, k, statusRowOptions{Symbol: runner.Symbol(), Title: runner.Name, Subtitle: strings.Join(names, ", "), State: state, StateColor: &color, Clickable: true})
			})
			if row.Clicked {
				m.openDevice(id)
			}
		}
	})
}
