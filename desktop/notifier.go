package main

import (
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
)

// System notifications for replies, failed responses, and questions, after the macOS app's Notifier
// and ChatNotification: posted unless the message was read on a paired Device first, and a click
// brings the app forward on the chat. The same "looking at" fact goes to the CLI (`ui.watching`),
// so a Runner does not push a reply the user is watching arrive to their phone.

type notificationKind int

const (
	notifyReply notificationKind = iota
	notifyPermission
	notifyFailure
	notifySummary
	notifyUrgent
)

// chatNotification is the message behind an alert, kept so a delayed delivery can check its
// current state.
type chatNotification struct {
	kind      notificationKind
	messageID string
	botID     string
	body      string
}

func notificationFor(message *model.Message) *chatNotification {
	if message.Author.Kind != model.AuthorBot || message.Notification == model.NotificationQuiet {
		return nil
	}
	base := chatNotification{messageID: message.ID, botID: message.Author.BotID}
	body := message.Body
	switch {
	// An access request is the bot's to explain in its reply, which notifies on its own.
	case body.Kind == model.BodyPermission && body.Request.IsPending() && body.Request.IsSecret():
		base.kind, base.body = notifyPermission, L("Asks for %@", body.Request.Summary)
		return &base
	case body.Kind == model.BodyPermission && body.Request.IsPending() && !body.Request.IsAccess():
		base.kind, base.body = notifyPermission, L("Confirmation needed: %@", body.Request.Summary)
		return &base
	case body.Kind == model.BodyTool && body.Tool.Run != nil && body.Tool.Run.State == model.CommandAsking:
		base.kind, base.body = notifyPermission, L("Confirmation needed: %@", "$ "+model.FirstLine(body.Tool.Run.Command))
		return &base
	case body.Kind == model.BodyText:
		if message.State.Kind == model.StateFailed {
			base.kind, base.body = notifyFailure, L("Reply failed: %@", message.State.Error)
			return &base
		}
		if message.State.Kind == model.StateComplete && strings.TrimSpace(body.Text) != "" {
			base.kind, base.body = notifyReply, body.Text
			switch message.Notification {
			case model.NotificationSummary:
				base.kind = notifySummary
			case model.NotificationUrgent:
				base.kind = notifyUrgent
			}
			return &base
		}
	}
	return nil
}

// finishedTurn is the last terminal reply of a turn: it wins, so a failure after partial output
// shows the error.
func finishedTurn(chat *model.Chat, botID string, startedAt time.Time) *chatNotification {
	for i := len(chat.Messages) - 1; i >= 0; i-- {
		message := chat.Messages[i]
		if message.Author.Kind != model.AuthorBot || message.Author.BotID != botID || message.CreatedAt.Before(startedAt.Add(-5*time.Second)) || message.Body.Kind != model.BodyText || message.Notification != "" {
			continue
		}
		if notification := notificationFor(message); notification != nil {
			return notification
		}
	}
	return nil
}

// canDeliver is a notification still unread, not on screen, and its message still saying what
// the alert would.
func canDeliver(notification *chatNotification, chat *model.Chat, watched string) bool {
	if watched == chat.ID || chat.UnreadCount <= 0 {
		return false
	}
	message := chat.Message(notification.messageID)
	if message == nil {
		return false
	}
	current := notificationFor(message)
	return current != nil && *current == *notification
}

type notifier struct {
	pendingPermissions map[string]bool
	announcedAttention map[string]bool
}

func newNotifier() *notifier {
	return &notifier{pendingPermissions: map[string]bool{}, announcedAttention: map[string]bool{}}
}

// started is notifications going out: once the first connection settles.
func (n *notifier) started() bool { return store != nil && !store.IsStarting }

// watchedChat is the chat the user is looking at: the main window is in front and shows it.
func (n *notifier) watchedChat() string {
	m := app.main
	if m == nil || !m.focused || !m.selection.IsChat() {
		return ""
	}
	return m.selection.ChatID
}

// watchingChanged follows the selection and the main window's visibility.
func (n *notifier) watchingChanged() {
	if n == nil || !n.started() {
		return
	}
	watched := n.watchedChat()
	store.SetWatchedChat(watched)
	// Whatever was posted for the chat now on screen has been seen.
	if watched != "" {
		clearNotices(watched)
	}
}

func (n *notifier) storeChanged(event model.Event) {
	if n == nil {
		return
	}
	switch event.Kind {
	case model.EventConnectionChanged:
		n.watchingChanged()
	case model.EventSnapshotReplaced, model.EventIdentityChanged:
		n.rememberPermissions()
		n.watchingChanged()
	case model.EventMessageAdded, model.EventMessageChanged:
		n.permissionChanged(event.ChatID, event.MessageID)
		n.attentionChanged(event.ChatID, event.MessageID)
	case model.EventMessageRemoved:
		n.clearPermission(event.MessageID)
	case model.EventTurnFinished:
		n.turnFinished(event.ChatID, event.BotID, event.StartedAt)
	}
}

// Structured summaries/escalations alert on arrival, once per message, separately from
// the generic turn-finished path. Preferences and read state are checked after the grace.
func (n *notifier) attentionChanged(chatID, messageID string) {
	chat := store.Chat(chatID)
	if chat == nil {
		return
	}
	message := chat.Message(messageID)
	if message == nil {
		return
	}
	notification := notificationFor(message)
	if notification == nil || notification.kind != notifySummary && notification.kind != notifyUrgent || n.announcedAttention[messageID] {
		return
	}
	n.announcedAttention[messageID] = true
	identity := store.IdentityID
	time.AfterFunc(3*time.Second, func() {
		post(func() {
			if store.IdentityID == identity && n.started() {
				n.post(notification, chatID)
			}
		})
	})
}

func allowsNotification(notification *chatNotification) bool {
	switch notification.kind {
	case notifySummary:
		return store.Attention.Preferences.Summaries
	case notifyUrgent:
		return store.Attention.Preferences.UrgentDirect
	}
	return true
}

func asks(message *model.Message) bool {
	body := message.Body
	return body.Kind == model.BodyPermission && body.Request.IsPending() ||
		body.Kind == model.BodyTool && body.Tool.Run != nil && body.Tool.Run.State == model.CommandAsking
}

func (n *notifier) rememberPermissions() {
	current := map[string]bool{}
	for _, chat := range store.Chats {
		for _, message := range chat.Messages {
			if asks(message) {
				current[message.ID] = true
			}
		}
	}
	for id := range n.pendingPermissions {
		if !current[id] {
			n.clearPermission(id)
		}
	}
	n.pendingPermissions = current
}

func (n *notifier) clearPermission(messageID string) {
	delete(n.pendingPermissions, messageID)
	removeNotice(messageID)
}

func (n *notifier) permissionChanged(chatID, messageID string) {
	chat := store.Chat(chatID)
	if chat == nil {
		return
	}
	message := chat.Message(messageID)
	if message == nil {
		return
	}
	body := message.Body
	if body.Kind != model.BodyPermission && !(body.Kind == model.BodyTool && body.Tool.Run != nil) {
		return
	}
	if !asks(message) {
		n.clearPermission(messageID)
		return
	}
	if n.pendingPermissions[messageID] {
		return
	}
	n.pendingPermissions[messageID] = true
	identity := store.IdentityID
	time.AfterFunc(3*time.Second, func() {
		post(func() {
			chat := store.Chat(chatID)
			if chat == nil || store.IdentityID != identity {
				return
			}
			current := chat.Message(messageID)
			if current == nil {
				return
			}
			if notification := notificationFor(current); notification != nil && notification.kind == notifyPermission {
				n.post(notification, chatID)
			}
		})
	})
}

func (n *notifier) turnFinished(chatID, botID string, startedAt time.Time) {
	// Give the final reply and read marks from paired Devices three seconds to arrive.
	identity := store.IdentityID
	time.AfterFunc(3*time.Second, func() {
		post(func() {
			if store.IdentityID != identity || !n.started() {
				return
			}
			if chat := store.Chat(chatID); chat != nil {
				if notification := finishedTurn(chat, botID, startedAt); notification != nil {
					n.post(notification, chatID)
				}
			}
		})
	})
}

func (n *notifier) post(notification *chatNotification, chatID string) {
	chat := store.Chat(chatID)
	if chat == nil || !allowsNotification(notification) || !canDeliver(notification, chat, n.watchedChat()) {
		return
	}
	title := store.Title(chat)
	if bot := store.Bot(notification.botID); bot != nil {
		title = bot.Name
	}
	subtitle := ""
	if !chat.IsBotDM() {
		subtitle = store.Title(chat)
	}
	body := strings.Join(strings.FieldsFunc(notification.body, func(r rune) bool { return r == '\n' || r == '\r' }), " ")
	if runes := []rune(body); len(runes) > 280 {
		body = string(runes[:280])
	}
	showNotice(noticeOptions{ID: notification.messageID, ChatID: chatID, Title: title, Subtitle: subtitle, Body: body})
}
