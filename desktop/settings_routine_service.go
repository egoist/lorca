package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Service discovery is view state only. Installation remains the existing CLI command on
// the selected owned Runner; the UI never installs a service or changes bot assignments.
type runnerServiceState struct {
	id         string
	generation uint64
	checking   bool
	status     *model.ServiceStatus
	failure    string
}

func (s settingsPane) runnerService(c *ui.Context, m *mainWindow, device *model.Device) {
	state := &m.settings.service
	if state.id != device.ID {
		*state = runnerServiceState{id: device.ID, generation: state.generation + 1}
	}
	p := colors(c)
	s.section(c.Key("runner-service"), L("Runner service"), nil, func(k *card) {
		value := L("Check service status on this Runner")
		switch {
		case device.Status != model.StatusOnline:
			value = L("Waiting for Runner")
		case state.checking:
			value = L("Checking…")
		case state.status != nil:
			value = L("Not installed")
			if state.status.Installed {
				value = L("Installed · stopped")
				if state.status.RunningPID != nil {
					value = L("Installed · running")
				}
			}
		}
		action := ""
		if device.Status == model.StatusOnline && !state.checking {
			action = L("Check status")
		}
		_, row := actionRow(c.Key("service-status"), k, L("Service"), actionRowOptions{Value: value, Tint: &p.Label2, Action: action})
		if row.Action {
			state.checking, state.failure = true, ""
			state.generation++
			id, generation := state.id, state.generation
			store.RunnerServiceStatus(id, func(status model.ServiceStatus, err error) {
				// A reply for a previous Device/request never replaces the current pane.
				if state.id != id || state.generation != generation || m.settingsDeviceID != id {
					return
				}
				state.checking = false
				if err != nil {
					state.failure = model.ErrorText(err)
					return
				}
				state.status, state.failure = &status, ""
			})
		}
		noteRow(c, k, L("For an owned computer that stays available, install the standalone CLI and run these commands on that Runner. Keep it powered on and awake. Quit the app or lorca serve before installing the service."), nil)
		install, status := "lorca service install", "lorca service status"
		if state.status != nil {
			if state.status.InstallCommand != "" {
				install = state.status.InstallCommand
			}
			if state.status.StatusCommand != "" {
				status = state.status.StatusCommand
			}
		}
		keyValueRow(c, k, L("Install"), install, true, nil)
		keyValueRow(c, k, L("Status"), status, true, nil)
		if state.status != nil {
			keyValueRow(c, k, L("Log"), state.status.Log, true, nil)
		}
		if state.failure != "" {
			noteRow(c, k, state.failure, &p.Orange)
		}
	})
}
