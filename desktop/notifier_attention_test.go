package main

import (
	"github.com/egoist/lorca/desktop/model"
	"testing"
	"time"
)

func TestAttentionNotificationsRespectTagsPreferencesAndReadState(t *testing.T) {
	demoWindow(t)
	message := &model.Message{ID: "notice", Author: model.BotAuthor("bot-nova"), Body: model.Body{Kind: model.BodyText, Text: "A decision changed"}, State: model.MessageState{Kind: model.StateComplete}, CreatedAt: time.Now()}
	chat := &model.Chat{ID: "chat", Messages: []*model.Message{message}, UnreadCount: 1}
	for _, test := range []struct {
		tag                      model.NotificationTag
		kind                     notificationKind
		summary, urgent, allowed bool
	}{
		{model.NotificationSummary, notifySummary, true, false, true},
		{model.NotificationSummary, notifySummary, false, true, false},
		{model.NotificationUrgent, notifyUrgent, false, true, true},
		{model.NotificationUrgent, notifyUrgent, true, false, false},
	} {
		message.Notification = test.tag
		store.Attention.Preferences.Summaries = test.summary
		store.Attention.Preferences.UrgentDirect = test.urgent
		notice := notificationFor(message)
		if notice == nil || notice.kind != test.kind || allowsNotification(notice) != test.allowed {
			t.Fatalf("tag/prefs %+v: %+v", test, notice)
		}
		if finishedTurn(chat, "bot-nova", message.CreatedAt) != nil {
			t.Fatal("structured message alerts twice at turn end")
		}
		if !canDeliver(notice, chat, "") || canDeliver(notice, chat, "chat") {
			t.Fatal("watched-chat policy")
		}
		chat.UnreadCount = 0
		if canDeliver(notice, chat, "") {
			t.Fatal("read-on-peer policy")
		}
		chat.UnreadCount = 1
	}
	message.Notification = model.NotificationQuiet
	message.State.Kind = model.StateFailed
	if notificationFor(message) != nil || finishedTurn(chat, "bot-nova", message.CreatedAt) != nil {
		t.Fatal("quiet report/failure alerted")
	}
	message.Notification = ""
	message.State.Kind = model.StateComplete
	if finishedTurn(chat, "bot-nova", message.CreatedAt) == nil {
		t.Fatal("ordinary reply was silenced")
	}
}
