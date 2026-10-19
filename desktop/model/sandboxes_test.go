package model

import (
	"encoding/json"
	"testing"
)

// A change sends a new token or value only when one was typed, so a saved one stays on the
// Runner, and Remove sends the sandbox as null.
func TestACloudSandboxChangeSendsOnlyWhatWasTyped(t *testing.T) {
	change := CloudSandboxChange{
		Kind:     "vercel",
		Settings: CloudSandboxSettings{ProjectID: "prj_1", TimeoutMinutes: 90},
		Env:      []Pair{{Name: "GITHUB_TOKEN"}, {Name: "ANTHROPIC_API_KEY", Value: "sk-ant"}, {Name: ""}},
	}
	got, _ := json.Marshal(change.body())
	want := `{"vercel":{"env":[{"name":"GITHUB_TOKEN"},{"name":"ANTHROPIC_API_KEY","value":"sk-ant"}],"image":"","project_id":"prj_1","team_id":"","timeout_minutes":90,"vcpus":null}}`
	if string(got) != want {
		t.Fatalf("got %s\nwant %s", got, want)
	}
	got, _ = json.Marshal(CloudSandboxChange{Kind: "cloudflare", Token: "t0ken", Settings: CloudSandboxSettings{URL: "https://lorca-sandbox.example.workers.dev"}}.body())
	if want := `{"cloudflare":{"env":[],"token":"t0ken","url":"https://lorca-sandbox.example.workers.dev"}}`; string(got) != want {
		t.Fatalf("got %s\nwant %s", got, want)
	}
	got, _ = json.Marshal(CloudSandboxChange{Kind: "cloudflare", Remove: true}.body())
	if want := `{"cloudflare":null}`; string(got) != want {
		t.Fatalf("got %s\nwant %s", got, want)
	}
}

// The demo sets a sandbox up, makes it the default, and removing it moves agents back to the
// Runner.
func TestTheDemoSetsUpAndRemovesASandbox(t *testing.T) {
	s := NewStore(nil, func(fn func()) { fn() }, true)
	var answered CloudSandboxes
	s.SetCloudSandbox("r1", CloudSandboxChange{Kind: "vercel", Token: "t", Settings: CloudSandboxSettings{ProjectID: "prj_1"}}, func(sandboxes CloudSandboxes, _ error) { answered = sandboxes })
	s.SetDefaultCloudSandbox("r1", "vercel", func(sandboxes CloudSandboxes, _ error) { answered = sandboxes })
	if !answered.Sandbox("vercel").SetUp || answered.Default != "vercel" || answered.Sandbox("vercel").Detail() != "Set up · prj_1" {
		t.Fatalf("not set up: %+v", answered)
	}
	s.SetCloudSandbox("r1", CloudSandboxChange{Kind: "vercel", Remove: true}, func(sandboxes CloudSandboxes, _ error) { answered = sandboxes })
	if answered.Sandbox("vercel").SetUp || answered.Default != "" {
		t.Fatalf("still set up: %+v", answered)
	}
}
