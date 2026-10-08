package model

import "errors"

// LoadOlderMessages asks for the page before this chat's first row. Automatic viewport
// paging pauses after a failed boundary; explicit link/retry requests can try that boundary again.
func (s *Store) LoadOlderMessages(id string) { s.loadOlderMessages(id, nil, false) }

// LoadOlderMessagesThen shares the existing per-chat request and answers on the store's
// ordered main-thread post queue, after rows and paging state have been applied. Callbacks
// retain only persistent state/ids; no UI context or build-pass element crosses the request.
func (s *Store) LoadOlderMessagesThen(id string, done func(error)) {
	s.loadOlderMessages(id, done, true)
}

func (s *Store) loadOlderMessages(id string, done func(error), retry bool) {
	chat := s.Chat(id)
	if s.IsMock || chat == nil || !chat.HasMore || len(chat.Messages) == 0 {
		if done != nil {
			done(errors.New(L("No older messages are available.")))
		}
		return
	}
	if s.olderDone == nil {
		s.olderDone = map[string][]func(error){}
	}
	if s.olderFailed == nil {
		s.olderFailed = map[string]string{}
	}
	if s.loadingOlder[id] {
		if done != nil {
			s.olderDone[id] = append(s.olderDone[id], done)
		}
		return
	}
	first, generation, identity := chat.Messages[0].ID, s.bootstrapGeneration, s.IdentityID
	hadIdentity := s.HasIdentity != nil && *s.HasIdentity
	if !retry && s.olderFailed[id] == first {
		return
	}
	delete(s.olderFailed, id)
	if done != nil {
		s.olderDone[id] = append(s.olderDone[id], done)
	}
	s.loadingOlder[id] = true
	Async(s, func() (WireMessagePage, error) {
		return call[WireMessagePage](s, "chats.messages", map[string]any{"chat_id": id, "before": first})
	}, func(page WireMessagePage, err error) {
		delete(s.loadingOlder, id)
		callbacks := s.olderDone[id]
		delete(s.olderDone, id)
		current := s.Chat(id)
		currentScope := generation == s.bootstrapGeneration && identity == s.IdentityID && (!hadIdentity || s.HasIdentity != nil && *s.HasIdentity) && current != nil && len(current.Messages) > 0 && current.Messages[0].ID == first
		if err == nil && !currentScope {
			err = errors.New(L("The transcript changed while loading older messages."))
		}
		if err == nil {
			known := map[string]bool{}
			for _, message := range current.Messages {
				known[message.ID] = true
			}
			var older []*Message
			for _, wire := range page.Messages {
				message := ToMessage(wire)
				if !known[message.ID] {
					older = append(older, message)
					known[message.ID] = true
				}
			}
			if len(older) == 0 && page.HasMore {
				err = errors.New(L("The transcript did not provide an older page."))
			} else {
				current.Messages = append(older, current.Messages...)
				current.HasMore = page.HasMore
				for _, message := range older {
					s.noteCommand(message, id)
				}
				s.emit(Event{Kind: EventOlderMessagesLoaded, ChatID: id})
			}
		}
		if err != nil && currentScope {
			s.olderFailed[id] = first
		}
		for _, callback := range callbacks {
			callback(err)
		}
	})
}

// IsLoadingOlder is a page of older messages on its way for the chat.
func (s *Store) IsLoadingOlder(id string) bool { return s.loadingOlder[id] }

// OlderMessagesFailed is a failed paging boundary the transcript can explicitly retry.
func (s *Store) OlderMessagesFailed(id string) bool {
	chat := s.Chat(id)
	boundary, failed := s.olderFailed[id]
	return failed && chat != nil && chat.HasMore && len(chat.Messages) > 0 && boundary == chat.Messages[0].ID
}
