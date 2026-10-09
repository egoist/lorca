package main

import (
	"slices"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The marketplace's workflows, after the macOS app's MarketplaceWorkflowPage: a section of them on
// the home page, the list onboarding opens, and a workflow's setup on the picked Runner, with its
// questions and its bot, the accounts it needs, a sample run to read, and its schedule, which stays
// off until the user turns it on. The CLI keeps the setup, so leaving the page keeps it too.

// workflowMatchingPacks is the workflows whose name, outcome, or description has every word of the
// query; all of them for none.
func workflowMatchingPacks(packs []model.WorkflowPack, query string) []model.WorkflowPack {
	words := strings.Fields(strings.ToLower(query))
	var found []model.WorkflowPack
	for _, pack := range packs {
		text := strings.ToLower(pack.Name + " " + pack.Outcome + " " + pack.Description)
		if !slices.ContainsFunc(words, func(word string) bool { return !strings.Contains(text, word) }) {
			found = append(found, pack)
		}
	}
	return found
}

// workflowGrid lays the workflows' rows two to a line: their symbol on a tile, their name, and
// their outcome. A click opens one.
func (mk *marketplace) workflowGrid(c *ui.Context, packs []model.WorkflowPack) {
	ui.Grid(c).Columns(2).GapX(8).GapY(2).Children(func() {
		for _, pack := range packs {
			media := func() { marketPluginIcon(c, pack.ID, firstNonEmpty(pack.SymbolName, "sparkles"), 40) }
			if marketRow(c, "workflow:"+pack.ID, marketRowOptions{Title: pack.Name, Subtitle: pack.Outcome}, media, nil) {
				mk.openWorkflow(pack)
			}
		}
	})
}

// workflowSection is the home page's Workflows, ahead of the featured plugins.
func (mk *marketplace) workflowSection(c *ui.Context, packs []model.WorkflowPack) {
	ui.Column(c.Key("workflows")).Gap(6).Children(func() {
		ui.Row(c).Height(28).Padding(0, 12).Children(func() {
			ui.Text(c, L("Workflows")).FontSize(14).FontWeight(600).SingleLine()
		})
		mk.workflowGrid(c, packs)
	})
}

// workflowListPage is the workflows alone: the first page of the sheet onboarding's Choose a
// Workflow opens.
func (mk *marketplace) workflowListPage(c *ui.Context) {
	ui.Column(c).Gap(12).Children(func() {
		marketPageTitle(c, L("Choose a Workflow"), nil)
		switch {
		case len(mk.catalog.Packs) > 0:
			mk.workflowGrid(c, mk.catalog.Packs)
		case mk.loading == marketFailed:
			ui.Column(c).Gap(10).Children(func() {
				marketStatusLine(c, L("The marketplace isn't available right now."))
				ui.Row(c).Padding(0, 12).Children(func() {
					if pushButton(c, L("Try Again"), pushOptions{}).Clicked() {
						mk.load()
					}
				})
			})
		case mk.loading == marketLoaded:
			marketStatusLine(c, L("Nothing here yet."))
		default:
			marketStatusLine(c, L("Loading…"))
		}
	})
}

// workflowPage is a workflow page's setup, and what the page shows of it while it is not saved:
// the answers as typed and the bots as picked, a bot being its id, or empty for the new one setup
// adds.
type workflowPage struct {
	pack       model.WorkflowPack
	progress   *model.WorkflowProgress
	answers    map[string]*string
	bots       map[string]string
	busy       bool
	fetching   bool
	fetchAgain bool
	closed     bool
}

func (mk *marketplace) openWorkflow(pack model.WorkflowPack) {
	wp := &workflowPage{pack: pack, answers: map[string]*string{}, bots: map[string]string{}}
	mk.show(&marketPage{kind: marketWorkflowPage, id: pack.ID, workflow: wp})
	if mk.stopWatch == nil {
		// Sign-ins, a sample's end, and the roster all move what a setup shows.
		mk.stopWatch = pluginWatchStore(func(e model.Event) {
			switch e.Kind {
			case model.EventRosterChanged, model.EventSnapshotReplaced, model.EventTurnFinished, model.EventConnectionChanged:
				for _, page := range mk.pages {
					if page.workflow != nil {
						mk.refreshWorkflow(page.workflow)
					}
				}
			}
		})
	}
	mk.startWorkflow(wp)
}

func (mk *marketplace) startWorkflow(wp *workflowPage) {
	on := mk.runner()
	if on == nil {
		return
	}
	mk.performWorkflow(wp, []workflowStep{{"start", func(*model.WorkflowProgress) map[string]any {
		return map[string]any{"pack_id": wp.pack.ID, "runner_id": on.ID}
	}}}, nil)
}

// workflowStep is one request of a few in a row: its method, and its parameters from the answer
// before it.
type workflowStep struct {
	method string
	params func(previous *model.WorkflowProgress) map[string]any
}

// performWorkflow runs requests one after another while the page's actions wait, then shows the
// last answer. A failure stops the run, and its reason shows at the foot of the sheet.
func (mk *marketplace) performWorkflow(wp *workflowPage, steps []workflowStep, then func(model.WorkflowProgress)) {
	if wp.busy || wp.closed {
		return
	}
	wp.busy = true
	var run func(i int, previous *model.WorkflowProgress)
	run = func(i int, previous *model.WorkflowProgress) {
		store.Workflow(steps[i].method, steps[i].params(previous), func(progress model.WorkflowProgress, err error) {
			if wp.closed {
				return
			}
			if err != nil {
				wp.busy = false
				mk.showNotice(model.ErrorText(err), true)
				mk.afterWorkflowRequest(wp)
				return
			}
			if i+1 < len(steps) {
				run(i+1, &progress)
				return
			}
			wp.busy = false
			wp.apply(progress)
			if then != nil {
				then(progress)
			}
			mk.afterWorkflowRequest(wp)
		})
	}
	run(0, wp.progress)
}

func (mk *marketplace) afterWorkflowRequest(wp *workflowPage) {
	if wp.fetchAgain {
		wp.fetchAgain = false
		mk.refreshWorkflow(wp)
	}
}

// refreshWorkflow reads the setup again after something it shows changed.
func (mk *marketplace) refreshWorkflow(wp *workflowPage) {
	if wp.progress == nil || wp.closed {
		return
	}
	if wp.busy || wp.fetching {
		wp.fetchAgain = true
		return
	}
	wp.fetching = true
	id := wp.progress.Setup.ID
	store.Workflow("get", map[string]any{"id": id}, func(progress model.WorkflowProgress, err error) {
		wp.fetching = false
		if err == nil && !wp.closed && wp.progress != nil && progress.Setup.ID == wp.progress.Setup.ID {
			wp.apply(progress)
		}
		mk.afterWorkflowRequest(wp)
	})
}

func (wp *workflowPage) apply(progress model.WorkflowProgress) {
	wp.progress = &progress
	for _, q := range progress.Setup.Pack.Questions {
		if wp.answers[q.ID] == nil {
			answer := progress.Setup.Answers[q.ID]
			wp.answers[q.ID] = &answer
		}
	}
	for _, specialist := range progress.Specialists {
		if _, ok := wp.bots[specialist.ID]; !ok {
			wp.bots[specialist.ID] = specialist.SelectedID
		}
	}
}

func (wp *workflowPage) answer(id string) string {
	if v := wp.answers[id]; v != nil {
		return strings.TrimSpace(*v)
	}
	return ""
}

// edited is the page showing answers or bots that running the sample has not saved yet.
func (wp *workflowPage) edited() bool {
	if wp.progress == nil {
		return false
	}
	for _, q := range wp.progress.Setup.Pack.Questions {
		if wp.answer(q.ID) != wp.progress.Setup.Answers[q.ID] {
			return true
		}
	}
	for _, specialist := range wp.progress.Specialists {
		if wp.bots[specialist.ID] != specialist.SelectedID {
			return true
		}
	}
	return false
}

func (mk *marketplace) runWorkflowSample(wp *workflowPage) {
	id := wp.progress.Setup.ID
	answers, bots := map[string]string{}, map[string]string{}
	for _, q := range wp.progress.Setup.Pack.Questions {
		answers[q.ID] = wp.answer(q.ID)
	}
	for role, bot := range wp.bots {
		if bot != "" {
			bots[role] = bot
		}
	}
	mk.performWorkflow(wp, []workflowStep{
		{"configure", func(*model.WorkflowProgress) map[string]any {
			return map[string]any{"id": id, "answers": answers, "bot_ids": bots}
		}},
		{"sample", func(*model.WorkflowProgress) map[string]any { return map[string]any{"id": id} }},
	}, func(saved model.WorkflowProgress) {
		// Saved now: a new bot is a bot of its own, and the menus show what setup holds.
		wp.bots = map[string]string{}
		wp.apply(saved)
	})
}

func (mk *marketplace) turnOnWorkflow(wp *workflowPage) {
	setup := wp.progress.Setup
	sample := *setup.Sample
	var steps []workflowStep
	if sample.State != "reviewed" {
		steps = append(steps, workflowStep{"review", func(*model.WorkflowProgress) map[string]any {
			return map[string]any{"id": setup.ID, "job_id": sample.JobID}
		}})
	}
	steps = append(steps, workflowStep{"enable", func(*model.WorkflowProgress) map[string]any { return map[string]any{"id": setup.ID} }})
	mk.performWorkflow(wp, steps, func(done model.WorkflowProgress) {
		chatID := ""
		if done.Setup.Sample != nil {
			chatID = done.Setup.Sample.ChatID
		}
		mk.finish(chatID)
	})
}

func (mk *marketplace) cancelWorkflow(wp *workflowPage) {
	setup := wp.progress.Setup
	wasOn := setup.Phase == "enabled"
	mk.performWorkflow(wp, []workflowStep{{"cancel", func(*model.WorkflowProgress) map[string]any { return map[string]any{"id": setup.ID} }}},
		func(model.WorkflowProgress) {
			if wasOn {
				mk.showNotice(L("%@ is off.", wp.pack.Name), false)
			} else {
				mk.showNotice(L("Setup cancelled. Your answers are kept for next time."), false)
			}
			wp.closed = true
			mk.goBack()
		})
}

func (mk *marketplace) chooseWorkflowAccount(wp *workflowPage, connection model.WorkflowConnection, accountID string) {
	id := wp.progress.Setup.ID
	mk.performWorkflow(wp, []workflowStep{{"connection", func(*model.WorkflowProgress) map[string]any {
		return map[string]any{"id": id, "service_id": connection.ServiceID, "plugin_id": accountID}
	}}}, nil)
}

// addWorkflowAccount installs the service's plugin on the Runner for this workflow.
func (mk *marketplace) addWorkflowAccount(wp *workflowPage, connection model.WorkflowConnection) {
	id := wp.progress.Setup.ID
	mk.performWorkflow(wp, []workflowStep{{"connection", func(*model.WorkflowProgress) map[string]any {
		return map[string]any{"id": id, "service_id": connection.ServiceID}
	}}}, nil)
}

func (mk *marketplace) workflowPage(c *ui.Context, page *marketPage) {
	p := colors(c)
	wp := page.workflow
	on := mk.runner()
	// The page follows the Runner picked in the top bar; each Runner has its own setup.
	if wp.progress != nil && on != nil && wp.progress.Setup.RunnerID != on.ID && !wp.busy {
		wp.bots = map[string]string{}
		mk.startWorkflow(wp)
	}
	byline := L("Pair a Runner first.")
	if on != nil {
		byline = L("Runs on %@", on.Name)
	}
	ui.Column(c).Gap(22).Children(func() {
		ui.Column(c).Gap(14).Children(func() {
			ui.Row(c).Gap(14).Padding(0, 12).Children(func() {
				marketPluginIcon(c, wp.pack.ID, firstNonEmpty(wp.pack.SymbolName, "sparkles"), 56)
				ui.Column(c).Gap(3).Grow(1).Shrink(1).MinWidth(0).Children(func() {
					ui.Text(c, wp.pack.Name).FontSize(20).FontWeight(600)
					ui.Text(c, byline).FontSize(12.5).TextColor(p.Label2)
				})
				ui.Row(c.Key("workflow-actions")).Gap(8).Children(func() { mk.workflowActions(c, wp) })
			})
			ui.Text(c, wp.pack.Outcome).Padding(0, 12).FontSize(13).LineHeight(1.4).TextColor(p.Label2).Selectable()
		})
		if wp.progress == nil {
			return
		}
		progress := wp.progress
		setup := progress.Setup
		locked := setup.Phase == "enabled" || progress.IsRunning || wp.busy
		mk.workflowSetupCard(c, wp, locked)
		if len(progress.Connections) > 0 {
			section(c, L("Accounts"), sectionHeading, nil, func(k *card) {
				for _, connection := range progress.Connections {
					ui.Box(c.Key("account:" + connection.ServiceID)).Children(func() { mk.workflowAccountRow(c, k, wp, connection, locked) })
				}
			})
		}
		if sample := setup.Sample; sample != nil {
			mk.workflowSampleCard(c, wp, sample)
		}
		if len(progress.Routines) > 0 {
			section(c, L("Schedule"), sectionHeading, nil, func(k *card) {
				for _, routine := range progress.Routines {
					o := statusRowOptions{Symbol: "pause.circle", Title: routine.Name, Subtitle: model.Schedule(routine.ScheduleText), State: L("Off")}
					if routine.IsEnabled {
						o.Symbol, o.State = "clock", L("On")
					}
					ui.Box(c.Key("routine:" + routine.ID)).Children(func() { statusRow(c, k, o) })
				}
			})
		}
		// The page's one button apart from the rest: Cancel Setup, or Turn Off once it is on.
		if len(setup.BotIDs) > 0 || setup.Sample != nil {
			title := L("Cancel Setup")
			if setup.Phase == "enabled" {
				title = L("Turn Off Workflow")
			}
			ui.Row(c).Padding(0, 12).Children(func() {
				if pushButton(c.Key("workflow-cancel"), title, pushOptions{Kind: buttonDestructive, Disabled: wp.busy}).Clicked() {
					mk.cancelWorkflow(wp)
				}
			})
		}
	})
}

// workflowActions is what can be done next, at the header's trailing end: run the sample, then turn
// its schedule on or leave it off.
func (mk *marketplace) workflowActions(c *ui.Context, wp *workflowPage) {
	if wp.busy {
		spinner(c, 16).Label(L("Loading…"))
	}
	if wp.progress == nil {
		if !wp.busy && mk.runner() != nil && pushButton(c, L("Try Again"), pushOptions{Large: true}).Clicked() {
			mk.startWorkflow(wp)
		}
		return
	}
	progress := wp.progress
	setup := progress.Setup
	if setup.Phase == "enabled" {
		return
	}
	hasResult := setup.Sample != nil && (setup.Sample.State == "ready" || setup.Sample.State == "reviewed") && len(progress.SampleMessages) > 0 && !progress.IsRunning
	if hasResult && !wp.edited() {
		if pushButton(c.Key("not-now"), L("Not Now"), pushOptions{Large: true, Disabled: wp.busy}).Clicked() {
			mk.finish(setup.Sample.ChatID)
		}
		if pushButton(c.Key("turn-on"), L("Turn On Schedule"), pushOptions{Kind: buttonPrimary, Large: true, Disabled: wp.busy}).Clicked() {
			mk.turnOnWorkflow(wp)
		}
		return
	}
	filled := !slices.ContainsFunc(setup.Pack.Questions, func(q model.WorkflowQuestion) bool { return wp.answer(q.ID) == "" })
	connected := !slices.ContainsFunc(progress.Connections, func(connection model.WorkflowConnection) bool {
		account := connection.Account()
		return account == nil || account.State != model.PluginReady
	})
	tip := ""
	if !filled {
		tip = L("Fill in the setup first.")
	} else if !connected {
		tip = L("Connect the accounts first.")
	}
	if pushButton(c.Key("run-sample"), L("Run Sample"), pushOptions{Kind: buttonPrimary, Large: true, Disabled: wp.busy || progress.IsRunning || !filled || !connected, Tooltip: tip}).Clicked() {
		mk.runWorkflowSample(wp)
	}
}

// workflowSetupCard holds the questions, the bot, and the account to use of several.
func (mk *marketplace) workflowSetupCard(c *ui.Context, wp *workflowPage, locked bool) {
	p := colors(c)
	progress := wp.progress
	section(c, Lc("Setup", "workflow questions"), sectionHeading, nil, func(k *card) {
		for _, q := range progress.Setup.Pack.Questions {
			value := wp.answers[q.ID]
			r := k.row(rowBox(c.Key("answer:" + q.ID)))
			r.Children(func() {
				rowKey(c, q.Label).Width(76)
				field := ui.TextInputBase(c, value).Grow(1).Shrink(1).MinWidth(0).FontSize(12).TextColor(p.Label).FocusRing(false).Label(q.Label)
				if q.Placeholder != "" {
					field.Placeholder(q.Placeholder)
				}
				if locked {
					field.ReadOnly(true).TextColor(p.Label2)
				}
			})
		}
		for _, specialist := range progress.Specialists {
			label := specialist.Name
			if len(progress.Specialists) == 1 {
				label = L("Bot")
			}
			var options []popUpOption
			// The new bot is a choice only while setup would add one; once it has, it is a bot.
			if specialist.SelectedID == "" {
				options = append(options, popUpOption{Value: "", Label: L("%@ (new)", specialist.Name)})
			}
			for i, bot := range specialist.Choices {
				options = append(options, popUpOption{Value: bot.ID, Label: bot.Name, Separated: i == 0 && specialist.SelectedID == ""})
			}
			ui.Box(c.Key("bot:" + specialist.ID)).Children(func() {
				if picked, changed := popUpRow(c, k, label, popUp{Options: options, Value: wp.bots[specialist.ID], Disabled: locked}, true); changed {
					wp.bots[specialist.ID] = picked
				}
			})
		}
		// An account of several is the user's to pick; one is simply used.
		for _, connection := range progress.Connections {
			if len(connection.Choices) < 2 {
				continue
			}
			var options []popUpOption
			current := ""
			if account := connection.Account(); account != nil {
				current = account.ID
			} else {
				options = append(options, popUpOption{Value: "", Label: L("Choose…")})
			}
			for _, account := range connection.Choices {
				options = append(options, popUpOption{Value: account.ID, Label: firstNonEmpty(account.AccountName, account.Name)})
			}
			ui.Box(c.Key("pick:" + connection.ServiceID)).Children(func() {
				if picked, changed := popUpRow(c, k, connection.Name, popUp{Options: options, Value: current, Disabled: locked}, true); changed && picked != "" {
					mk.chooseWorkflowAccount(wp, connection, picked)
				}
			})
		}
	})
}

// workflowAccountRow is an account the workflow needs: its short state, or the one thing to do
// about it. A click on it opens the plugin's own sheet.
func (mk *marketplace) workflowAccountRow(c *ui.Context, k *card, wp *workflowPage, connection model.WorkflowConnection, locked bool) {
	p := colors(c)
	account := connection.Account()
	o := statusRowOptions{Symbol: "puzzlepiece.extension", PluginID: connection.ServiceID, Title: connection.Name}
	switch {
	case account == nil && connection.SelectedID != "":
		// Just added: the Runner has yet to report it.
		o.State = L("Connecting…")
	case account == nil && len(connection.Choices) > 1:
		o.State = L("Not chosen")
	case account == nil && connection.Available:
		if !locked {
			o.ActionTitle = L("Add")
		}
	case account == nil:
		o.State = L("Not available")
		o.StateDetail = L("%@ isn't in the marketplace yet. This setup waits for it.", connection.Name)
	default:
		o.Subtitle, o.Clickable, o.Tooltip = account.AccountName, true, L("Open %@", account.Name)
		switch account.State {
		case model.PluginReady:
			o.State = L("Connected")
		case model.PluginNeedsAuth, model.PluginInsufficientAccess:
			o.ActionTitle = L("Sign In")
		case model.PluginNeedsSetup:
			o.ActionTitle = L("Set Up")
		case model.PluginConnecting:
			o.State = L("Connecting…")
		default:
			o.State, o.StateColor, o.StateDetail = L("Can't connect"), &p.Red, account.Detail
		}
	}
	_, result := statusRow(c, k, o)
	switch {
	case (result.Clicked || result.Action) && account != nil:
		if on := mk.runner(); on != nil {
			mk.w.presentPlugin(account.ID, on, "", "")
		}
	case result.Action:
		mk.addWorkflowAccount(wp, connection)
	}
}

// workflowSampleCard is the sample: its replies as the chat shows them, or how it is going.
func (mk *marketplace) workflowSampleCard(c *ui.Context, wp *workflowPage, sample *model.WorkflowSample) {
	p := colors(c)
	progress := wp.progress
	bot := ""
	for _, specialist := range progress.Specialists {
		for _, choice := range specialist.Choices {
			if choice.ID == sample.BotID {
				bot = choice.Name
			}
		}
	}
	section(c, L("Sample"), sectionHeading, nil, func(k *card) {
		switch {
		case progress.IsRunning:
			k.row(ui.Row(c).Gap(8).Padding(12, 12)).Children(func() {
				spinner(c, 14)
				ui.Text(c, L("%@ is working on it…", bot)).FontSize(textCaption).TextColor(p.Label2)
			})
		case (sample.State == "ready" || sample.State == "reviewed") && len(progress.SampleMessages) > 0:
			k.row(ui.Column(c).Gap(16).Padding(12, 12)).Children(func() {
				for _, message := range progress.SampleMessages {
					ui.Box(c.Key(message.ID)).Children(func() { markdownView(c, message.Body.Text, markdownOptions{}) })
				}
			})
		default:
			noteRow(c, k, L("The sample didn't finish. %@'s chat says what happened.", bot), nil)
		}
	})
}
