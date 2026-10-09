package main

import (
	"fmt"
	"net/url"
	"runtime"
	"strconv"
	"strings"
	"unicode/utf16"

	"github.com/egoist/mygo"
)

// What the app does with the system around its windows: the unread badge, links, and
// notifications.

// setBadge shows the chats' unread count where the system has a place for it: the launcher entry
// on Linux, the tray's tooltip elsewhere. Zero clears it.
func setBadge(count int) {
	mygo.App.SetBadgeCount(count)
	if app.tray != nil {
		tip := appName()
		if count > 0 {
			tip += " · " + strconv.Itoa(count)
		}
		app.tray.SetToolTip(tip)
	}
}

// openExternal opens a web link in the browser or a mail link in the mail app. The links come from
// bots, plugins, and the marketplace, so any other scheme is refused: the system would start
// whatever app handles it.
func openExternal(link string) error {
	u, err := url.Parse(link)
	if err != nil || !(u.Scheme == "mailto" || (u.Scheme == "http" || u.Scheme == "https") && u.Host != "") {
		return fmt.Errorf("not a web or mail link: %q", link)
	}
	go mygo.Shell.OpenExternal(link)
	return nil
}

// noticeOptions is one system notification: a reply, a failed response, or a question.
type noticeOptions struct {
	// ID is the message's; a question's notification goes when it is answered.
	ID       string
	ChatID   string
	Title    string
	Subtitle string
	Body     string
}

type postedNotice struct {
	notification *mygo.Notification
	chatID       string
}

// notices are the notifications posted for the chats, taken back once they are seen. A click
// brings the app forward on the chat. Main thread only.
var notices = map[string]postedNotice{}

// showNotice posts a notification; one with the same id replaces it.
func showNotice(options noticeOptions) {
	if !mygo.NotificationsSupported() {
		return
	}
	title, body := options.Title, options.Body
	// Windows shows a notification as a tray balloon, which holds 63 UTF-16 units of title and 255
	// of text, the subtitle's line among them: the words are cut to fit, ending in an ellipsis.
	if runtime.GOOS == "windows" {
		title = fitUTF16(title, 63)
		room := 255
		if options.Subtitle != "" {
			room -= len(utf16.Encode([]rune(options.Subtitle))) + 1
		}
		body = fitUTF16(body, room)
	}
	notification := mygo.NewNotification(mygo.NotificationOptions{
		Title:    title,
		Subtitle: options.Subtitle,
		Body:     body,
	})
	chatID := options.ChatID
	notification.OnClick(func() { app.openChat(chatID) })
	removeNotice(options.ID)
	notices[options.ID] = postedNotice{notification: notification, chatID: chatID}
	go func() {
		if err := notification.Show(); err != nil {
			post(func() {
				if notices[options.ID].notification == notification {
					delete(notices, options.ID)
				}
			})
		}
	}()
}

// removeNotice takes back the notification of a message, such as a question answered elsewhere.
func removeNotice(id string) {
	if old, ok := notices[id]; ok {
		delete(notices, id)
		go old.notification.Close()
	}
}

// clearNotices takes back what was posted for a chat that is now on screen.
func clearNotices(chatID string) {
	for id, entry := range notices {
		if entry.chatID == chatID {
			delete(notices, id)
			go entry.notification.Close()
		}
	}
}

// fitUTF16 cuts s to at most n UTF-16 units, the last of them an ellipsis when it cuts.
func fitUTF16(s string, n int) string {
	if len(utf16.Encode([]rune(s))) <= n {
		return s
	}
	if n <= 0 {
		return ""
	}
	var kept []rune
	used := 0
	for _, r := range s {
		size := utf16.RuneLen(r)
		if used+size > n-1 {
			break
		}
		kept = append(kept, r)
		used += size
	}
	return strings.TrimRight(string(kept), " ") + "…"
}

// appVersion is the build's version, "dev" for a development build.
func appVersion() string {
	if version := mygo.App.Version(); version != "" {
		return version
	}
	return "dev"
}
