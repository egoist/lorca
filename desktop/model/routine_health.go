package model

import (
	"strings"
	"time"

	// Routine timezones on Windows, which has no IANA database of its own.
	_ "time/tzdata"
)

// RoutineHealth is how a routine's checks and runs have gone, as its Runner records them.
type RoutineHealth struct {
	LastCheckAt   time.Time
	LastSuccessAt time.Time
	// Status is how the last check went: "quiet", "ready", "failed", or "blocked".
	Status                 string
	ConnectionFailures     int
	AuthenticationFailures int
	// ModelStatus and ModelAuthenticationFailures are the runs' own streak with the model
	// provider, which checks do not clear.
	ModelStatus                 string
	ModelAuthenticationFailures int
}

type WireRoutineHealth struct {
	LastCheckAt            *float64 `json:"last_check_at"`
	LastSuccessAt          *float64 `json:"last_success_at"`
	Status                 *string  `json:"status"`
	ConnectionFailures     int      `json:"connection_failures"`
	AuthenticationFailures int      `json:"authentication_failures"`
	Model                  *struct {
		Status                 *string `json:"status"`
		AuthenticationFailures int     `json:"authentication_failures"`
	} `json:"model"`
}

func toRoutineHealth(wire *WireRoutineHealth) RoutineHealth {
	if wire == nil {
		return RoutineHealth{}
	}
	health := RoutineHealth{Status: str(wire.Status), ConnectionFailures: wire.ConnectionFailures, AuthenticationFailures: wire.AuthenticationFailures}
	if wire.LastCheckAt != nil {
		health.LastCheckAt = seconds(*wire.LastCheckAt)
	}
	if wire.LastSuccessAt != nil {
		health.LastSuccessAt = seconds(*wire.LastSuccessAt)
	}
	if wire.Model != nil {
		health.ModelStatus, health.ModelAuthenticationFailures = str(wire.Model.Status), wire.Model.AuthenticationFailures
	}
	return health
}

// RoutineProblem is what went wrong with a routine, in the words the inspector row and the
// routine sheet use, after the macOS app's RoutineProblem.
type RoutineProblem int

const (
	ProblemNone RoutineProblem = iota
	// ProblemSignedOut and ProblemModelSignedOut: three failed sign-ins in a row paused it, of
	// its check or of its runs.
	ProblemSignedOut
	ProblemModelSignedOut
	ProblemOffline
	ProblemCantConnect
	ProblemModelCantConnect
	ProblemSignInFailed
	ProblemModelSignInFailed
	ProblemCheckFailed
	// ProblemCheckBlocked is a check that called something that could change things, or that the
	// bot's Access leaves out.
	ProblemCheckBlocked
)

// Problem is what went wrong, while something did: the CLI's state with the kind of failure.
func (r Routine) Problem() RoutineProblem {
	if r.IsRunning {
		return ProblemNone
	}
	health := r.Health
	model := health.ModelStatus != ""
	switch r.State {
	case "blocked":
		if r.PausedReason != "authentication" {
			return ProblemCheckBlocked
		}
		if health.ModelAuthenticationFailures >= 3 {
			return ProblemModelSignedOut
		}
		return ProblemSignedOut
	case "waiting_for_runner":
		return ProblemOffline
	case "failed":
		switch {
		case model && health.ModelAuthenticationFailures > 0:
			return ProblemModelSignInFailed
		case model:
			return ProblemModelCantConnect
		case health.AuthenticationFailures > 0:
			return ProblemSignInFailed
		case health.ConnectionFailures > 0:
			return ProblemCantConnect
		}
		return ProblemCheckFailed
	}
	return ProblemNone
}

// Text is one or two words for the row and the sheet's State.
func (p RoutineProblem) Text() string {
	switch p {
	case ProblemSignedOut, ProblemModelSignedOut:
		return L("Needs sign-in")
	case ProblemOffline:
		return L("Waiting for Runner")
	case ProblemCantConnect, ProblemModelCantConnect:
		return L("Can’t connect")
	case ProblemSignInFailed, ProblemModelSignInFailed:
		return L("Sign-in failed")
	case ProblemCheckFailed, ProblemCheckBlocked:
		return L("Check failed")
	}
	return ""
}

// NeedsUser is whether the user has to do something; a connection that fails is tried again on
// its own.
func (p RoutineProblem) NeedsUser() bool {
	return p != ProblemNone && p != ProblemCantConnect && p != ProblemModelCantConnect
}

// Explanation is what happened and how to fix it, for the routine sheet.
func (p RoutineProblem) Explanation(bot, runner string) string {
	switch p {
	case ProblemModelSignedOut:
		return L("The model provider turned down three sign-ins in a row, so the routine is paused. Reconnect the provider in Settings, then resume it.")
	case ProblemSignedOut:
		return L("The check couldn’t sign in to a plugin three times in a row, so the routine is paused. Sign in to the plugin again on %@, then resume it.", runner)
	case ProblemOffline:
		return L("%@ is offline, so the routine waits for it. To keep it available while the app is closed, run lorca service install on it.", runner)
	case ProblemModelCantConnect:
		return L("The last run couldn’t reach the model provider. It tries again at the next run, waiting longer after each failure.")
	case ProblemCantConnect:
		return L("The last check couldn’t connect. It tries again at the next check, waiting longer after each failure.")
	case ProblemModelSignInFailed:
		return L("The last run couldn’t sign in to the model provider. Reconnect it in Settings; after three failures in a row the routine pauses.")
	case ProblemSignInFailed:
		return L("The last check couldn’t sign in to a plugin. Sign in to it again on %@; after three failures in a row the routine pauses.", runner)
	case ProblemCheckFailed:
		return L("The check stopped with an error. %@ got the error and can fix the check.", bot)
	case ProblemCheckBlocked:
		return L("The check tried to change something, or to use something this bot's Access leaves out. Ask %@ to fix it.", bot)
	}
	return ""
}

// ScheduleSummary is the schedule in words, with its timezone when this computer keeps other
// hours, now or in half a year: "Weekdays at 9:00 AM (New York time)". An interval counts time,
// whatever the zone.
func (r Routine) ScheduleSummary() string {
	if strings.HasPrefix(r.Schedule, "every ") {
		return r.ScheduleText
	}
	zone, err := time.LoadLocation(r.Timezone)
	if err != nil {
		return r.ScheduleText
	}
	differs := false
	for _, at := range []time.Time{Now(), Now().AddDate(0, 0, 182)} {
		_, there := at.In(zone).Zone()
		_, here := at.Local().Zone()
		differs = differs || there != here
	}
	if !differs {
		return r.ScheduleText
	}
	city := strings.ReplaceAll(r.Timezone[strings.LastIndex(r.Timezone, "/")+1:], "_", " ")
	return L("%@ (%@ time)", r.ScheduleText, city)
}

// LastCheckSummary is "Today 9:00 AM · nothing new" for a routine with a check that has run,
// else "".
func (r Routine) LastCheckSummary() string {
	if !r.HasCheck || r.Health.LastCheckAt.IsZero() {
		return ""
	}
	when := DaySeparator(r.Health.LastCheckAt)
	switch r.Health.Status {
	case "quiet":
		return L("%@ · nothing new", when)
	case "ready":
		return L("%@ · found something", when)
	case "failed", "blocked":
		return L("%@ · failed", when)
	}
	return when
}

// ServiceStatus is whether `lorca service` keeps the CLI running on a Runner.
type ServiceStatus struct {
	Installed bool `json:"installed"`
	Running   bool `json:"running"`
}

// RunnerServiceStatus asks a Runner, through this computer's CLI, how its service stands.
func (s *Store) RunnerServiceStatus(id string, done func(ServiceStatus, error)) {
	if s.IsMock {
		s.post(func() { done(ServiceStatus{}, nil) })
		return
	}
	Async(s, func() (ServiceStatus, error) {
		return call[ServiceStatus](s, "device.service_status", map[string]any{"id": id})
	}, done)
}
