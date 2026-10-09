package main

import (
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// One installed plugin on a Runner, after the macOS app's PluginViewController: its state, the
// sign-in for a remote server, its variables (a secret is written, never read back), the skills it
// brought, and Remove. It also shows the Always allowed rules for its tools, and for Browser opened
// from a bot's inspector, the bot's profiles.

// pluginSheetWatch holds the sheets that follow the store while they are up.
var pluginSheetWatch struct {
	store    *model.Store
	next     int
	watchers map[int]func(model.Event)
}

// pluginWatchStore calls fn with the store's events until stop is called: a sheet that shows what
// a Runner has reloads it when the Runner's state moves.
func pluginWatchStore(fn func(model.Event)) (stop func()) {
	watch := &pluginSheetWatch
	if watch.store != store {
		watched := store
		watch.store, watch.watchers = store, map[int]func(model.Event){}
		store.Subscribe(func(event model.Event) {
			if watch.store != watched {
				return
			}
			fns := make([]func(model.Event), 0, len(watch.watchers))
			for _, each := range watch.watchers {
				fns = append(fns, each)
			}
			for _, each := range fns {
				each(event)
			}
		})
	}
	watch.next++
	id, watchers := watch.next, watch.watchers
	watchers[id] = fn
	return func() { delete(watchers, id) }
}

// presentPlugin is the sheet of a plugin installed on a Runner, or for one of the Runner's mcp.json
// servers, the server's own. Opened for a bot (from its inspector), Browser's lists the bot's
// profiles, and a screenshot goes to chatID.
func (w *appWindow) presentPlugin(pluginID string, runner *model.Device, botID, chatID string) {
	var installed *model.InstalledPlugin
	for i := range runner.Plugins {
		if runner.Plugins[i].ID == pluginID {
			plugin := runner.Plugins[i]
			installed = &plugin
		}
	}
	if installed != nil && installed.IsMcpServer() {
		w.presentMcpServer(runner, installed.Name)
		return
	}
	s := &pluginSheet{w: w, pluginID: pluginID, runner: runner, installed: installed, values: map[string]string{}, copiedAt: map[string]time.Time{}}
	if bot := store.Bot(botID); bot != nil && pluginID == model.BrowserPluginID {
		s.profiles = newBrowserProfiles(w, bot, runner, chatID)
	}
	s.load()
	// The Runner's state moved: a sign-in finished, a connection failed.
	stop := pluginWatchStore(func(event model.Event) {
		if event.Kind == model.EventRosterChanged || event.Kind == model.EventSnapshotReplaced {
			s.load()
		}
	})
	w.present(s.view, func() {
		s.closed = true
		stop()
		if s.profiles != nil {
			s.profiles.close()
		}
	})
}

// pluginSheet is the plugin sheet's state.
type pluginSheet struct {
	w         *appWindow
	pluginID  string
	runner    *model.Device
	installed *model.InstalledPlugin
	// profiles are Browser's for the bot the sheet was opened for.
	profiles *browserProfiles

	detail    *model.PluginDetail
	loadError string
	// loads is the newest load: only it renders, so an older answer arriving late (a sealed request
	// to another Runner) never covers a newer one, such as the detail with a sign-in code.
	loads  int
	saving bool
	closed bool
	// values are what was typed into the variables' fields.
	values map[string]string
	// copiedAt is when a server's sign-in code was copied, which its row says for a moment.
	copiedAt map[string]time.Time
}

func (s *pluginSheet) name() string {
	if s.detail != nil && s.detail.Status.Name != "" {
		return s.detail.Status.Name
	}
	if s.installed != nil {
		return s.installed.Name
	}
	return s.pluginID
}

func (s *pluginSheet) load() {
	s.loads++
	load := s.loads
	store.PluginDetail(s.pluginID, s.runner.ID, func(detail model.PluginDetail, err error) {
		if s.closed || load != s.loads {
			return
		}
		if err != nil {
			s.loadError = model.ErrorText(err)
			return
		}
		s.detail, s.loadError = &detail, ""
	})
}

// rules are Auto-review's Always allowed rules for the plugin's tools.
func (s *pluginSheet) rules() []model.AutoReviewRule {
	prefix := s.pluginID + "/"
	var out []model.AutoReviewRule
	for _, rule := range store.AutoReview.Rules {
		if strings.HasPrefix(rule.Tool, prefix) {
			out = append(out, rule)
		}
	}
	return out
}

func (s *pluginSheet) resetRules() {
	prefix := s.pluginID + "/"
	review := store.AutoReview
	var kept []model.AutoReviewRule
	for _, rule := range review.Rules {
		if !strings.HasPrefix(rule.Tool, prefix) {
			kept = append(kept, rule)
		}
	}
	review.Rules = kept
	store.SetAutoReview(review)
}

// value is a variable's field: what was typed, else the variable's value, a secret's empty.
func (s *pluginSheet) value(variable model.PluginDetailVariable) string {
	if value, ok := s.values[variable.Name]; ok {
		return value
	}
	if variable.Secret {
		return ""
	}
	return variable.Value
}

func (s *pluginSheet) save() {
	if s.detail == nil {
		return
	}
	variables := s.detail.Variables
	values := map[string]string{}
	for _, variable := range variables {
		value := s.value(variable)
		if value != "" || !variable.Secret {
			values[variable.Name] = value
		}
	}
	s.saving = true
	store.SetPluginVariables(s.pluginID, s.runner.ID, values, func(_ model.InstalledPlugin, err error) {
		if s.closed {
			return
		}
		s.saving = false
		if err != nil {
			s.w.showAlert(alertOptions{Message: L("Couldn't save"), Informative: model.ErrorText(err)}, nil)
			return
		}
		// A secret is written, never read back.
		for _, variable := range variables {
			if variable.Secret {
				delete(s.values, variable.Name)
			}
		}
		s.load()
	})
}

// rename gives a named account a new name when editing ends. Its id, sign-in, and tools stay.
func (s *pluginSheet) rename(name string) {
	if s.detail == nil || name == "" || name == s.detail.Status.AccountName {
		return
	}
	store.RenamePluginAccount(s.pluginID, s.runner.ID, name, func(_ model.InstalledPlugin, err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.w.showAlert(alertOptions{Message: L("Couldn't rename it"), Informative: model.ErrorText(err)}, nil)
			return
		}
		s.load()
	})
}

func (s *pluginSheet) connect() {
	// The Runner notes the sign-in on the plugin, so the State row reads it.
	store.ConnectPlugin(s.pluginID, s.runner.ID, func(err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.w.showAlert(alertOptions{Message: L("Couldn't start the sign-in"), Informative: model.ErrorText(err)}, nil)
			return
		}
		s.load()
	})
}

// signOut forgets a server's sign-in on the Runner; the plugin's next use asks again.
func (s *pluginSheet) signOut(server string) {
	store.SignOutPlugin(s.pluginID, s.runner.ID, server, func(err error) {
		if s.closed {
			return
		}
		if err != nil {
			s.w.showAlert(alertOptions{Message: L("Couldn't sign out of %@", s.name()), Informative: model.ErrorText(err)}, nil)
			return
		}
		s.load()
	})
}

func (s *pluginSheet) confirmRemove(sh *sheet) {
	s.w.showAlert(alertOptions{
		Message:     L("Remove %@ from %@?", s.name(), s.runner.Name),
		Informative: L("Every bot on %@ loses it, and its keys and sign-ins there are forgotten.", s.runner.Name),
		Style:       alertWarning,
		Buttons:     []alertButton{{Title: L("Remove")}, {Title: L("Cancel")}},
	}, func(index int) {
		if index != 0 {
			return
		}
		store.UninstallPlugin(s.pluginID, s.runner.ID, func(err error) {
			if err != nil {
				s.w.showAlert(alertOptions{Message: L("Couldn't remove it"), Informative: model.ErrorText(err)}, nil)
				return
			}
			sh.dismiss()
		})
	})
}

func (s *pluginSheet) view(c *ui.Context, sh *sheet) {
	p := colors(c)
	var parts []string
	if s.installed != nil && s.installed.Description != "" {
		parts = append(parts, s.installed.Description)
	}
	parts = append(parts, L("Installed on %@.", s.runner.Name))
	result := sheetFrame(c, sheetOptions{Title: s.name(), Subtitle: strings.Join(parts, " "), Width: 520, Confirm: L("Done"), NoCancel: true}, func() {
		s.statusSection(c)
		if s.profiles != nil {
			s.profiles.view(c)
		}
		s.signInSection(c)
		if s.detail != nil && len(s.detail.Variables) > 0 {
			s.setupSection(c)
		}
		if s.detail != nil && len(s.detail.Skills) > 0 {
			section(c, L("Skills"), sectionCaption, nil, func(k *card) {
				for _, skill := range s.detail.Skills {
					keyValueRow(c, k, skill.Name, skill.Description, false, nil)
				}
			})
		}
		note := L("Keys and sign-ins are sent sealed to %@ and stay there.", s.runner.Name)
		if s.runner.IsThisDevice {
			note = L("Keys and sign-ins stay on this device.")
		}
		providerNote(c, note, &p.Label2)
		ui.Row(c).Gap(8).Children(func() {
			if s.detail != nil && len(s.detail.Variables) > 0 {
				if pushButton(c, L("Save"), pushOptions{Disabled: s.saving}).Clicked() {
					s.save()
				}
			}
			ui.Spacer(c)
			if pushButton(c, L("Remove…"), pushOptions{}).Clicked() {
				s.confirmRemove(sh)
			}
		})
	})
	if result.Confirmed || result.Cancelled {
		sh.dismiss()
	}
}

// statusSection is the plugin's state, the rules that let its tools run without asking, and its
// site.
func (s *pluginSheet) statusSection(c *ui.Context) {
	p := colors(c)
	// A named account (Gmail · Work) says how it stands in its Account card, and its service's page
	// has the site.
	named := s.loadError == "" && s.isNamedAccount()
	rules := s.rules()
	if named && len(rules) == 0 {
		return
	}
	section(c, L("Status"), sectionCaption, nil, func(k *card) {
		if s.loadError != "" || s.detail == nil {
			value, tint := L("Loading…"), p.Label2
			if s.loadError != "" {
				value, tint = s.loadError, p.Red
			}
			keyValueRow(c, k, L("State"), value, false, &tint)
			return
		}
		detail := s.detail
		if !named {
			tint := p.tone(detail.Status.State.Tone())
			keyValueRow(c, k, L("State"), detail.Status.Detail, false, &tint)
		}
		if len(rules) > 0 {
			prefix := s.pluginID + "/"
			tools := make([]string, 0, len(rules))
			for _, rule := range rules {
				tools = append(tools, strings.TrimPrefix(rule.Tool, prefix))
			}
			_, r := actionRow(c, k, Lc("Always allowed", "plugin tools"), actionRowOptions{Value: strings.Join(tools, ", "), Tint: &p.Label, Action: L("Reset")})
			if r.Action {
				s.resetRules()
			}
		}
		if site := hostOf(detail.Homepage); !named && detail.Homepage != "" && site != "" {
			_, r := actionRow(c, k, L("Site"), actionRowOptions{Value: site, Tint: &p.Label2, Action: L("Open")})
			if r.Action {
				_ = openExternal(detail.Homepage)
			}
		}
	})
}

// signInSection is the sign-in of each of the plugin's servers that signs in with OAuth. A
// device-flow sign-in waits for its code: the code, and a button that copies it and opens the page
// to enter it on.
func (s *pluginSheet) signInSection(c *ui.Context) {
	p := colors(c)
	if s.detail == nil {
		return
	}
	var servers []model.PluginDetailServer
	for _, server := range s.detail.Servers {
		if server.OAuth {
			servers = append(servers, server)
		}
	}
	named, status := s.isNamedAccount(), s.detail.Status
	if len(servers) == 0 && !named {
		return
	}
	title := L("Sign-in")
	if named {
		// A named account's one card: its name, and its sign-in, which says how it stands.
		title = L("Account")
	}
	section(c, title, sectionCaption, nil, func(k *card) {
		if named {
			if name, ok := editableRow(c, k, L("Name"), status.AccountName, L("Work"), false, true); ok {
				s.rename(name)
			}
		}
		for _, server := range servers {
			label := L("Account")
			switch {
			case named:
				label = L("Sign-in")
			case len(servers) > 1:
				label = server.Name
			}
			if server.Code != "" && server.Link != "" {
				copied := false
				if at, ok := s.copiedAt[server.Name]; ok {
					if since := c.Now().Sub(at); since < 1500*time.Millisecond {
						copied = true
						c.After(1500*time.Millisecond - since)
					}
				}
				title := L("Copy code and open %@", firstNonEmpty(hostOf(server.Link), L("link")))
				if copied {
					title = L("Copied")
				}
				_, r := actionRow(c, k, label, actionRowOptions{Value: server.Code, Tint: &p.Label, Mono: true, Action: title, Copied: copied})
				if r.Action {
					c.WriteClipboard(server.Code)
					s.copiedAt[server.Name] = c.Now()
					_ = openExternal(server.Link)
				}
				continue
			}
			value, tint, action, second := L("Not signed in"), p.Label2, L("Sign in"), ""
			if server.SignedIn {
				value, tint, action, second = L("Signed in"), p.Green, L("Sign Out"), L("Sign in again")
			}
			if named && status.State != model.PluginReady && status.State != model.PluginNeedsAuth {
				text, tone := status.ShortStatus()
				value, tint = text, p.tone(tone)
			}
			if named && status.State == model.PluginNeedsSetup {
				// Before its setup, an account has nothing to sign in with.
				action, second = "", ""
			}
			_, r := actionRow(c, k, label, actionRowOptions{Value: value, Tint: &tint, Action: action, Second: second})
			if r.Second {
				s.connect()
			}
			if r.Action {
				if server.SignedIn {
					s.signOut(server.Name)
				} else {
					s.connect()
				}
			}
		}
		// Why an account can't connect, in the server's words.
		if named && status.State == model.PluginError && status.Detail != "" {
			noteRow(c, k, status.Detail, nil)
		}
	})
}

func (s *pluginSheet) isNamedAccount() bool {
	return s.detail != nil && s.detail.Status.AccountName != ""
}

// setupSection is the plugin's variables: a secret is masked and never shown, its field saying
// whether it is set.
func (s *pluginSheet) setupSection(c *ui.Context) {
	p := colors(c)
	section(c, Lc("Setup", "plugin variables"), sectionCaption, nil, func(k *card) {
		for _, variable := range s.detail.Variables {
			key := variable.Name
			if variable.Required {
				key += " *"
			}
			r := k.row(ui.Row(c.Key(variable.Name)).Gap(10).MinHeight(34).Padding(4, 10, 4, 12).MinWidth(0).Label(variable.Name))
			r.Children(func() {
				ui.Text(c, key).Grow(1).Shrink(1).MinWidth(0).Font(monoFont).FontSize(11).TextColor(p.Label2).SingleLine()
				placeholder := ""
				switch {
				case variable.Secret && variable.IsSet:
					placeholder = L("Set · type to replace")
				case !variable.IsSet:
					placeholder = L("Not set")
				}
				value := s.value(variable)
				field := textField(c, &value, fieldOptions{Placeholder: placeholder, Secure: variable.Secret, Mono: true, Label: variable.Name}).
					Width(230).Shrink(0).MinHeight(22).Padding(2, 7).FontSize(11)
				if variable.Description != "" {
					field.Tooltip(variable.Description)
				}
				// Until it is typed in, a field follows the Runner's value.
				if field.Changed() {
					s.values[variable.Name] = value
				}
			})
		}
	})
}
