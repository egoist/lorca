package main

import (
	"slices"

	"github.com/egoist/lorca/desktop/model"
)

// openTranscriptLink routes an internal result reference in this window. Everything sent to
// the operating system still goes through the web/mail allowlist.
func (m *mainWindow) openTranscriptLink(href string) {
	if link, ok := model.ParseMessageLink(href); ok {
		m.openMessage(link)
		return
	}
	openLink(href)
}

func (m *mainWindow) openMessage(link model.MessageLink) {
	if store.Chat(link.ChatID) == nil {
		m.showAlert(alertOptions{Message: L("The linked chat is unavailable.")}, nil)
		return
	}
	m.selectChat(link.ChatID)
	s := m.chatStateFor(link.ChatID)
	s.linkGeneration++
	s.linkedMessageID, s.linkedMessageLoading = link.MessageID, false
	s.scrollToLatest = false
	m.invalidate()
}

// resolveLinkedMessage runs after the current rows are rebuilt, before the list is built.
// ScrollTo then uses the measured target row in this build pass, not an old row index.
func (m *mainWindow) resolveLinkedMessage(s *chatState, chat *model.Chat) {
	id := s.linkedMessageID
	if id == "" {
		return
	}
	for _, row := range s.rows {
		if row.kind == rowMessage && row.message.ID == id {
			s.linkedMessageID, s.linkedMessageLoading = "", false
			s.list.FollowEnd = false
			m.revealMessage(s, id)
			return
		}
	}
	if s.linkedMessageLoading {
		return
	}
	if !chat.HasMore || slices.ContainsFunc(chat.Messages, func(message *model.Message) bool { return message.ID == id }) {
		s.linkedMessageID = ""
		m.showAlert(alertOptions{Message: L("The linked message is unavailable.")}, nil)
		return
	}
	s.linkedMessageLoading = true
	generation := s.linkGeneration
	store.LoadOlderMessagesThen(chat.ID, func(err error) {
		// Replies are on the main thread. Navigating away, another link, or a destroyed window
		// revokes only this reveal; the valid history page remains in the shared store.
		if m.chat != s || m.selection.ChatID != s.chatID || s.linkGeneration != generation || s.linkedMessageID != id || m.win != nil && m.win.IsDestroyed() {
			return
		}
		s.linkedMessageLoading = false
		if err != nil {
			s.linkedMessageID = ""
			m.showAlert(alertOptions{Message: L("Could not load the linked message."), Informative: model.ErrorText(err)}, nil)
		}
		m.invalidate()
	})
}
