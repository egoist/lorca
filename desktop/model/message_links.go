package model

import (
	"net/url"
	"strings"
)

// MessageLink names one locally synced chat and an opaque message id. It is navigation,
// never a system URL handler or an authority to run a task.
type MessageLink struct {
	ChatID    string
	MessageID string
}

// ParseMessageLink accepts only the internal message-reference shape emitted by handoffs.
// Query values are decoded once; unrelated schemes, authority fields and ambiguous ids fail.
func ParseMessageLink(href string) (MessageLink, bool) {
	u, err := url.Parse(href)
	if err != nil || !strings.EqualFold(u.Scheme, "lorca") || u.Host != "message" || u.User != nil || u.Opaque != "" || u.Path != "" || u.Fragment != "" || strings.Contains(href, "#") {
		return MessageLink{}, false
	}
	query, err := url.ParseQuery(u.RawQuery)
	if err != nil || len(strings.Split(u.RawQuery, "&")) != 2 || len(query) != 2 || len(query["chat_id"]) != 1 || len(query["message_id"]) != 1 {
		return MessageLink{}, false
	}
	link := MessageLink{ChatID: query.Get("chat_id"), MessageID: query.Get("message_id")}
	return link, link.ChatID != "" && link.MessageID != ""
}

// IsExternalLink preserves the desktop host's web/mail allowlist. Internal navigation does
// not broaden which schemes the operating system may open.
func IsExternalLink(href string) bool {
	u, err := url.Parse(href)
	return err == nil && (u.Scheme == "mailto" || (u.Scheme == "http" || u.Scheme == "https") && u.Host != "")
}
