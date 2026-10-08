package main

import (
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Edits a bot's MEMORY.md, the notes that open every turn, after the macOS app's
// MemoryViewController. A save carries the hash of the text that was opened, so it is refused when
// the bot wrote in between; the user then reloads the bot's version or overwrites it knowingly.

type memoryState struct {
	bot *model.Bot
	// memory is the file as last read: the hash a save expects, and the budget.
	memory model.BotMemory
	text   string
	saving bool
}

// presentMemory edits a bot's MEMORY.md; onSaved runs once it is saved.
func (w *appWindow) presentMemory(bot *model.Bot, memory model.BotMemory, onSaved func()) {
	st := &memoryState{bot: bot, memory: memory, text: memory.Text}
	// The subtitle states the budget the sheet opened with.
	subtitle := L("MEMORY.md opens at the start of every turn: the first %d lines or %@, whichever cuts first. Longer notes belong in memory/<topic>.md files the bot reads on demand.",
		memory.MaxLines, model.Kilobytes(memory.MaxBytes))
	w.present(func(c *ui.Context, s *sheet) {
		p := colors(c)
		title := L("%@'s memory", bot.Name)
		result := sheetFrame(c, sheetOptions{
			Title:           title,
			Subtitle:        subtitle,
			Width:           560,
			Confirm:         L("Save"),
			ConfirmDisabled: st.saving,
			ReturnInContent: true,
		}, func() {
			textArea(c, &st.text, 0, fieldOptions{Mono: true, AutoFocus: true, Label: title}).Height(340).LineHeight(1.45)
			gauge, tone := memoryGauge(st.text, st.memory)
			ui.Text(c, gauge).FontSize(textCaption).LineHeight(1.4).TextColor(p.tone(tone))
		})
		switch {
		case result.Confirmed:
			st.save(w, s, st.memory.Hash, onSaved)
		case result.Cancelled:
			s.dismiss()
		}
	}, nil)
}

// save writes the draft; with an expected hash it is refused when the bot wrote in between.
func (st *memoryState) save(w *appWindow, s *sheet, expectedHash string, onSaved func()) {
	st.saving = true
	store.WriteBotMemory(st.bot.ID, st.text, expectedHash, func(_ string, err error) {
		if err == nil {
			if onSaved != nil {
				onSaved()
			}
			s.dismiss()
			return
		}
		st.saving = false
		if strings.Contains(model.ErrorText(err), "changed since") {
			st.resolveConflict(w, s, onSaved)
			return
		}
		w.showAlert(alertOptions{Message: L("Couldn't save the memory"), Informative: model.ErrorText(err)}, nil)
	})
}

// resolveConflict is the bot having written while the user was editing: show the bot's version,
// or write over it.
func (st *memoryState) resolveConflict(w *appWindow, s *sheet, onSaved func()) {
	name := st.bot.Name
	w.showAlert(alertOptions{
		Message:     L("%@ changed this file while you were editing", name),
		Informative: L("Reload shows %@'s version and discards your draft. Overwrite saves yours over it.", name),
		Buttons:     []alertButton{{Title: L("Reload")}, {Title: L("Overwrite with mine")}, {Title: L("Cancel")}},
	}, func(index int) {
		switch index {
		case 0:
			store.BotMemory(st.bot.ID, func(fresh model.BotMemory, err error) {
				// When it can't be read, the draft stays and the next save tries again.
				if err == nil {
					st.memory, st.text = fresh, fresh.Text
				}
			})
		case 1:
			st.save(w, s, "", onSaved)
		}
	})
}

// memoryGauge is lines and bytes against the load budget: orange from 80%, red once anything
// stops loading.
func memoryGauge(text string, memory model.BotMemory) (string, model.Tone) {
	lines := 0
	if text != "" {
		lines = strings.Count(text, "\n") + 1
	}
	bytes := len(text)
	var status string
	if lines == 1 {
		status = L("%d line of %d · %@ of %@", lines, memory.MaxLines, model.Kilobytes(bytes), model.Kilobytes(memory.MaxBytes))
	} else {
		status = L("%d lines of %d · %@ of %@", lines, memory.MaxLines, model.Kilobytes(bytes), model.Kilobytes(memory.MaxBytes))
	}
	overLines, overBytes := lines > memory.MaxLines, bytes > memory.MaxBytes
	if overLines || overBytes {
		hidden := 0
		if overLines {
			hidden = lines - memory.MaxLines
		}
		switch {
		case hidden == 1:
			status += " · " + L("%d line past the budget will not load", hidden)
		case hidden > 1:
			status += " · " + L("%d lines past the budget will not load", hidden)
		default:
			status += " · " + L("the end will not load")
		}
		return status, model.ToneRed
	}
	if float64(lines) >= float64(memory.MaxLines)*0.8 || float64(bytes) >= float64(memory.MaxBytes)*0.8 {
		return status, model.ToneOrange
	}
	return status, model.ToneSecondary
}
