package model

import (
	"encoding/json"
	"os"
	"testing"
)

// The new-bot sheet's Runner menu offers store.Runners(); the CLI's create_bot and edit_bot
// take the same Devices, tested against the same cases (`crates/cli/src/model.rs`).
func TestRunnersAreTheDevicesTheBotToolsTake(t *testing.T) {
	data, err := os.ReadFile("../../crates/cli/testdata/runners.json")
	if err != nil {
		t.Fatal(err)
	}
	var cases []struct {
		OS      string `json:"os"`
		Unknown bool   `json:"unknown"`
		Runner  bool   `json:"runner"`
	}
	if err := json.Unmarshal(data, &cases); err != nil || len(cases) < 9 {
		t.Fatalf("cases: %v %d", err, len(cases))
	}
	s := NewStore(nil, func(fn func()) { fn() }, false)
	for i, c := range cases {
		device := ToDevice(WireDevice{ID: string(rune('a' + i)), Name: "D", OS: c.OS, Unknown: c.Unknown})
		if device.IsRunner() != c.Runner {
			t.Errorf("os %q unknown %v: IsRunner %v, want %v", c.OS, c.Unknown, device.IsRunner(), c.Runner)
		}
		s.Devices = append(s.Devices, device)
	}
	offered := map[string]bool{}
	for _, runner := range s.Runners() {
		offered[runner.ID] = true
	}
	for i, c := range cases {
		if offered[string(rune('a'+i))] != c.Runner {
			t.Errorf("os %q unknown %v: offered %v, want %v", c.OS, c.Unknown, !c.Runner, c.Runner)
		}
	}
}
