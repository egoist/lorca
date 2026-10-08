package model

import (
	"cmp"
	"strings"
)

// Runtime health and scheduling are read from the CLI; the UI never schedules work itself.
type WireRoutineHealth struct {
	LastCheckAt   *float64 `json:"last_check_at"`
	LastSuccessAt *float64 `json:"last_success_at"`
	RetryAt       *float64 `json:"retry_at"`
}

type RoutinePolicy struct {
	Timezone        *string
	MissedRunPolicy *string
}

func (r Routine) SchedulingTimezone() string { return cmp.Or(r.Timezone, "UTC") }
func (r Routine) MissedPolicy() string       { return cmp.Or(r.MissedRunPolicy, "coalesce") }

func (r Routine) NextSummary() string {
	if r.NextRunText != "" {
		return r.NextRunText
	}
	if !r.NextRunAt.IsZero() {
		return Upcoming(r.NextRunAt)
	}
	return "—"
}

func (r Routine) StateText() string {
	if r.IsRunning {
		return L("Running…")
	}
	if r.PausedReason == "authentication" {
		return L("Blocked")
	}
	switch r.State {
	case "waiting_for_runner":
		return L("Waiting for Runner")
	case "quiet":
		return L("Quiet · nothing new")
	case "failed":
		return L("Failed")
	case "blocked":
		return L("Blocked")
	}
	if !r.IsEnabled {
		if r.PausedReason == "away" {
			return L("Paused while you were away")
		}
		return L("Paused")
	}
	return L("On")
}

func (r Routine) CanRunNow() bool {
	return !r.IsRunning && (!r.HasRunnerAvailability || r.RunnerAvailable) && r.PausedReason != "authentication"
}

// SetRoutinePolicy validates through the canonical CLI API and applies its confirmed result
// only on the ordered main-thread queue. Health, cursors and Runner assignments are not sent.
func (s *Store) SetRoutinePolicy(id string, patch RoutinePolicy, done func(error)) {
	if s.Routine(id) == nil {
		s.post(func() {
			if done != nil {
				done(&RequestError{L("Request failed")})
			}
		})
		return
	}
	params := map[string]any{"id": id}
	if patch.Timezone != nil {
		params["timezone"] = strings.TrimSpace(*patch.Timezone)
	}
	if patch.MissedRunPolicy != nil {
		params["missed_run_policy"] = *patch.MissedRunPolicy
	}
	if s.routinePolicyRequests == nil {
		s.routinePolicyRequests = map[string]uint64{}
	}
	s.routinePolicyRequests[id]++
	version := s.routinePolicyRequests[id]
	apply := func(wire WireRoutine, err error) {
		if err == nil && wire.ID != id {
			err = &RequestError{L("Request failed")}
		}
		if err == nil && version == s.routinePolicyRequests[id] {
			for i, routine := range s.Routines {
				if routine.ID == id {
					s.Routines[i] = ToRoutine(wire)
					s.emit(Event{Kind: EventRosterChanged})
					break
				}
			}
		}
		if done != nil {
			done(err)
		}
	}
	if s.IsMock {
		// Demo changes use the same main-thread reply boundary; no provider/check is run.
		s.post(func() {
			if version != s.routinePolicyRequests[id] {
				if done != nil {
					done(nil)
				}
				return
			}
			routine := s.Routine(id)
			if routine != nil {
				if patch.Timezone != nil {
					routine.Timezone = params["timezone"].(string)
					routine.NextRunText = ""
				}
				if patch.MissedRunPolicy != nil {
					routine.MissedRunPolicy = params["missed_run_policy"].(string)
				}
				s.emit(Event{Kind: EventRosterChanged})
			}
			if done != nil {
				done(nil)
			}
		})
		return
	}
	Async(s, func() (WireRoutine, error) {
		var reply struct {
			Routine WireRoutine `json:"routine"`
		}
		err := s.request("routines.update", params, &reply)
		return reply.Routine, err
	}, apply)
}

type ServiceStatus struct {
	Installed      bool   `json:"installed"`
	RunningPID     *int   `json:"running_pid"`
	Supervised     bool   `json:"supervised"`
	Log            string `json:"log"`
	InstallCommand string `json:"install_command"`
	StatusCommand  string `json:"status_command"`
	Detail         string `json:"detail"`
}

// RunnerServiceStatus is read-only discovery through the local CLI's existing sealed route.
func (s *Store) RunnerServiceStatus(id string, done func(ServiceStatus, error)) {
	if s.IsMock {
		s.post(func() {
			done(ServiceStatus{InstallCommand: "lorca service install", StatusCommand: "lorca service status", Log: "—"}, nil)
		})
		return
	}
	Async(s, func() (ServiceStatus, error) {
		return call[ServiceStatus](s, "device.service_status", map[string]any{"id": id})
	}, done)
}
