package main

import (
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// browserProfiles is the Browser plugin sheet's Profiles section for one bot, after the macOS app's
// BrowserProfilesSection: a row per profile with how it stands, + to add one, and on a click or a
// right-click the profile's menu: open it, take it over or hand it back, take a screenshot into the
// chat, close it, or delete it.
type browserProfiles struct {
	w             *appWindow
	botID, chatID string
	botName       string
	runner        *model.Device

	profiles  []model.BrowserProfile
	loaded    bool
	loadError string
	// pending is what a row says while its action runs on the Runner ("Opening…").
	pending map[string]string
	// loads is the newest read: an older answer, or one begun before an action, never covers it.
	loads  int
	timer  *time.Timer
	closed bool
}

func newBrowserProfiles(w *appWindow, bot *model.Bot, runner *model.Device, chatID string) *browserProfiles {
	b := &browserProfiles{w: w, botID: bot.ID, botName: bot.Name, chatID: chatID, runner: runner, pending: map[string]string{}}
	b.load()
	b.schedule()
	return b
}

// close stops asking the Runner once the sheet is down.
func (b *browserProfiles) close() {
	b.closed = true
	if b.timer != nil {
		b.timer.Stop()
		b.timer = nil
	}
}

// schedule asks the Runner again while the sheet is up: on this computer every two seconds,
// through the relay every five, since another Device can change the browser too.
func (b *browserProfiles) schedule() {
	if b.closed || b.timer != nil || store.IsMock {
		return
	}
	every := 5 * time.Second
	if b.runner.IsThisDevice {
		every = 2 * time.Second
	}
	b.timer = time.AfterFunc(every, func() {
		post(func() {
			b.timer = nil
			b.load()
			b.schedule()
		})
	})
}

func (b *browserProfiles) load() {
	if b.closed {
		return
	}
	b.loads++
	load := b.loads
	store.BrowserProfiles(b.botID, func(profiles []model.BrowserProfile, err error) {
		if b.closed || load != b.loads {
			return
		}
		if err != nil {
			b.loadError = model.ErrorText(err)
			return
		}
		b.profiles, b.loaded, b.loadError = profiles, true, ""
	})
}

func (b *browserProfiles) profile(id string) (model.BrowserProfile, bool) {
	for _, profile := range b.profiles {
		if profile.ID == id {
			return profile, true
		}
	}
	return model.BrowserProfile{}, false
}

// stateText is one or two words; orange while the bot waits on the user to hand the browser back.
func (b *browserProfiles) stateText(p *palette, profile model.BrowserProfile) (string, ui.Color) {
	if pending, ok := b.pending[profile.ID]; ok {
		return pending, p.Label2
	}
	switch profile.State {
	case model.BrowserBot:
		return Lc("Open", "browser"), p.Label2
	case model.BrowserTakingOver:
		return L("Taking over…"), p.Label2
	case model.BrowserHuman:
		return L("You have control"), p.Orange
	}
	return Lc("Closed", "browser"), p.Label2
}

// explanation is what the state means for the bot, as the row's tooltip.
func (b *browserProfiles) explanation(profile model.BrowserProfile) string {
	switch profile.State {
	case model.BrowserBot:
		return L("%@'s browser calls use this profile.", b.botName)
	case model.BrowserTakingOver:
		return L("Waiting for %@'s current step in the browser to finish.", b.botName)
	case model.BrowserHuman:
		return L("%@'s browser calls wait until you return the browser.", b.botName)
	}
	return L("Open it on %@ to sign in. %@ can open it too.", b.runner.Name, b.botName)
}

func (b *browserProfiles) view(c *ui.Context) {
	if !b.loaded && b.loadError == "" {
		return
	}
	p := colors(c)
	add := func() {
		if hoverButton(c, hoverButtonOptions{Symbol: "plus", Size: 13, Tooltip: L("Add Profile")}).Clicked() {
			b.add()
		}
	}
	section(c, L("Profiles"), sectionCaption, add, func(k *card) {
		for _, profile := range b.profiles {
			state, tint := b.stateText(p, profile)
			ui.Box(c.Key(profile.ID)).Children(func() {
				row, _ := statusRow(c, k, statusRowOptions{Symbol: "person.crop.circle", Title: profile.Name, State: state, StateColor: &tint, Clickable: true, Tooltip: b.explanation(profile)})
				menu := b.menu(profile)
				row.ContextMenu(menu)
				row.Menu(menu)
			})
		}
		switch {
		case b.loadError != "":
			noteRow(c, k, b.loadError, nil)
		case len(b.profiles) == 0:
			noteRow(c, k, L("A profile keeps sign-ins for %@'s browser. Add one, then open it on %@ to sign in.", b.botName, b.runner.Name), nil)
		}
	})
}

func (b *browserProfiles) menu(profile model.BrowserProfile) func(*ui.Menu) {
	return func(menu *ui.Menu) {
		_, busy := b.pending[profile.ID]
		items := false
		switch profile.State {
		case model.BrowserStopped:
			// A window opens only on the Runner's own screen.
			if b.runner.IsThisDevice {
				if menu.Item(L("Open Browser")).Disabled(busy).Chosen() {
					b.perform("browser.open", profile, nil, L("Opening…"), L("Couldn't open the browser"))
				}
			} else {
				menu.Item(L("Open on %@", b.runner.Name)).Disabled(true)
			}
			items = true
		case model.BrowserBot:
			if menu.Item(L("Take Over")).Disabled(busy).Chosen() {
				b.perform("browser.takeover", profile, nil, L("Taking over…"), L("Couldn't take over the browser"))
			}
			items = true
		case model.BrowserHuman:
			if menu.Item(L("Return to Bot")).Disabled(busy).Chosen() {
				b.perform("browser.resume", profile, map[string]any{"revision": profile.Revision}, L("Returning…"), L("Couldn't hand the browser back"))
			}
			items = true
		}
		if (profile.State == model.BrowserBot || profile.State == model.BrowserHuman) && b.chatID != "" {
			if menu.Item(L("Take Screenshot")).Disabled(busy).Chosen() {
				b.perform("browser.screenshot", profile, map[string]any{"chat_id": b.chatID}, "", L("Couldn't take a screenshot"))
			}
		}
		// Close works while an open or a takeover waits.
		if profile.State != model.BrowserStopped || busy {
			if menu.Item(L("Close Browser")).Chosen() {
				b.perform("browser.stop", profile, nil, L("Closing…"), L("Couldn't close the browser"))
			}
			items = true
		}
		if items {
			menu.Separator()
		}
		if menu.Item(L("Delete…")).Disabled(busy).Chosen() {
			b.confirmDelete(profile)
		}
	}
}

func (b *browserProfiles) perform(method string, profile model.BrowserProfile, params map[string]any, label, failure string) {
	if label != "" {
		b.pending[profile.ID] = label
	}
	// A read that started before this action could describe the browser as it was.
	b.loads++
	all := map[string]any{"session_id": profile.ID}
	for key, value := range params {
		all[key] = value
	}
	store.BrowserProfileAction(method, b.botID, all, func(err error) {
		delete(b.pending, profile.ID)
		if b.closed {
			return
		}
		if err != nil {
			b.w.showAlert(alertOptions{Message: failure, Informative: model.ErrorText(err)}, nil)
		}
		b.load()
	})
}

func (b *browserProfiles) confirmDelete(profile model.BrowserProfile) {
	b.w.showAlert(alertOptions{
		Message:     L("Delete the “%@” profile?", profile.Name),
		Informative: L("Its browser closes, and the sites it signed in to are signed out for %@.", b.botName),
		Style:       alertWarning,
		Buttons:     []alertButton{{Title: L("Delete"), Destructive: true}, {Title: L("Cancel")}},
	}, func(answer int) {
		if answer == 0 {
			b.perform("browser.delete", profile, nil, L("Deleting…"), L("Couldn't delete the profile"))
		}
	})
}

func (b *browserProfiles) add() {
	name := ""
	b.w.showAlert(alertOptions{
		Message:     L("New Profile"),
		Informative: L("%@ keeps the sign-ins you make in it.", b.botName),
		Buttons:     []alertButton{{Title: L("Add")}, {Title: L("Cancel")}},
		Accessory: func(c *ui.Context) {
			textField(c, &name, fieldOptions{Placeholder: Lc("Work", "browser profile"), AutoFocus: true})
		},
	}, func(answer int) {
		name = strings.TrimSpace(name)
		if answer != 0 || name == "" {
			return
		}
		store.BrowserProfileAction("browser.create", b.botID, map[string]any{"name": name}, func(err error) {
			if b.closed {
				return
			}
			if err != nil {
				b.w.showAlert(alertOptions{Message: L("Couldn't add the profile"), Informative: model.ErrorText(err)}, nil)
			}
			b.load()
		})
	})
}
