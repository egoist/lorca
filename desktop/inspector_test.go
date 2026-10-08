package main

import (
	"testing"

	"github.com/egoist/mygo/ui"
)

func TestInspectorRoutineSwitch(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(func(c *ui.Context) {
		applyTheme(c)
		m.inspectorRoutines(c, store.Bot("bot-nova"))
	}, 360, 400)
	settle(tt)
	routine := store.Routine("rt-brief")
	if routine == nil || !routine.IsEnabled {
		t.Fatal("the demo routine is missing or paused")
	}
	if err := tt.Click(L("Pause %@", routine.Name)); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if store.Routine("rt-brief").IsEnabled || m.hasSheet() {
		t.Fatal("the switch should pause the routine without opening its sheet")
	}
	if err := tt.Click(L("Resume %@", routine.Name)); err != nil {
		t.Fatal(err)
	}
	settle(tt)
	if !store.Routine("rt-brief").IsEnabled || m.hasSheet() {
		t.Fatal("the switch should resume the routine without opening its sheet")
	}
}
