package model

import "slices"

// CloudSandboxKinds are the cloud sandboxes a Runner's coding agents can run in, set up with the
// user's own account.
var CloudSandboxKinds = []string{"vercel", "cloudflare"}

// CloudSandboxName is a sandbox's product name, which is not translated.
func CloudSandboxName(kind string) string {
	if kind == "cloudflare" {
		return "Cloudflare Sandbox"
	}
	return "Vercel Sandbox"
}

// CloudSandboxSettings is what a set-up sandbox shows of how it is set up: Vercel's team,
// project, image, vCPUs, and time limit, or the Cloudflare Worker's address.
type CloudSandboxSettings struct {
	TeamID         string `json:"team_id"`
	ProjectID      string `json:"project_id"`
	Image          string `json:"image"`
	VCPUs          int    `json:"vcpus"`
	TimeoutMinutes int    `json:"timeout_minutes"`
	URL            string `json:"url"`
}

// CloudSandbox is what `coding.sandboxes` says of one of a Runner's sandboxes: never a token or a
// value, only a token's last characters and the variables' names.
type CloudSandbox struct {
	Kind     string               `json:"kind"`
	Name     string               `json:"name"`
	SetUp    bool                 `json:"set_up"`
	Settings CloudSandboxSettings `json:"settings"`
	// Token is the saved token's last characters, "…a1b2".
	Token string   `json:"token"`
	Env   []string `json:"env"`
}

// CloudSandboxes is a Runner's sandboxes, and where a start that names no place runs: empty for
// the Runner itself.
type CloudSandboxes struct {
	Default   string         `json:"default"`
	Sandboxes []CloudSandbox `json:"sandboxes"`
}

// Sandbox is the sandbox `kind` as the Runner answered it.
func (s CloudSandboxes) Sandbox(kind string) CloudSandbox {
	for _, sandbox := range s.Sandboxes {
		if sandbox.Kind == kind {
			return sandbox
		}
	}
	return CloudSandbox{Kind: kind, Name: CloudSandboxName(kind)}
}

// Detail is the line under a sandbox's name: where it is set up, or that it is not.
func (s CloudSandbox) Detail() string {
	switch {
	case !s.SetUp:
		return L("Not set up")
	case s.Kind == "cloudflare":
		return L("Set up · %@", s.Settings.URL)
	case s.Settings.ProjectID != "":
		return L("Set up · %@", s.Settings.ProjectID)
	}
	return L("Set up")
}

// CloudSandboxChange changes one sandbox: a token or a variable's value left empty keeps the saved
// one, and a variable it does not list goes. Remove takes the sandbox off the Runner.
type CloudSandboxChange struct {
	Kind     string
	Remove   bool
	Token    string
	Settings CloudSandboxSettings
	Env      []Pair
}

// body is the change as `coding.sandboxes.set` takes it.
func (c CloudSandboxChange) body() map[string]any {
	if c.Remove {
		return map[string]any{c.Kind: nil}
	}
	env := make([]map[string]string, 0, len(c.Env))
	for _, pair := range c.Env {
		if pair.Name == "" {
			continue
		}
		entry := map[string]string{"name": pair.Name}
		if pair.Value != "" {
			entry["value"] = pair.Value
		}
		env = append(env, entry)
	}
	fields := map[string]any{"env": env}
	if c.Token != "" {
		fields["token"] = c.Token
	}
	if c.Kind == "cloudflare" {
		fields["url"] = c.Settings.URL
	} else {
		fields["team_id"], fields["project_id"], fields["image"] = c.Settings.TeamID, c.Settings.ProjectID, c.Settings.Image
		fields["vcpus"], fields["timeout_minutes"] = optional(c.Settings.VCPUs), optional(c.Settings.TimeoutMinutes)
	}
	return map[string]any{c.Kind: fields}
}

func optional(n int) any {
	if n <= 0 {
		return nil
	}
	return n
}

// mockSandboxes is the demo's sandboxes on a Runner: none set up until the user sets one up.
func (s *Store) mockSandboxes(runnerID string) CloudSandboxes {
	if saved, ok := s.mockCloudSandboxes[runnerID]; ok {
		return saved
	}
	out := CloudSandboxes{}
	for _, kind := range CloudSandboxKinds {
		out.Sandboxes = append(out.Sandboxes, CloudSandbox{Kind: kind, Name: CloudSandboxName(kind)})
	}
	return out
}

// CloudSandboxes is Runner `runnerID`'s cloud sandboxes, asked there through the relay when it is
// another Device.
func (s *Store) CloudSandboxes(runnerID string, done func(CloudSandboxes, error)) {
	if s.IsMock {
		sandboxes := s.mockSandboxes(runnerID)
		s.post(func() { done(sandboxes, nil) })
		return
	}
	Async(s, func() (CloudSandboxes, error) {
		var answer CloudSandboxes
		err := s.request("coding.sandboxes", map[string]any{"runner_id": runnerID}, &answer)
		return answer, err
	}, done)
}

// SetCloudSandbox sets up, changes, or removes one of Runner `runnerID`'s sandboxes, and answers
// them as they now stand. The token and the values go to the Runner sealed.
func (s *Store) SetCloudSandbox(runnerID string, change CloudSandboxChange, done func(CloudSandboxes, error)) {
	s.setCloudSandboxes(runnerID, change.body(), func(sandboxes *CloudSandboxes) {
		at := slices.IndexFunc(sandboxes.Sandboxes, func(each CloudSandbox) bool { return each.Kind == change.Kind })
		if at < 0 {
			return
		}
		next := CloudSandbox{Kind: change.Kind, Name: CloudSandboxName(change.Kind)}
		if !change.Remove {
			next.SetUp, next.Settings, next.Token = true, change.Settings, "…demo"
			for _, pair := range change.Env {
				next.Env = append(next.Env, pair.Name)
			}
		} else if sandboxes.Default == change.Kind {
			sandboxes.Default = ""
		}
		sandboxes.Sandboxes[at] = next
	}, done)
}

// SetDefaultCloudSandbox is where Runner `runnerID`'s agents run when a bot names no place: a
// sandbox `kind`, or the Runner itself for "".
func (s *Store) SetDefaultCloudSandbox(runnerID, kind string, done func(CloudSandboxes, error)) {
	s.setCloudSandboxes(runnerID, map[string]any{"default": kind}, func(sandboxes *CloudSandboxes) { sandboxes.Default = kind }, done)
}

func (s *Store) setCloudSandboxes(runnerID string, body map[string]any, mock func(*CloudSandboxes), done func(CloudSandboxes, error)) {
	if s.IsMock {
		sandboxes := s.mockSandboxes(runnerID)
		sandboxes.Sandboxes = slices.Clone(sandboxes.Sandboxes)
		mock(&sandboxes)
		s.mockCloudSandboxes[runnerID] = sandboxes
		s.post(func() { done(sandboxes, nil) })
		return
	}
	body["runner_id"] = runnerID
	Async(s, func() (CloudSandboxes, error) {
		var answer CloudSandboxes
		err := s.request("coding.sandboxes.set", body, &answer)
		return answer, err
	}, done)
}

// CheckCloudSandbox asks the provider whether sandbox `kind` on Runner `runnerID` works as it is
// set up: nil when it does, else what the provider said.
func (s *Store) CheckCloudSandbox(runnerID, kind string, done func(error)) {
	if s.IsMock {
		s.post(func() { done(nil) })
		return
	}
	Async(s, func() (struct{}, error) {
		var answer struct {
			OK    bool   `json:"ok"`
			Error string `json:"error"`
		}
		if err := s.request("coding.sandboxes.check", map[string]any{"runner_id": runnerID, "kind": kind}, &answer); err != nil {
			return struct{}{}, err
		}
		if !answer.OK {
			return struct{}{}, &RequestError{answer.Error}
		}
		return struct{}{}, nil
	}, func(_ struct{}, err error) { done(err) })
}
