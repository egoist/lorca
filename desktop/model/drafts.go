package model

import "errors"

// SendDraft sends a draft card's message as `fields` shows it: an edit first, as the review
// item's next version, then that version approved, which the bot's Runner sends. `always` then
// turns drafts off for the bot, so its next messages go out directly. On an error the card is put
// back as it was.
func (s *Store) SendDraft(chatID, messageID, botID string, card DraftCard, fields DraftFields, always bool, done func(error)) {
	s.setDraft(chatID, messageID, func(d *DraftCard) { d.State, d.Fields = "approved", fields.Clone() })
	finish := func(err error) {
		if err != nil {
			s.setDraft(chatID, messageID, func(d *DraftCard) { *d = card })
		} else if always && botID != "" {
			s.sendDirectly(botID)
		}
		done(err)
	}
	if s.IsMock {
		s.post(func() {
			s.setDraft(chatID, messageID, func(d *DraftCard) { d.State = "succeeded" })
			finish(nil)
		})
		return
	}
	approve := func(version uint64) {
		s.decideDraft("approve", map[string]any{"id": card.ReviewID, "expected_version": version}, func(_ ReviewItem, err error) { finish(err) })
	}
	if fields.Equal(card.Fields) {
		approve(card.Version)
		return
	}
	s.decideDraft("edit", map[string]any{"id": card.ReviewID, "expected_version": card.Version, "message": fields}, func(edited ReviewItem, err error) {
		if err != nil {
			finish(err)
			return
		}
		approve(edited.Version)
	})
}

// DiscardDraft discards a draft card's message: nothing is sent.
func (s *Store) DiscardDraft(chatID, messageID string, card DraftCard, done func(error)) {
	s.setDraft(chatID, messageID, func(d *DraftCard) { d.State = "rejected" })
	if s.IsMock {
		s.post(func() { done(nil) })
		return
	}
	s.decideDraft("reject", map[string]any{"id": card.ReviewID, "expected_version": card.Version}, func(_ ReviewItem, err error) {
		if err != nil {
			s.setDraft(chatID, messageID, func(d *DraftCard) { *d = card })
		}
		done(err)
	})
}

// decideDraft asks the card's Runner to change its review item, by the version the card showed.
func (s *Store) decideDraft(action string, params map[string]any, done func(ReviewItem, error)) {
	identity := s.IdentityID
	Async(s, func() (ReviewItem, error) { return call[ReviewItem](s, "reviews."+action, params) }, func(next ReviewItem, err error) {
		if s.IdentityID != identity {
			err = errors.New(L("The account changed while this review was loading."))
		}
		if err == nil && next.ID != "" {
			next = s.upsertReview(next)
		}
		done(next, err)
	})
}

// sendDirectly has the bot send its messages without a draft from now on, as its Access says once
// drafts are off.
func (s *Store) sendDirectly(botID string) {
	bot := s.Bot(botID)
	if bot == nil {
		return
	}
	policy := bot.Permissions.Clone()
	if policy == nil {
		policy = FullAccess()
	}
	policy.Drafts = false
	bot.Permissions = policy
	s.rosterTouched(botID)
	s.perform("bots.update", map[string]any{"id": botID, "drafts": false})
}

func (s *Store) setDraft(chatID, messageID string, change func(*DraftCard)) {
	s.Update(messageID, chatID, func(message *Message) {
		if message.Body.Kind != BodyDraft || message.Body.Draft == nil {
			return
		}
		card := *message.Body.Draft
		card.Fields = card.Fields.Clone()
		change(&card)
		message.Body.Draft = &card
	})
}
