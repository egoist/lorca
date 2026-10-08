package main

import (
	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Optional cross-branch adapter: #79 registers its existing task budget sheet after
// consolidation. A nil opener keeps unavailable accounting UI out of the task editor.
var taskBudgetOpener func(*appWindow, *model.Bot, string, string)

func (m *mainWindow) inspectorDurableTasks(c *ui.Context, chat *model.Chat) {
	section(c.Key("durable-tasks"), L("Tasks"), sectionCaption, nil, func(k *card) {
		for _, task := range store.TasksIn(chat.ID) {
			owner := task.OwnerBotID
			if bot := store.Bot(owner); bot != nil {
				owner = bot.Name
			}
			row := k.row(rowBox(c.Key(task.ID)).Label(task.Goal).AlignItems(ui.Start))
			row.Children(func() {
				ui.Column(c).Grow(1).Shrink(1).MinWidth(0).Gap(3).Children(func() {
					ui.Text(c, task.Goal).FontSize(12).FontWeight(500).SingleLine()
					color := colors(c).Label2
					if task.State == model.TaskBlocked {
						color = colors(c).Orange
					}
					ui.Text(c, task.State.Title()+" · "+owner).FontSize(11).TextColor(color).SingleLine()
				})
				if linkButton(c, L("Open…"), false).Label(L("Open task %@", task.Goal)).Clicked() {
					m.presentDurableTask(chat.ID, task)
				}
			})
		}
		k.row(rowBox(c.Key("create"))).Children(func() {
			ui.Text(c, L("New task")).Grow(1).FontSize(12).TextColor(colors(c).Label2)
			if linkButton(c, L("Create…"), !store.IsConnected).Label(L("Create task")).Clicked() {
				m.presentDurableTask(chat.ID, nil)
			}
		})
	})
}
