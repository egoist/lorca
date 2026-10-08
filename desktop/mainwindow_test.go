package main

import (
	"math"
	"testing"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

func TestWidened(t *testing.T) {
	area := mygo.Rectangle{X: 0, Y: 40, Width: 1920, Height: 1000}
	rect := func(x, width int) mygo.Rectangle { return mygo.Rectangle{X: x, Y: 100, Width: width, Height: 760} }
	tests := []struct {
		name string
		r    mygo.Rectangle
		by   int
		want mygo.Rectangle
	}{
		{"with room", rect(200, 900), 281, rect(200, 1181)},
		{"at the right edge", rect(1020, 900), 281, rect(739, 1181)},
		{"near the right edge", rect(920, 900), 281, rect(739, 1181)},
		{"at the left edge", rect(0, 900), 261, rect(0, 1161)},
		{"wider than the room", rect(100, 1700), 281, rect(0, 1920)},
		{"filling the area", rect(0, 1920), 281, rect(0, 1920)},
		{"partly off the display", rect(1500, 900), 281, rect(1219, 1181)},
		{"off both sides", rect(-100, 2200), 261, rect(-100, 2200)},
		{"all the room", rect(300, 900), math.MaxInt32, rect(0, 1920)},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if got := widened(test.r, area, test.by); got != test.want {
				t.Errorf("widened(%v, %d) = %v, want %v", test.r, test.by, got, test.want)
			}
		})
	}
}

func TestPaneDividersKeepDragCapture(t *testing.T) {
	for _, sidebar := range []bool{true, false} {
		name := "sidebar"
		if !sidebar {
			name = "inspector"
		}
		t.Run(name, func(t *testing.T) {
			m := demoWindow(t)
			tt := ui.NewTester(m.frame(m.view), 1180, 760)
			runPosts()
			tt.Frame()
			s, i := m.panes(1180)
			x, step, from, low, high := s, float32(24), m.sidebarWidth, float32(sidebarMin), float32(sidebarMax)
			width := func() float32 { return m.sidebarWidth }
			saved := func() int { return prefs.get().SidebarWidth }
			if !sidebar {
				x, step, from, low, high = 1180-i-1, -24, m.inspectorWidth, inspectorMin, inspectorMax
				width = func() float32 { return m.inspectorWidth }
				saved = func() int { return prefs.get().InspectorWidth }
			}
			y := float32(headerHeight + 80)
			tt.Move(x, y)
			if tt.Cursor() != ui.CursorResizeEW {
				t.Fatal("the divider does not offer a resize cursor")
			}
			tt.Press(x, y)
			// Move farther than the seven-DIP handle, keeping the pointer held throughout.
			for moves := 1; moves <= 2; moves++ {
				tt.Move(x+step*float32(moves), y)
				want := clamp(from+24*float32(moves), low, high)
				if got := width(); got != want {
					t.Fatalf("move %d: pane width %.1f, want %.1f", moves, got, want)
				}
			}
			tt.Release(x+2*step, y)
			if got, want := saved(), int(width()); got != want {
				t.Fatalf("saved width %d, want %d", got, want)
			}
			released := width()
			tt.Move(x+3*step, y)
			if width() != released {
				t.Fatal("the pane kept resizing after the pointer was released")
			}
		})
	}
}

func TestSidebarDividerCollapseKeepsInspectorWidth(t *testing.T) {
	m := demoWindow(t)
	tt := ui.NewTester(m.frame(m.view), 1180, 760)
	runPosts()
	tt.Frame()
	sidebar, _ := m.panes(1180)
	inspector := m.inspectorWidth
	y := float32(headerHeight + 80)
	tt.Press(sidebar, y)
	tt.Move(float32(sidebarMin/2-10), y)
	if !m.sidebarCollapsed || !prefs.get().SidebarCollapsed {
		t.Fatal("dragging past the minimum did not collapse the sidebar")
	}
	// The disappearing sidebar handle must not give its captured drag to the inspector handle.
	tt.Move(float32(sidebarMin/2+80), y)
	tt.Release(float32(sidebarMin/2+80), y)
	if m.inspectorWidth != inspector {
		t.Fatal("the sidebar drag changed the inspector width after collapsing")
	}
}
