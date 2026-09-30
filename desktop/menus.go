package main

import (
	"context"
	"sync"
	"time"

	"github.com/egoist/mygo"
)

// MenuItemSpec is one item of a menu the page describes: its words come from the page, which
// holds the translations, and a click comes back to the page as its id.
type MenuItemSpec struct {
	ID          string `json:"id,omitempty"`
	Label       string `json:"label,omitempty"`
	Role        string `json:"role,omitempty"`
	Accelerator string `json:"accelerator,omitempty"`
	// Type is "separator", "checkbox", or "radio"; empty is a plain item.
	Type     string `json:"type,omitempty"`
	Disabled bool   `json:"disabled,omitempty"`
	Hidden   bool   `json:"hidden,omitempty"`
	Checked  bool   `json:"checked,omitempty"`
	// OpensMain brings the main window up before the command reaches it, as File › New Bot does
	// from anywhere.
	OpensMain bool           `json:"opensMain,omitempty"`
	Submenu   []MenuItemSpec `json:"submenu,omitempty"`
}

// MenuItemState changes an item of the menu bar in place: its enabled state, title, check.
type MenuItemState struct {
	ID       string  `json:"id"`
	Label    *string `json:"label,omitempty"`
	Disabled *bool   `json:"disabled,omitempty"`
	Hidden   *bool   `json:"hidden,omitempty"`
	Checked  *bool   `json:"checked,omitempty"`
}

// MenuCommand is a menu bar item the user chose, sent to the page that owns the command.
var MenuCommand = mygo.NewEvent[string]("menu:command")

var roles = map[string]mygo.MenuRole{
	"undo": mygo.RoleUndo, "redo": mygo.RoleRedo, "cut": mygo.RoleCut, "copy": mygo.RoleCopy,
	"paste": mygo.RolePaste, "delete": mygo.RoleDelete, "selectAll": mygo.RoleSelectAll,
	"quit": mygo.RoleQuit, "close": mygo.RoleClose, "minimize": mygo.RoleMinimize, "zoom": mygo.RoleZoom,
	"toggleFullScreen": mygo.RoleToggleFullScreen, "toggleDevTools": mygo.RoleToggleDevTools,
	"reload": mygo.RoleReload, "resetZoom": mygo.RoleResetZoom, "zoomIn": mygo.RoleZoomIn, "zoomOut": mygo.RoleZoomOut,
	"pasteAndMatchStyle": mygo.RolePasteAndMatchStyle, "about": mygo.RoleAbout, "hide": mygo.RoleHide,
	"hideOthers": mygo.RoleHideOthers, "unhide": mygo.RoleUnhide, "front": mygo.RoleFront,
	"services": mygo.RoleServices, "appMenu": mygo.RoleAppMenu, "windowMenu": mygo.RoleWindowMenu,
}

func buildItems(specs []MenuItemSpec, click func(spec MenuItemSpec, win *mygo.Window)) []*mygo.MenuItem {
	items := make([]*mygo.MenuItem, 0, len(specs))
	for _, spec := range specs {
		if spec.Type == "separator" {
			items = append(items, mygo.Separator())
			continue
		}
		item := &mygo.MenuItem{
			ID:          spec.ID,
			Label:       spec.Label,
			Accelerator: spec.Accelerator,
			Disabled:    spec.Disabled,
			Hidden:      spec.Hidden,
			Checked:     spec.Checked,
		}
		switch spec.Type {
		case "checkbox":
			item.Type = mygo.MenuItemCheckbox
		case "radio":
			item.Type = mygo.MenuItemRadio
		}
		if role, ok := roles[spec.Role]; ok {
			item.Role = role
		}
		if len(spec.Submenu) > 0 {
			item.Submenu = buildItems(spec.Submenu, click)
		} else if item.Role == "" && spec.ID != "" {
			spec := spec
			item.Click = func(_ *mygo.MenuItem, win *mygo.Window) { click(spec, win) }
		}
		items = append(items, item)
	}
	return items
}

// Menus puts the pages' menus on screen: the menu bar, and menus that pop up.
type Menus struct{}

type menuBars struct {
	mu   sync.Mutex
	main *mygo.Menu
	// Other windows' own menus, by window id.
	own map[int]*mygo.Menu
}

var bars = &menuBars{own: map[int]*mygo.Menu{}}

// SetBar makes items the calling window's menu bar. The main window's is the app's, which every
// window without one of its own shares.
func (Menus) SetBar(ctx context.Context, items []MenuItemSpec) {
	win := mygo.CallerWindow(ctx)
	menu := mygo.NewMenu(buildItems(items, func(spec MenuItemSpec, clicked *mygo.Window) {
		app.menuCommand(spec, win)
	}))
	bars.mu.Lock()
	defer bars.mu.Unlock()
	if win != nil && app.isMainWindow(win) {
		bars.main = menu
		mygo.App.SetMenu(menu)
		return
	}
	if win != nil {
		bars.own[win.ID()] = menu
		win.SetMenu(menu)
	}
}

// Update changes items of the calling window's menu bar in place.
func (Menus) Update(ctx context.Context, states []MenuItemState) {
	win := mygo.CallerWindow(ctx)
	bars.mu.Lock()
	menu := bars.main
	if win != nil && !app.isMainWindow(win) {
		menu = bars.own[win.ID()]
	}
	bars.mu.Unlock()
	if menu == nil {
		return
	}
	for _, state := range states {
		item := menu.ItemByID(state.ID)
		if item == nil {
			continue
		}
		if state.Label != nil {
			item.SetLabel(*state.Label)
		}
		if state.Disabled != nil {
			item.SetEnabled(!*state.Disabled)
		}
		if state.Hidden != nil {
			item.SetVisible(!*state.Hidden)
		}
		if state.Checked != nil {
			item.SetChecked(*state.Checked)
		}
	}
}

// Popup shows a menu at x, y in the calling page (CSS pixels) and returns the id of the item
// chosen, or "" when the menu closed without a choice.
func (Menus) Popup(ctx context.Context, items []MenuItemSpec, x, y int) string {
	chosen := make(chan string, 1)
	menu := mygo.NewMenu(buildItems(items, func(spec MenuItemSpec, _ *mygo.Window) {
		select {
		case chosen <- spec.ID:
		default:
		}
	}))
	menu.PopupAt(mygo.CallerWindow(ctx), x, y)
	// GTK ends the menu before it activates the item, so the click can land just after.
	select {
	case id := <-chosen:
		return id
	case <-time.After(150 * time.Millisecond):
		return ""
	}
}
