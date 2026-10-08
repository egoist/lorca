package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// inspectorDurableTasks lists the chat's tasks, what waits on the user first, as rows that open
// the task; the section hides while the chat has none, and its title's + starts a new one. Past
// five rows the rest wait behind Show More.
func (m *mainWindow) inspectorDurableTasks(c *ui.Context, chat *model.Chat) {
	tasks := store.TasksIn(chat.ID)
	if len(tasks) == 0 {
		return
	}
	p := colors(c)
	shown := tasks
	if m.inspector.tasksShowingAll != chat.ID && len(tasks) > 5 {
		shown = tasks[:4]
	}
	chatID := chat.ID
	section(c.Key("durable-tasks"), L("Tasks"), sectionCaption, func() {
		if hoverButton(c, hoverButtonOptions{Symbol: "plus", Tooltip: L("New Task")}).Clicked() {
			m.presentDurableTask(chatID, nil)
		}
	}, func(k *card) {
		for _, task := range shown {
			detail := task.State.Title()
			if chat.IsGroup() {
				if owner := store.Bot(task.OwnerBotID); owner != nil {
					detail += " · " + owner.Name
				}
			}
			id := task.ID
			ui.Box(c.Key(task.ID)).Children(func() {
				if taskRow(c, k, task.State.Symbol(), taskTint(p, task.State), task.Goal, detail) {
					if current := store.DurableTask(id); current != nil {
						m.presentDurableTask(chatID, current)
					}
				}
			})
		}
		if len(shown) < len(tasks) {
			ui.Box(c.Key("all")).Children(func() {
				if taskRow(c, k, "ellipsis", p.Label3, L("Show %d More", len(tasks)-len(shown)), "") {
					m.inspector.tasksShowingAll = chatID
				}
			})
		}
	})
}

// taskRow is a row that opens what it shows: a symbol, a title of up to two lines, and a detail
// line. It answers a click.
func taskRow(c *ui.Context, k *card, symbolName string, tint ui.Color, title, detail string) bool {
	p := colors(c)
	r := k.row(rowBox(c).MinHeight(44).Label(title).Tooltip(title).Cursor(ui.CursorPointer).Role(ui.RoleButton))
	if r.Hovered() {
		r.Background(p.RowHover)
	}
	clicked := r.Clicked()
	r.Children(func() {
		ui.Row(c).Width(18).Justify(ui.Center).TextColor(tint).Children(func() { symbol(c, symbolName, 15, 1.8) })
		ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(1).Children(func() {
			ui.Text(c, title).FontSize(12.5).FontWeight(500).MaxLines(2)
			if detail != "" {
				ui.Text(c, detail).FontSize(textCaption).TextColor(p.Label2).SingleLine()
			}
		})
	})
	return clicked
}

// taskTint colors the states that wait on the user; the rest stay quiet.
func taskTint(p *palette, state model.TaskState) ui.Color {
	switch state {
	case model.TaskBlocked:
		return p.Orange
	case model.TaskAwaitingReview:
		return p.Accent
	case model.TaskCompleted, model.TaskCancelled:
		return p.Label3
	default:
		return p.Label2
	}
}
