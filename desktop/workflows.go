package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
	"slices"
	"strings"
)

// Presentation-only state. The CLI owns reuse, encrypted recovery, permissions and scheduling.
type workflowSheet struct {
	w                     *appWindow
	sheet                 *sheet
	openChat              func(string)
	packs                 []model.WorkflowPack
	pack                  *model.WorkflowPack
	runnerID              string
	progress              *model.WorkflowProgress
	answers               map[string]*string
	bots                  map[string]*string
	editing, busy, closed bool
	error                 string
	generation            int
	refreshAgain          bool
	stop                  func()
}

func (w *appWindow) presentWorkflows(pack *model.WorkflowPack, runnerID string, openChat func(string)) *workflowSheet {
	if runnerID == "" {
		if d := store.ThisDevice(); d != nil && d.IsRunner() {
			runnerID = d.ID
		} else if runners := store.Runners(); len(runners) > 0 {
			runnerID = runners[0].ID
		}
	}
	s := &workflowSheet{w: w, openChat: openChat, runnerID: runnerID, answers: map[string]*string{}, bots: map[string]*string{}}
	s.sheet = w.present(s.view, func() {
		s.closed = true
		if s.stop != nil {
			s.stop()
		}
	})
	s.stop = pluginWatchStore(func(e model.Event) {
		switch e.Kind {
		case model.EventRosterChanged, model.EventSnapshotReplaced, model.EventTurnFinished, model.EventConnectionChanged:
			s.refresh()
		}
	})
	if pack != nil {
		s.start(*pack)
	} else {
		s.loadCatalog()
	}
	return s
}

func (s *workflowSheet) loadCatalog() {
	if s.closed || s.busy {
		return
	}
	s.busy = true
	s.error = ""
	s.generation++
	generation := s.generation
	store.Marketplace(func(catalog model.Marketplace, err error) {
		if s.closed || generation != s.generation {
			return
		}
		s.busy = false
		if err != nil {
			s.error = model.ErrorText(err)
			return
		}
		s.packs = catalog.Packs
	})
}

func (s *workflowSheet) start(pack model.WorkflowPack) {
	if s.closed || s.busy {
		return
	}
	s.pack = &pack
	s.perform("start", map[string]any{"pack_id": pack.ID, "runner_id": s.runnerID})
}

func (s *workflowSheet) perform(method string, params map[string]any) {
	if s.closed || s.busy {
		return
	}
	s.busy = true
	s.error = ""
	s.generation++
	generation := s.generation
	store.Workflow(method, params, func(progress model.WorkflowProgress, err error) {
		if s.closed || generation != s.generation {
			return
		}
		s.busy = false
		if err != nil {
			s.error = model.ErrorText(err)
		} else {
			s.progress = &progress
			s.runnerID = progress.Setup.RunnerID
			s.pack = &progress.Setup.Pack
			if method != "get" {
				s.editing = progress.Setup.Phase == "questions"
				s.answers = map[string]*string{}
				s.bots = map[string]*string{}
			}
			s.seedFields()
		}
		if err == nil && !s.editing && (s.refreshAgain || method == "sample") {
			s.refreshAgain = false
			s.refresh()
		}
	})
}

func (s *workflowSheet) seedFields() {
	if s.progress == nil {
		return
	}
	for _, q := range s.progress.Setup.Pack.Questions {
		if s.answers[q.ID] == nil {
			v := s.progress.Setup.Answers[q.ID]
			s.answers[q.ID] = &v
		}
	}
	for _, b := range s.progress.Specialists {
		if s.bots[b.ID] == nil {
			v := b.SelectedID
			s.bots[b.ID] = &v
		}
	}
}
func (s *workflowSheet) refresh() {
	if s.closed || s.progress == nil || s.editing {
		return
	}
	if s.busy {
		s.refreshAgain = true
		return
	}
	s.perform("get", map[string]any{"id": s.progress.Setup.ID})
}
func (s *workflowSheet) configure() {
	if s.progress == nil {
		return
	}
	answers, bots := map[string]string{}, map[string]string{}
	for id, v := range s.answers {
		answers[id] = *v
	}
	for id, v := range s.bots {
		if *v != "" {
			bots[id] = *v
		}
	}
	s.perform("configure", map[string]any{"id": s.progress.Setup.ID, "answers": answers, "bot_ids": bots})
}
func (s *workflowSheet) action(method string) {
	if s.progress == nil {
		return
	}
	params := map[string]any{"id": s.progress.Setup.ID}
	if method == "review" && s.progress.Setup.Sample != nil {
		params["job_id"] = s.progress.Setup.Sample.JobID
	}
	s.perform(method, params)
}
func (s *workflowSheet) connect(serviceID, instanceID string) {
	if s.progress == nil {
		return
	}
	params := map[string]any{"id": s.progress.Setup.ID, "service_id": serviceID}
	if instanceID != "" {
		params["plugin_id"] = instanceID
	} else {
		params["account_name"] = s.progress.Setup.Pack.Name
	}
	s.perform("connection", params)
}
func (s *workflowSheet) finish() {
	chatID := ""
	if s.progress != nil && s.progress.Setup.Sample != nil {
		chatID = s.progress.Setup.Sample.ChatID
	}
	s.sheet.dismiss()
	if s.openChat != nil {
		s.openChat(chatID)
	}
}

func (s *workflowSheet) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	title, subtitle := L("What would you like to accomplish?"), L("Choose a workflow, connect its tools, and review a sample before enabling a schedule.")
	if s.pack != nil {
		title, subtitle = s.pack.Name, s.pack.Outcome
	}
	var leading func()
	if s.pack != nil {
		leading = func() {
			if pushButton(c.Key("workflow-back"), L("Back"), pushOptions{Disabled: s.busy}).Clicked() {
				s.pack = nil
				s.progress = nil
				s.editing = false
				s.answers = map[string]*string{}
				s.bots = map[string]*string{}
				s.loadCatalog()
			}
		}
	}
	result := sheetFrame(c, sheetOptions{Title: title, Subtitle: subtitle, Width: 720, Confirm: L("Close"), NoCancel: true, ReturnInContent: true, Leading: leading}, func() {
		if s.error != "" {
			ui.Text(c, s.error).FontSize(13).TextColor(p.Red).Role(ui.RoleStatus)
		}
		if s.pack == nil {
			s.catalogView(c)
			return
		}
		runnerName := s.runnerID
		if runner := store.Device(s.runnerID); runner != nil {
			runnerName = runner.Name
		}
		ui.Text(c, L("Runs on %@. Closing this page saves your progress.", runnerName)).FontSize(12).TextColor(p.Label2)
		if s.progress == nil {
			if s.busy {
				workflowLoading(c, L("Loading setup…"))
			} else if pushButton(c, L("Try Again"), pushOptions{}).Clicked() {
				s.start(*s.pack)
			}
			return
		}
		if s.busy {
			workflowLoading(c, L("Updating setup…"))
		}
		setup := s.progress.Setup
		if setup.Phase == "cancelled" {
			ui.Text(c, L("Setup is cancelled. Its specialists and connections are kept for reuse, and its imported routines are paused.")).FontSize(13).LineHeight(1.4).TextColor(p.Label2)
			if pushButton(c, L("Resume Setup"), pushOptions{Disabled: s.busy}).Clicked() {
				s.start(setup.Pack)
			}
			return
		}
		if s.editing || setup.Phase == "questions" {
			s.questionsView(c)
		} else {
			s.progressView(c)
		}
		cancel := L("Cancel Setup")
		if setup.Phase == "enabled" {
			cancel = L("Cancel Setup and Pause Imported Routines")
		}
		if pushButton(c.Key("cancel-setup"), cancel, pushOptions{Disabled: s.busy}).Clicked() {
			s.action("cancel")
		}
	})
	if result.Confirmed || result.Cancelled {
		sh.dismiss()
	}
}
func workflowLoading(c *ui.Context, text string) {
	ui.Row(c).Gap(8).Children(func() { spinner(c, 14); ui.Text(c, text).FontSize(12).TextColor(colors(c).Label2) })
}

func (s *workflowSheet) catalogView(c *ui.Context) {
	p := colors(c)
	runners := store.Runners()
	options := make([]popUpOption, 0, len(runners))
	for _, runner := range runners {
		options = append(options, popUpOption{Value: runner.ID, Label: runner.Name})
	}
	if len(options) > 0 {
		if v, changed, _ := popUpButton(c.Key("runner"), popUp{Options: options, Value: s.runnerID, Label: L("Runner"), Style: popUpBordered, Disabled: s.busy}); changed {
			s.runnerID = v
		}
	}
	if s.busy {
		workflowLoading(c, L("Loading workflows…"))
		return
	}
	if len(s.packs) == 0 {
		ui.Text(c, L("This CLI has no guided workflows. Update Lorca to add them.")).FontSize(13).TextColor(p.Label2)
		if pushButton(c, L("Try Again"), pushOptions{}).Clicked() {
			s.loadCatalog()
		}
		return
	}
	workflowPackRows(c, s.packs, func(pack model.WorkflowPack) { s.start(pack) }, s.runnerID == "")
}

func workflowPackRows(c *ui.Context, packs []model.WorkflowPack, open func(model.WorkflowPack), disabled bool) {
	p := colors(c)
	ui.Column(c.Key("workflow-packs")).Gap(10).Children(func() {
		ui.Text(c, L("Guided Workflows")).FontSize(14).FontWeight(600)
		for _, pack := range packs {
			ui.Row(c.Key(pack.ID)).Gap(12).Padding(10).Radius(10).Background(p.Card).Children(func() {
				symbol(c, "sparkles", 24, 1.8).TextColor(p.Accent)
				ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(4).Children(func() {
					ui.Text(c, pack.Name).FontSize(13).FontWeight(600)
					ui.Text(c, pack.Outcome).FontSize(12).TextColor(p.Label2).LineHeight(1.35)
				})
				if pushButton(c, L("Set Up…"), pushOptions{Disabled: disabled, Tooltip: pack.Name}).Clicked() {
					open(pack)
				}
			})
		}
	})
}

func (s *workflowSheet) questionsView(c *ui.Context) {
	p := colors(c)
	ui.Text(c, L("1 · Answer the questions for this workflow")).FontSize(13).FontWeight(600)
	for _, q := range s.progress.Setup.Pack.Questions {
		ui.Column(c.Key("answer:" + q.ID)).Gap(5).Children(func() {
			ui.Text(c, q.Label).FontSize(13)
			textField(c, s.answers[q.ID], fieldOptions{Label: L("Answer for %@", q.Label), Placeholder: q.Placeholder, Disabled: s.busy})
		})
	}
	if len(s.progress.Specialists) > 0 {
		ui.Text(c, L("Specialists · reuse a bot on this Runner or add a suitable one.")).FontSize(12).TextColor(p.Label2)
	}
	for _, b := range s.progress.Specialists {
		state := s.bots[b.ID]
		options := []popUpOption{{Value: "", Label: L("Reuse a suitable bot or add %@", b.Name)}}
		for _, choice := range b.Choices {
			options = append(options, popUpOption{Value: choice.ID, Label: choice.Name})
		}
		if v, changed, _ := popUpButton(c.Key("specialist:"+b.ID), popUp{Options: options, Value: *state, Label: L("Specialist: %@", b.Name), Style: popUpBordered, Disabled: s.busy}); changed {
			*state = v
		}
	}
	if pushButton(c.Key("continue-setup"), L("Continue"), pushOptions{Kind: buttonPrimary, Disabled: s.busy}).Clicked() {
		s.configure()
	}
}

func (s *workflowSheet) progressView(c *ui.Context) {
	p := colors(c)
	progress := s.progress
	connected := 0
	for _, connection := range progress.Connections {
		if connection.State == model.PluginReady {
			connected++
		}
	}
	ui.Text(c, L("2 · Connections · %d of %d ready", connected, len(progress.Connections))).FontSize(13).FontWeight(600)
	for _, connection := range progress.Connections {
		ui.Column(c.Key("connection:" + connection.ServiceID)).Gap(7).Children(func() { s.connectionView(c, connection) })
	}
	if progress.BlockedReason != "" {
		ui.Text(c, progress.BlockedReason).FontSize(12).TextColor(p.Orange).Role(ui.RoleStatus)
	}
	ui.Text(c, L("3 · Run a sample and review the result")).FontSize(13).FontWeight(600)
	if progress.IsRunning {
		workflowLoading(c, L("Running the sample…"))
	} else {
		if sample := progress.Setup.Sample; sample != nil {
			if sample.Error != "" {
				ui.Text(c, sample.Error).FontSize(12).TextColor(p.Red)
			}
			if sample.State == "running" {
				ui.Text(c, L("The sample was interrupted. Run it again to produce a result.")).FontSize(12).TextColor(p.Label2)
			}
			for _, message := range progress.SampleMessages {
				if message.Body.Kind == model.BodyText {
					ui.Box(c.Key(message.ID)).Padding(10).Radius(8).Background(p.Card).Children(func() { ui.Text(c, message.Body.Text).FontSize(13).LineHeight(1.4).Selectable() })
				}
			}
			if sample.State == "ready" && len(progress.SampleMessages) > 0 {
				if pushButton(c.Key("review-result"), L("I Have Reviewed This Result"), pushOptions{Kind: buttonPrimary, Disabled: s.busy}).Clicked() {
					s.action("review")
				}
			}
		}
		run := L("Run Sample")
		if progress.Setup.Sample != nil {
			run = L("Run Another Sample")
		}
		if pushButton(c.Key("run-sample"), run, pushOptions{Disabled: s.busy || !progress.CanSample}).Clicked() {
			s.action("sample")
		}
	}
	if progress.CanEnable && progress.Setup.Sample != nil && progress.Setup.Sample.State == "reviewed" {
		ui.Text(c, L("4 · Choose whether to enable the schedule")).FontSize(13).FontWeight(600)
		for _, routine := range progress.Routines {
			state := L("Paused")
			if routine.IsEnabled {
				state = L("Enabled")
			}
			ui.Text(c, routine.Name+" · "+routine.ScheduleText+" · "+state).FontSize(12).TextColor(p.Label2)
		}
		if progress.Setup.Phase == "enabled" {
			ui.Text(c, L("This workflow's schedules are enabled.")).FontSize(13).TextColor(p.Green)
			if pushButton(c, L("Open Workflow Chat"), pushOptions{Disabled: s.busy}).Clicked() {
				s.finish()
			}
		} else {
			if pushButton(c.Key("enable-schedules"), L("Enable Schedules"), pushOptions{Kind: buttonPrimary, Disabled: s.busy}).Clicked() {
				s.action("enable")
			}
			if pushButton(c.Key("finish-paused"), L("Finish with Schedules Paused"), pushOptions{Disabled: s.busy}).Clicked() {
				s.finish()
			}
		}
	} else {
		ui.Text(c, L("Imported routines stay paused until you review a sample and enable them.")).FontSize(12).TextColor(p.Label2)
	}
	ui.Row(c).Gap(8).Children(func() {
		if progress.Setup.Phase != "enabled" && pushButton(c.Key("edit-setup"), L("Edit Setup"), pushOptions{Disabled: s.busy || progress.IsRunning}).Clicked() {
			s.editing = true
		}
		if pushButton(c.Key("refresh-progress"), L("Refresh Progress"), pushOptions{Disabled: s.busy}).Clicked() {
			s.refresh()
		}
	})
}

func (s *workflowSheet) connectionView(c *ui.Context, connection model.WorkflowConnection) {
	p := colors(c)
	ui.Text(c, connection.Name).FontSize(13).FontWeight(600)
	tint := p.Label2
	if connection.State == model.PluginReady {
		tint = p.Green
	}
	ui.Text(c, connection.Detail).FontSize(12).TextColor(tint)
	disabled := s.busy || s.progress.IsRunning || s.progress.Setup.Phase == "enabled"
	if len(connection.Choices) > 0 {
		options := []popUpOption{{Value: "", Label: L("Choose an account…")}}
		for _, choice := range connection.Choices {
			options = append(options, popUpOption{Value: choice.ID, Label: choice.Label()})
		}
		if v, changed, _ := popUpButton(c.Key("account"), popUp{Options: options, Value: connection.SelectedID, Label: L("%@ account", connection.Name), Style: popUpBordered, Disabled: disabled}); changed && v != "" {
			s.connect(connection.ServiceID, v)
		}
	} else if connection.Available && connection.SelectedID == "" {
		if pushButton(c, L("Add %@", connection.Name), pushOptions{Disabled: disabled}).Clicked() {
			s.connect(connection.ServiceID, "")
		}
	} else if !connection.Available {
		ui.Text(c, L("This integration is unavailable in the current marketplace. Update Lorca or the marketplace, then resume this saved setup.")).FontSize(12).LineHeight(1.4).TextColor(p.Orange)
	}
	if connection.SelectedID != "" && !slices.ContainsFunc(connection.Choices, func(a model.WorkflowAccount) bool { return a.ID == connection.SelectedID }) {
		if pushButton(c, L("Clear Removed Account Selection"), pushOptions{Disabled: disabled}).Clicked() {
			s.perform("clear_connection", map[string]any{"id": s.progress.Setup.ID, "service_id": connection.ServiceID})
		}
	}
	if connection.SelectedID != "" && connection.State != model.PluginReady {
		label := L("Sign In…")
		if connection.State == model.PluginNeedsSetup {
			label = L("Set Up…")
		}
		if pushButton(c, label, pushOptions{Disabled: s.busy}).Clicked() {
			if runner := store.Device(s.runnerID); runner != nil {
				s.w.presentPlugin(connection.SelectedID, runner)
			}
		}
	}
}

func workflowMatchingPacks(packs []model.WorkflowPack, query string) []model.WorkflowPack {
	words := strings.Fields(strings.ToLower(query))
	var found []model.WorkflowPack
	for _, pack := range packs {
		text := strings.ToLower(pack.Name + " " + pack.Outcome + " " + pack.Description)
		if slices.ContainsFunc(words, func(word string) bool { return !strings.Contains(text, word) }) {
			continue
		}
		found = append(found, pack)
	}
	return found
}
