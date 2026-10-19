package main

import (
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The Bots and Devices panes, on the Device picked in the header: the bots assigned to a Runner,
// and the Device itself.

func (s settingsPane) bots(c *ui.Context, m *mainWindow) {
	p := colors(c)
	device := store.Device(m.settingsDeviceID)
	title := L("Bots")
	if device != nil {
		title = L("Bots on %@", device.Name)
	}
	s.frame(c, string(model.PaneBots)+"|"+m.settingsDeviceID, func() {
		s.section(c, title, nil, func(k *card) {
			settingsRunnerRows(c, k, device, func() {
				bots := store.BotsOn(device.ID)
				for _, bot := range bots {
					rowElement, row := botRow(c, k, bot, botRowOptions{
						Detail:           model.ProviderName(bot.Provider, store.Providers),
						AccessorySymbol:  "bubble.left",
						AccessoryTooltip: L("Open chat"),
						Clickable:        true,
					})
					s.mark(c, rowElement, bot.Name)
					if row.Clicked || row.Accessory {
						m.open(store.DM(bot.ID))
					}
				}
				if len(bots) == 0 {
					keyValueRow(c, k, L("No bots assigned"), "", false, &p.Label2)
				}
				addElement, add := actionRow(c, k, L("New"), actionRowOptions{Tint: &p.Label2, Action: L("New Bot…")})
				s.mark(c, addElement, L("New"))
				if add.Action {
					m.newBot()
				}
			})
		})
		s.footnote(c, L("A bot runs on the Runner it is assigned to, with your account's credentials and that Runner's plugins."))
	})
}

// settingsServiceState is what the picked Runner said of `lorca service`, asked once each time
// the Devices pane shows it.
type settingsServiceState struct {
	asked  string
	status *model.ServiceStatus
}

// statusOf asks an online Runner how its service stands, and is what it answered, if it did.
func (s *settingsServiceState) statusOf(device *model.Device) *model.ServiceStatus {
	if !device.IsStandaloneRunner() || device.Status != model.StatusOnline {
		return nil
	}
	if s.asked != device.ID {
		s.asked, s.status = device.ID, nil
		id := device.ID
		store.RunnerServiceStatus(id, func(status model.ServiceStatus, err error) {
			if err == nil && s.asked == id {
				s.status = &status
			}
		})
	}
	return s.status
}

// device is the picked Device itself: what it is, whether it is online, its machine key, whether
// `lorca service` keeps a Runner's CLI running, and Unpair.
func (s settingsPane) device(c *ui.Context, m *mainWindow) {
	p := colors(c)
	device := store.Device(m.settingsDeviceID)
	s.frame(c, string(model.PaneDevice)+"|"+m.settingsDeviceID, func() {
		if device == nil {
			return
		}
		service := m.settings.service.statusOf(device)
		settingsDeviceHeader(c, device, s.w)
		s.section(c, L("Machine"), nil, func(k *card) {
			unknown := device.OS == model.OSUnknown
			if unknown {
				noteRow(c, k, model.UnknownDeviceNote(), nil)
			}
			machineKey := machineKeyEntry().row
			s.mark(c, keyValueRow(c, k, machineKey, device.MachineKey, true, nil), machineKey)
			if !unknown {
				s.mark(c, keyValueRow(c, k, L("OS"), device.OS.DisplayName()+" · "+device.OSVersion, false, nil), L("OS"))
			}
			if device.App != nil {
				s.appUpdateRow(c, k, device, *device.App)
			}
			if device.Update != nil {
				s.cliUpdateRow(c, k, device, *device.Update)
			}
			role := L("Device · never runs bots")
			if device.IsRunner() {
				role = L("Runner · runs bots with its own credentials")
			}
			s.mark(c, keyValueRow(c, k, L("Role"), role, false, nil), L("Role"))
			if service != nil {
				value := L("Not installed")
				switch {
				case service.Running:
					value = L("Running")
				case service.Installed:
					value = L("Installed, not running")
				}
				s.mark(c, keyValueRow(c.Key("service"), k, L("Background service"), value, false, nil), L("Background service"))
			}
			lastSeen := model.LastSeen(device.LastSeen)
			if device.Status == model.StatusOnline {
				lastSeen = L("Active now")
			}
			s.mark(c, keyValueRow(c, k, L("Last seen"), lastSeen, false, nil), L("Last seen"))
			s.mark(c, keyValueRow(c, k, L("Relay"), settingsRelayText(), true, nil), L("Relay"))
			if device.IsThisDevice && store.IsIdentityDevice {
				phrase := backupPhraseEntry().row
				phraseElement, show := actionRow(c.Key("phrase"), k, phrase, actionRowOptions{Value: L("Restores your identity on another computer"), Tint: &p.Label2, Action: L("Show…")})
				s.mark(c, phraseElement, phrase)
				if show.Action {
					s.w.confirmShowPhrase()
				}
			}
			pairing := pairingEntry().row
			unpairElement, unpair := actionRow(c.Key("pairing"), k, pairing, actionRowOptions{Value: L("Paired to this account"), Tint: &p.Label2, Action: L("Unpair…")})
			s.mark(c, unpairElement, pairing)
			if unpair.Action {
				s.w.confirmUnpair(device)
			}
		})
		if service != nil && !service.Installed {
			s.footnote(c, L("Run lorca service install in Terminal on %@ to keep its bots running while Lorca is closed.", device.Name))
		}
	})
}

// settingsRelayText is the relay this computer syncs through, and how that goes.
func settingsRelayText() string {
	url := store.RelayURL
	switch {
	case url == "":
		return L("Not configured")
	case store.RelayUpdateRequired:
		return L("%@ · update Lorca to sync", url)
	case store.RelayConnected:
		return url
	case store.RelayError != "":
		// Why the last try to connect failed, in the CLI's words.
		return url + " · " + store.RelayError
	}
	return L("%@ · offline", url)
}

// settingsDeviceHeader is the Device's symbol beside its name, its model and system, and a dot and
// words for whether it is online. Rename… beside the name asks for a new one.
func settingsDeviceHeader(c *ui.Context, device *model.Device, w *appWindow) {
	p := colors(c)
	status, tint := "", p.Label2
	switch device.Status {
	case model.StatusOnline:
		tint = p.Green
		switch {
		case device.IsThisDevice:
			status = L("This computer · CLI running")
		case device.IsRunner():
			status = L("Online · paired")
		default:
			status = L("Online · paired · not a Runner")
		}
	case model.StatusPairing:
		status, tint = L("Pairing…"), p.Orange
	default:
		status = model.LastSeen(device.LastSeen)
		if device.IsRunner() {
			status += L(" · jobs wait on the relay")
		}
	}
	// Presence as a small dot: green online, orange pairing, gray offline.
	dot := p.Label3
	if device.Status == model.StatusOnline || device.Status == model.StatusPairing {
		dot = tint
	}
	ui.Row(c).AlignItems(ui.Start).Gap(14).Children(func() {
		ui.Row(c).Size(44, 44).Margin(4, 0, 0, 0).Center().TextColor(p.Label2).Children(func() {
			symbol(c, device.Symbol(), 40, 1.4)
		})
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(3).Children(func() {
			ui.Row(c).Gap(10).MinWidth(0).Children(func() {
				ui.Text(c, device.Name).FontSize(22).FontWeight(600).MaxLines(1).Shrink(1).MinWidth(0)
				// A machine that never said what it is has no name of its own to change.
				if device.OS != model.OSUnknown && pushButton(c, L("Rename…"), pushOptions{Small: true}).Clicked() {
					renameDevice(w, device)
				}
			})
			if device.OS != model.OSUnknown {
				ui.Text(c, device.Model+" · "+device.OSVersion).FontSize(12.5).TextColor(p.Label2)
			}
			ui.Row(c).Gap(6).Margin(5, 0, 0, 0).Children(func() {
				ui.Box(c).Size(8, 8).Radius(4).Background(dot)
				ui.Text(c, status).FontSize(11.5).FontWeight(500).TextColor(tint)
			})
		})
	})
}

// renameDevice asks for the Device's name, which every paired Device shows. A blank name takes back
// the name the machine goes by.
func renameDevice(w *appWindow, device *model.Device) {
	id, value := device.ID, device.Name
	placeholder := device.MachineName
	if placeholder == "" {
		placeholder = device.Name
	}
	w.showAlert(alertOptions{
		Message:     L("Rename Device"),
		Informative: L("Every paired Device shows this name. Leave it blank to use the name the machine goes by."),
		Buttons:     []alertButton{{Title: L("Rename")}, {Title: L("Cancel")}},
		Accessory: func(c *ui.Context) {
			textField(c, &value, fieldOptions{Placeholder: placeholder, AutoFocus: true, Label: L("Name")})
		},
	}, func(answer int) {
		if answer != 0 || strings.TrimSpace(value) == device.Name {
			return
		}
		store.SetDeviceName(id, value, func(err error) {
			if err != nil {
				w.showAlert(alertOptions{Message: L("Couldn't rename it"), Informative: model.ErrorText(err)}, nil)
			}
		})
	})
}

// updateRowValue is a version and how its update goes, for the app's and the CLI's rows, and
// whether to offer Update: while a newer version waits and the Runner is online.
func updateRowValue(device *model.Device, version string, update model.DeviceUpdate) (value string, offersUpdate bool) {
	latest := update.Latest
	switch {
	case update.State == "installing":
		return L("%@ · Installing %@…", version, latest), false
	case update.State == "restarting":
		return L("%@ · Restarts into %@ once no bot is at work", version, latest), false
	case update.State == "installed":
		return L("%@ · %@ is installed; restart lorca serve to run it", version, latest), false
	case latest != "":
		return L("%@ · %@ is available", version, latest), device.Status == model.StatusOnline
	case update.Error != "":
		return version + " · " + update.Error, false
	case update.Auto:
		return L("%@ · Up to date", version), false
	}
	return L("%@ · Up to date · automatic updates off", version), false
}

// appUpdateRow is the version of the desktop app on a Runner, how its update goes, and Update while
// a newer version waits and the Runner is online. This computer's Update opens the update window;
// another Runner's app installs its latest version and relaunches once no bot is at work there.
func (s settingsPane) appUpdateRow(c *ui.Context, k *card, device *model.Device, app model.DeviceApp) {
	p := colors(c)
	value, offersUpdate := updateRowValue(device, app.Version, app.Update)
	action := ""
	if offersUpdate {
		action = L("Update")
	}
	rowElement, row := actionRow(c.Key("app"), k, L("Lorca app"), actionRowOptions{Value: value, Tint: &p.Label2, Tooltip: app.Update.Error, Action: action})
	s.mark(c, rowElement, L("Lorca app"))
	if !row.Action {
		return
	}
	if device.IsThisDevice && updatesEnabled() {
		checkForUpdates()
		return
	}
	id, name, w := device.ID, device.Name, s.w
	store.UpdateDeviceApp(id, func(err error) {
		if err != nil {
			w.showAlert(alertOptions{Message: L("Couldn’t update %@", name), Informative: model.ErrorText(err)}, nil)
		}
	})
}

// cliUpdateRow is the version of a CLI that updates itself, how its update goes, and Update while
// a newer release waits and the Runner is online.
func (s settingsPane) cliUpdateRow(c *ui.Context, k *card, device *model.Device, update model.DeviceUpdate) {
	p := colors(c)
	value, offersUpdate := updateRowValue(device, device.Version, update)
	action := ""
	if offersUpdate {
		action = L("Update")
	}
	rowElement, row := actionRow(c, k, L("Lorca CLI"), actionRowOptions{Value: value, Tint: &p.Label2, Tooltip: update.Error, Action: action})
	s.mark(c, rowElement, L("Lorca CLI"))
	if row.Action {
		id, name, w := device.ID, device.Name, s.w
		store.UpdateDevice(id, func(err error) {
			if err != nil {
				w.showAlert(alertOptions{Message: L("Couldn’t update %@", name), Informative: model.ErrorText(err)}, nil)
			}
		})
	}
}

// confirmShowPhrase asks before showing the backup phrase, which the CLI re-derives from the master
// secret this computer keeps, then shows it with Copy.
func (w *appWindow) confirmShowPhrase() {
	w.showAlert(alertOptions{
		Message:     L("Show the backup phrase?"),
		Informative: L("Anyone who sees it can restore your identity and read everything in your account. Make sure nobody else can see your screen."),
		Buttons:     []alertButton{{Title: L("Show")}, {Title: L("Cancel")}},
	}, func(answer int) {
		if answer != 0 {
			return
		}
		store.BackupPhrase(func(phrase []string, err error) {
			if err != nil {
				w.showAlert(alertOptions{Message: L("Couldn’t show the backup phrase"), Informative: model.ErrorText(err)}, nil)
				return
			}
			w.showAlert(alertOptions{
				Message:     L("Your backup phrase"),
				Informative: L("This phrase is your master secret. It re-derives every key and unwraps everything on the relay. Write it down — nobody can reset it for you."),
				Width:       600,
				Buttons:     []alertButton{{Title: L("Done")}},
				Accessory: func(c *ui.Context) {
					ui.Column(c).AlignItems(ui.Start).Gap(12).Children(func() {
						phraseGrid(c, phrase)
						copyButton(c, strings.Join(phrase, " "), copyOptions{Title: L("Copy Phrase"), Bordered: true})
					})
				},
			}, nil)
		})
	})
}

// confirmUnpair asks before unpairing a Device, then unpairs it. Another Device is unpaired by id;
// this one forgets the identity, and the app goes back to onboarding.
func (w *appWindow) confirmUnpair(device *model.Device) {
	// Only the backup phrase brings the identity back once the Device that holds it forgets it.
	holdsIdentity := device.IsThisDevice && store.IsIdentityDevice
	var informative string
	switch {
	case holdsIdentity:
		informative = L("This computer forgets its keys, credentials, and synced chats, and bots assigned to it stop running until you assign them to another Runner. Your other paired Devices keep everything, and you can pair again any time. Because this computer holds your identity, your backup phrase becomes the only way to restore it.")
	case device.IsThisDevice:
		informative = L("This computer forgets its keys, credentials, and synced chats, and bots assigned to it stop running until you assign them to another Runner. Your other paired Devices keep everything, and you can pair again any time.")
	case device.IsRunner():
		informative = L("It loses its keys and synced chats the next time it connects, and bots assigned to it stop running until you assign them to another Runner. You can pair it again any time.")
	default:
		informative = L("It loses its keys and synced chats the next time it connects. You can pair it again any time.")
	}
	message := L("Unpair \"%@\"?", device.Name)
	if device.IsThisDevice {
		message = L("Unpair this computer?")
	}
	style := alertWarning
	if holdsIdentity {
		style = alertCritical
	}
	id, name, this := device.ID, device.Name, device.IsThisDevice
	w.showAlert(alertOptions{
		Message:     message,
		Informative: informative,
		Style:       style,
		Buttons:     []alertButton{{Title: L("Unpair"), Destructive: true}, {Title: L("Cancel")}},
	}, func(answer int) {
		if answer != 0 {
			return
		}
		failed := func(err error) {
			if err != nil {
				w.showAlert(alertOptions{Message: L("Couldn’t unpair %@", name), Informative: model.ErrorText(err)}, nil)
			}
		}
		switch {
		case !this:
			store.UnpairDevice(id, failed)
		case store.IsMock:
			// The demo has no CLI to forget the identity: onboarding opens over the demo account,
			// as Show Onboarding Again opens it, and closing it brings the demo back. It hides
			// this window, so the click returns first.
			post(app.showOnboarding)
		default:
			store.ForgetIdentity(failed)
		}
	})
}
