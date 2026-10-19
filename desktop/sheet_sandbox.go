package main

import (
	"slices"
	"strconv"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// A Runner's cloud sandbox set up in a sheet of its own, from Settings → Coding Agents: Vercel
// Sandbox with the user's access token and project, or Cloudflare Sandbox with the address and
// token of the Worker they deployed, and the variables the agents need there. A saved token or
// value never comes back from the Runner: its field stays empty, and empty keeps it. Saving asks
// the provider whether it works, and the sheet stays open saying why when it does not.

type sandboxSheet struct {
	w      *appWindow
	runner *model.Device
	saved  model.CloudSandbox
	done   func(model.CloudSandboxes)

	token, teamID, projectID, image, vcpus, timeout, url string
	env                                                  []model.Pair
	// focusEnv is a new variable whose name takes the keyboard.
	focusEnv bool

	busy   bool
	status *providerStatus
	closed bool
}

// presentCloudSandbox sets up or changes `sandbox` on `runner`; done gets the sandboxes as the
// Runner then answers them.
func (w *appWindow) presentCloudSandbox(runner *model.Device, sandbox model.CloudSandbox, done func(model.CloudSandboxes)) {
	s := &sandboxSheet{w: w, runner: runner, saved: sandbox, done: done}
	settings := sandbox.Settings
	s.teamID, s.projectID, s.image, s.url = settings.TeamID, settings.ProjectID, settings.Image, settings.URL
	if settings.VCPUs > 0 {
		s.vcpus = strconv.Itoa(settings.VCPUs)
	}
	if settings.TimeoutMinutes > 0 {
		s.timeout = strconv.Itoa(settings.TimeoutMinutes)
	}
	// A saved variable shows by name; its value stays on the Runner.
	for _, name := range sandbox.Env {
		s.env = append(s.env, model.Pair{Name: name})
	}
	w.present(s.view, func() { s.closed = true })
}

func (s *sandboxSheet) change() (model.CloudSandboxChange, string) {
	change := model.CloudSandboxChange{Kind: s.saved.Kind, Token: strings.TrimSpace(s.token)}
	number := func(text, problem string) (int, string) {
		text = strings.TrimSpace(text)
		if text == "" {
			return 0, ""
		}
		n, err := strconv.Atoi(text)
		if err != nil || n <= 0 {
			return 0, problem
		}
		return n, ""
	}
	settings := model.CloudSandboxSettings{TeamID: strings.TrimSpace(s.teamID), ProjectID: strings.TrimSpace(s.projectID), Image: strings.TrimSpace(s.image), URL: strings.TrimSpace(s.url)}
	var problem string
	if settings.VCPUs, problem = number(s.vcpus, L("vCPUs is a whole number")); problem != "" {
		return change, problem
	}
	if settings.TimeoutMinutes, problem = number(s.timeout, L("The time limit is a whole number of minutes")); problem != "" {
		return change, problem
	}
	change.Settings = settings
	for _, pair := range s.env {
		pair.Name = strings.TrimSpace(pair.Name)
		if pair.Name == "" {
			continue
		}
		if pair.Value == "" && !slices.Contains(s.saved.Env, pair.Name) {
			return change, L("%@ needs a value", pair.Name)
		}
		change.Env = append(change.Env, pair)
	}
	return change, ""
}

func (s *sandboxSheet) missing() bool {
	needsToken := !s.saved.SetUp && strings.TrimSpace(s.token) == ""
	if s.saved.Kind == "cloudflare" {
		return needsToken || strings.TrimSpace(s.url) == ""
	}
	return needsToken || strings.TrimSpace(s.projectID) == ""
}

// save sends the change, then asks the provider whether it works.
func (s *sandboxSheet) save(sh *sheet) {
	change, problem := s.change()
	if problem != "" {
		s.status = &providerStatus{text: problem, tone: model.ToneRed}
		return
	}
	s.busy, s.status = true, &providerStatus{text: L("Saving…"), spinning: true}
	store.SetCloudSandbox(s.runner.ID, change, func(sandboxes model.CloudSandboxes, err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.busy, s.status = false, &providerStatus{text: model.ErrorText(err), tone: model.ToneRed}
			return
		}
		s.done(sandboxes)
		s.saved = sandboxes.Sandbox(change.Kind)
		s.token = ""
		s.status = &providerStatus{text: L("Checking with %@…", s.saved.Name), spinning: true}
		store.CheckCloudSandbox(s.runner.ID, change.Kind, func(err error) {
			if s.closed {
				return
			}
			s.busy = false
			if err != nil {
				s.status = &providerStatus{text: L("Saved, but %@ did not take it: %@", s.saved.Name, model.ErrorText(err)), tone: model.ToneRed}
				return
			}
			sh.dismiss()
		})
	})
}

func (s *sandboxSheet) confirmRemove(sh *sheet) {
	s.w.showAlert(alertOptions{
		Message:     L("Remove %@ from %@?", s.saved.Name, s.runner.Name),
		Informative: L("Its token and variables leave %@, and agents there run on %@ again. Agents already in a sandbox keep it until they are forgotten.", s.runner.Name, s.runner.Name),
		Style:       alertWarning,
		Buttons:     []alertButton{{Title: L("Remove")}, {Title: L("Cancel")}},
	}, func(index int) {
		if index != 0 || s.closed {
			return
		}
		s.busy, s.status = true, &providerStatus{text: L("Removing…"), spinning: true}
		store.SetCloudSandbox(s.runner.ID, model.CloudSandboxChange{Kind: s.saved.Kind, Remove: true}, func(sandboxes model.CloudSandboxes, err error) {
			if s.closed {
				return
			}
			if err != nil {
				s.busy, s.status = false, &providerStatus{text: model.ErrorText(err), tone: model.ToneRed}
				return
			}
			s.done(sandboxes)
			sh.dismiss()
		})
	})
}

func (s *sandboxSheet) view(c *ui.Context, sh *sheet) {
	name := s.saved.Name
	title := L("Set Up %@", name)
	var leading func()
	if s.saved.SetUp {
		title = name
		leading = func() {
			if pushButton(c, L("Remove…"), pushOptions{Kind: buttonDestructive, Disabled: s.busy}).Clicked() {
				s.confirmRemove(sh)
			}
		}
	}
	subtitle := L("Coding agents on %@ run in Vercel Sandbox, in your own Vercel account: an access token and the project the sandboxes belong to.", s.runner.Name)
	if s.saved.Kind == "cloudflare" {
		subtitle = L("Coding agents on %@ run in Containers of the Worker you deployed from sandboxes/cloudflare in Lorca's repository, in your own Cloudflare account.", s.runner.Name)
	}
	result := sheetFrame(c, sheetOptions{
		Title:           title,
		Subtitle:        subtitle,
		Width:           560,
		Confirm:         L("Save"),
		ConfirmDisabled: s.busy || s.missing(),
		Leading:         leading,
	}, func() {
		s.formView(c)
		providerNote(c, L("Each agent gets a sandbox of its own with a clone of the repository, and pushes from there. Lorca keeps the token and the values on %@, and hides them wherever an agent prints them.", s.runner.Name), nil)
		providerStatusLine(c, s.status)
	})
	if result.Confirmed {
		s.save(sh)
	}
	if result.Cancelled {
		sh.dismiss()
	}
}

func (s *sandboxSheet) formView(c *ui.Context) {
	tokenPlaceholder := ""
	if s.saved.SetUp && s.saved.Token != "" {
		tokenPlaceholder = L("Saved (%@). Leave empty to keep it.", s.saved.Token)
	}
	cloudflare := s.saved.Kind == "cloudflare"
	labels := []string{L("Token"), L("Environment")}
	if cloudflare {
		labels = append(labels, L("Worker"))
	} else {
		labels = append(labels, L("Project ID"), L("Team ID"), L("Base image"), L("vCPUs"), L("Time limit"))
	}
	labelWidth := mcpFormLabelWidth(c, labels...)
	field := func(label string, value *string, o fieldOptions) {
		providerFormRow(c, labelWidth, label, func() {
			o.Label, o.Disabled = label, s.busy
			textField(c, value, o)
		})
	}
	ui.Column(c).Gap(8).Children(func() {
		if cloudflare {
			field(L("Worker"), &s.url, fieldOptions{Placeholder: "https://lorca-sandbox.example.workers.dev", Mono: true, AutoFocus: !s.saved.SetUp})
			field(L("Token"), &s.token, fieldOptions{Placeholder: tokenPlaceholder, Mono: true, Secure: true})
			providerFormNote(c, labelWidth, L("The Worker's LORCA_TOKEN secret."), nil)
		} else {
			field(L("Token"), &s.token, fieldOptions{Placeholder: tokenPlaceholder, Mono: true, Secure: true, AutoFocus: !s.saved.SetUp})
			providerFormNote(c, labelWidth, L("An access token from your Vercel account settings."), nil)
			field(L("Project ID"), &s.projectID, fieldOptions{Placeholder: "prj_…", Mono: true})
			field(L("Team ID"), &s.teamID, fieldOptions{Placeholder: L("team_… (none for a personal account)"), Mono: true})
			field(L("Base image"), &s.image, fieldOptions{Placeholder: L("Vercel's, with Node.js, Python, and coding agents")})
			field(L("vCPUs"), &s.vcpus, fieldOptions{Placeholder: "2"})
			field(L("Time limit"), &s.timeout, fieldOptions{Placeholder: L("45 minutes")})
			providerFormNote(c, labelWidth, L("How long a sandbox runs at most, in minutes, as your plan allows."), nil)
		}
		ui.Row(c).Gap(10).AlignItems(ui.Start).Children(func() {
			p := colors(c)
			label := ui.Text(c, L("Environment")).Width(labelWidth).Shrink(0).FontSize(12).TextColor(p.Label2).SingleLine()
			if len(s.env) > 0 {
				label.Padding(6, 0, 0, 0)
			}
			ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Children(func() { s.envEditor(c) })
		})
		providerFormNote(c, labelWidth, L("GITHUB_TOKEN to clone, push, and open pull requests, and what each agent signs in with, such as ANTHROPIC_API_KEY or OPENAI_API_KEY."), nil)
	})
}

// envEditor is the variables as rows of a name and a hidden value, a link to add one, and a
// button to remove each. A saved variable's value is empty, which keeps it.
func (s *sandboxSheet) envEditor(c *ui.Context) {
	p := colors(c)
	removed, added := -1, false
	ui.Column(c).Gap(6).Label(L("Environment")).Children(func() {
		for i := range s.env {
			pair := &s.env[i]
			ui.Row(c.Key(i)).Gap(6).Children(func() {
				name := textField(c, &pair.Name, fieldOptions{Placeholder: "GITHUB_TOKEN", Mono: true, Disabled: s.busy, Label: L("Name")}).
					MinHeight(24).Padding(2, 7).Grow(1).Basis(0).MinWidth(0)
				if s.focusEnv && i == len(s.env)-1 {
					s.focusEnv = false
					name.Focus()
				}
				placeholder := L("value")
				if slices.Contains(s.saved.Env, strings.TrimSpace(pair.Name)) {
					placeholder = L("saved")
				}
				textField(c, &pair.Value, fieldOptions{Placeholder: placeholder, Mono: true, Secure: true, Disabled: s.busy, Label: L("Value")}).
					MinHeight(24).Padding(2, 7).Grow(1.35).Basis(0).MinWidth(0)
				if hoverButton(c, hoverButtonOptions{Symbol: "minus.circle", Size: 14, Tooltip: L("Remove"), Disabled: s.busy}).Clicked() {
					removed = i
				}
			})
		}
		add := ui.ButtonBase(c).AlignSelf(ui.Start).Gap(4).FontSize(12).FontWeight(500).TextColor(p.Accent).Label(L("Add Variable")).Cursor(ui.CursorPointer)
		if s.busy {
			add.Disabled(true)
		}
		add.Children(func() {
			symbol(c, "plus", 11, 2.4)
			ui.Text(c, L("Add Variable")).SingleLine()
		})
		added = add.Clicked()
	})
	switch {
	case removed >= 0:
		s.env = slices.Delete(slices.Clone(s.env), removed, removed+1)
	case added:
		s.env = append(slices.Clone(s.env), model.Pair{})
		s.focusEnv = true
	}
}
