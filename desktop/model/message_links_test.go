package model

import "testing"

func TestParseHandoffMessageLink(t *testing.T) {
	for _, test := range []struct{ href, chat, message string }{
		{"lorca://message?chat_id=chat-a&message_id=msg-b", "chat-a", "msg-b"},
		{"LORCA://message?message_id=msg%2Fwith%2Bsign&chat_id=chat%20with%20spaces", "chat with spaces", "msg/with+sign"},
		{"lorca://message?chat_id=%E8%81%8A%E5%A4%A9&message_id=%23reference", "聊天", "#reference"},
	} {
		got, ok := ParseMessageLink(test.href)
		if !ok || got.ChatID != test.chat || got.MessageID != test.message {
			t.Fatalf("%q = %+v, %v", test.href, got, ok)
		}
	}
	for _, href := range []string{
		"https://message?chat_id=c&message_id=m", "lorca://pair?chat_id=c&message_id=m", "lorca:message?chat_id=c&message_id=m",
		"lorca://message/path?chat_id=c&message_id=m", "lorca://message:80?chat_id=c&message_id=m", "lorca://user@message?chat_id=c&message_id=m",
		"lorca://message?chat_id=c", "lorca://message?chat_id=&message_id=m", "lorca://message?chat_id=c&message_id=", "lorca://message?chat_id=c&message_id=m&extra=x",
		"lorca://message?chat_id=c&chat_id=d&message_id=m", "lorca://message?chat_id=c&message_id=m&message_id=n",
		"lorca://message?chat_id=%zz&message_id=m", "lorca://message?chat_id=c&message_id=m#fragment", "lorca://message?chat_id=c&message_id=m#",
		"lorca://message?chat_id=c&&message_id=m", "lorca://message?chat_id=c&message_id=m&", "lorca://message?chat_id=c;extra&message_id=m",
	} {
		if link, ok := ParseMessageLink(href); ok {
			t.Errorf("accepted %q as %+v", href, link)
		}
	}
}

func TestHandoffNavigationPreservesExternalAllowlist(t *testing.T) {
	for _, href := range []string{"https://example.com/result", "http://localhost:4862/help", "mailto:user@example.com"} {
		if !IsExternalLink(href) {
			t.Errorf("refused allowed external link %q", href)
		}
	}
	for _, href := range []string{"lorca://message?chat_id=c&message_id=m", "lorca://pair", "file:///tmp/private", "javascript:alert(1)", "data:text/plain,hello", "ftp://example.com", "https:relative", "https://", "//example.com", "not a url"} {
		if IsExternalLink(href) {
			t.Errorf("allowed external opener for %q", href)
		}
	}
}
