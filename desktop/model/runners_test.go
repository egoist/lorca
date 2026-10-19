package model

import (
	"encoding/json"
	"os"
	"slices"
	"testing"
)

// The new-bot sheet's Runner pop-up offers store.Runners(). The CLI's create_bot and edit_bot
// take the same Devices: both are held to testdata/runner_devices.json, which the CLI's
// `the_snapshot_marks_the_runners_the_bot_tools_take` reads too.
func TestRunnersAreTheDevicesTheCLIMarks(t *testing.T) {
	raw, err := os.ReadFile("testdata/runner_devices.json")
	if err != nil {
		t.Fatal(err)
	}
	var wires []WireDevice
	if err := json.Unmarshal(raw, &wires); err != nil {
		t.Fatal(err)
	}
	s := NewStore(nil, func(fn func()) { fn() }, false)
	var want []string
	for _, wire := range wires {
		device := ToDevice(wire)
		if device.IsRunner() != wire.Runner {
			t.Errorf("%s (%q): IsRunner %v, the CLI says %v", wire.Name, wire.OS, device.IsRunner(), wire.Runner)
		}
		if wire.Runner {
			want = append(want, wire.ID)
		}
		s.Devices = append(s.Devices, device)
	}
	var got []string
	for _, device := range s.Runners() {
		got = append(got, device.ID)
	}
	if !slices.Equal(got, want) {
		t.Errorf("Runners() = %v, want %v", got, want)
	}
	// A system this app does not know shows as Linux, but it is no Runner.
	haiku := ToDevice(WireDevice{ID: "haiku", Name: "Haiku box", OS: "haiku"})
	if haiku.OS != OSLinux || haiku.IsRunner() {
		t.Errorf("haiku: OS %q, IsRunner %v", haiku.OS, haiku.IsRunner())
	}
}
