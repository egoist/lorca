package main

import (
	"fmt"
	"slices"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Outputs are CLI-owned immutable chat records. The sheet's state belongs to its lifetime;
// request generations reject older replies, and all callbacks run on the main-thread queue.
type outputsState struct {
	chatID    string
	sheet     *sheet
	messages  []*model.Message
	loading   bool
	loaded    bool
	errorText string
	hasMore   bool
	request   uint64
	count     int
	activity  time.Time
	actions   map[string]*outputActionState
}

type outputActionState struct {
	busy      bool
	errorText string
}

func (s *outputsState) active() bool {
	return s.sheet != nil && s.sheet.window != nil && (s.sheet.window.win == nil || !s.sheet.window.win.IsDestroyed())
}

func (s *outputsState) reload() {
	if !s.active() {
		return
	}
	s.request++
	request := s.request
	s.loading = true
	if chat := store.Chat(s.chatID); chat != nil {
		s.count, s.activity = len(chat.Messages), chat.LastActivity()
	}
	store.LoadOutputs(s.chatID, "", func(page model.OutputPage, err error) {
		if !s.active() || request != s.request || store.Chat(s.chatID) == nil {
			return
		}
		s.loading, s.loaded = false, true
		s.errorText = model.ErrorText(err)
		if err == nil {
			s.messages = slices.Clone(page.Messages)
			slices.Reverse(s.messages)
			s.hasMore = page.HasMore
		}
		s.sheet.window.invalidate()
	})
}

func (m *mainWindow) presentOutputs(chatID string) {
	if chatID == "" || store.Chat(chatID) == nil {
		return
	}
	state := &outputsState{chatID: chatID, actions: map[string]*outputActionState{}}
	state.sheet = m.present(func(c *ui.Context, sheet *sheet) {
		state.view(c)
	}, func() { state.request++ })
	state.reload()
}

func (m *mainWindow) outputsButton(c *ui.Context, chatID string) {
	if chatID == "" {
		return
	}
	if hoverButton(c.Key("outputs"), hoverButtonOptions{Symbol: "tray.full.fill", Tooltip: L("Outputs"), Label: L("Outputs")}).Clicked() {
		m.presentOutputs(chatID)
	}
}

func (s *outputsState) view(c *ui.Context) {
	p := colors(c)
	if chat := store.Chat(s.chatID); chat == nil {
		if s.loading {
			s.request++
		}
		s.loading = false
		s.messages = nil
		s.errorText = L("Chat unavailable")
	} else if !s.loading && (len(chat.Messages) != s.count || !chat.LastActivity().Equal(s.activity)) {
		s.reload()
	}
	result := sheetFrame(c, sheetOptions{Title: L("Outputs"), Width: 620, Confirm: L("Done"), NoCancel: true, Leading: func() {
		if pushButton(c.Key("outputs-refresh"), L("Refresh"), pushOptions{Disabled: s.loading}).Clicked() {
			s.reload()
		}
	}}, func() {
		switch {
		case s.loading:
			ui.Text(c, L("Loading outputs…")).FontSize(12).TextColor(p.Label2)
		case s.errorText != "":
			ui.Text(c, L("Outputs unavailable: %@", s.errorText)).FontSize(12).TextColor(p.Red)
		case len(s.messages) == 0:
			ui.Text(c, L("No outputs yet.")).FontSize(12).TextColor(p.Label2)
		case len(s.messages) == 1:
			ui.Text(c, L("%d output version", 1)).FontSize(12).TextColor(p.Label2)
		default:
			ui.Text(c, L("%d output versions", len(s.messages))).FontSize(12).TextColor(p.Label2)
		}
		for _, message := range s.messages {
			if message.Output == nil {
				continue
			}
			ui.Column(c.Key(outputKey(message))).Label("output:" + message.ID).Gap(6).Padding(12).Radius(10).Background(p.Card).Children(func() { s.outputRow(c, message) })
		}
		if s.hasMore {
			if pushButton(c.Key("outputs-earlier"), L("Load earlier outputs"), pushOptions{Disabled: s.loading}).Clicked() {
				store.LoadOlderMessages(s.chatID)
			}
		}
	})
	if result.Confirmed || result.Cancelled {
		s.sheet.dismiss()
	}
}

func (s *outputsState) outputRow(c *ui.Context, message *model.Message) {
	p := colors(c)
	output := message.Output
	ui.Text(c, L("%@ · v%d", output.Name, uint64(output.Version))).FontSize(14).FontWeight(600)
	name := output.BotID
	if bot := store.Bot(output.BotID); bot != nil {
		name = bot.Name
	}
	ui.Text(c, L("Produced by %@ · %@", name, model.Clock(message.CreatedAt))).FontSize(11).TextColor(p.Label2)
	if output.TaskID != "" {
		ui.Text(c, L("Task: %@", output.TaskID)).FontSize(11).TextColor(p.Label2)
	}
	ui.Text(c, output.Mime).FontSize(11).TextColor(p.Label2)
	if evidence := output.Evidence; evidence != nil {
		color := p.Label
		if evidence.Status == "failed" {
			color = p.Red
		}
		ui.Text(c, evidence.Title()+" · "+evidence.StatusText()+": "+evidence.Summary).FontSize(12).TextColor(color).LineHeight(1.4)
		if evidence.Command != "" {
			ui.Text(c, L("Command: %@", evidence.Command)).FontSize(11).TextColor(p.Label2)
		}
		if evidence.ExitCode != nil {
			ui.Text(c, L("Exit code: %d", *evidence.ExitCode)).FontSize(11).TextColor(p.Label2)
		}
	}
	if output.URL != "" {
		if link := output.DocumentURL(); link != "" {
			if pushButton(c.Key("document"), L("Open document"), pushOptions{Small: true}).Label(L("Open document %@", output.Name)).Clicked() {
				_ = openExternal(link)
			}
		} else {
			ui.Text(c, L("Document link unavailable")).FontSize(12).TextColor(p.Red)
		}
		return
	}
	if len(message.Attachments) == 0 {
		ui.Text(c, L("File unavailable")).FontSize(12).TextColor(p.Red)
		return
	}
	attachment := message.Attachments[0]
	path := store.LocalFile(attachment, s.chatID, message.ID)
	fileError := store.AttachmentError(attachment.ID)
	if attachment.IsImage() && path != "" {
		w, h := imageSize(attachment)
		ui.Box(c.Key("thumbnail")).Size(w, h).Radius(8).Clip().Background(p.Code).Children(func() {
			if bitmap := loadBitmap(path); bitmap != nil {
				ui.Image(c, bitmap).Size(w, h).Fit(ui.Contain)
			}
		})
	}
	switch {
	case fileError != "":
		ui.Text(c, L("File unavailable: %@", fileError)).FontSize(12).TextColor(p.Red)
		if pushButton(c.Key("retry"), L("Retry"), pushOptions{Small: true}).Label(L("Retry %@", output.Name)).Clicked() {
			store.RetryAttachment(attachment, s.chatID, message.ID)
		}
	case path == "":
		ui.Text(c, L("Fetching…")).FontSize(12).TextColor(p.Label2)
	default:
		ui.Text(c, attachment.Name+" · "+model.SizeText(attachment.Size)).FontSize(11).TextColor(p.Label2)
		action := s.actions[message.ID]
		if action == nil {
			action = &outputActionState{}
			s.actions[message.ID] = action
		}
		ui.Row(c).Gap(8).Children(func() {
			if pushButton(c.Key("preview"), L("Preview"), pushOptions{Small: true, Disabled: action.busy}).Label(L("Preview %@", output.Name)).Clicked() {
				s.sheet.window.presentOutputPreview(attachment, path)
			}
			if pushButton(c.Key("open"), L("Open"), pushOptions{Small: true, Disabled: action.busy}).Label(L("Open %@", output.Name)).Clicked() {
				s.openFile(message.ID, path)
			}
			if pushButton(c.Key("save"), L("Save As…"), pushOptions{Small: true, Disabled: action.busy}).Label(L("Save %@", output.Name)).Clicked() {
				s.saveFile(message.ID, attachment, path)
			}
		})
		if action.busy {
			ui.Text(c, L("Working…")).FontSize(11).TextColor(p.Label2)
		}
		if action.errorText != "" {
			ui.Text(c, action.errorText).FontSize(12).TextColor(p.Red)
			if pushButton(c.Key("retry-action"), L("Retry"), pushOptions{Small: true}).Label(L("Retry %@", output.Name)).Clicked() {
				action.errorText = ""
				store.RetryAttachment(attachment, s.chatID, message.ID)
			}
		}
	}
}

func outputTextPreview(mime string) bool {
	return strings.HasPrefix(mime, "text/") || mime == "application/json" || mime == "application/xml"
}

func outputKey(message *model.Message) string {
	return fmt.Sprintf("%s:v%d", message.ID, message.Output.Version)
}
