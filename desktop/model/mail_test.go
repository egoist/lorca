package model

import (
	"encoding/json"
	"testing"
)

func TestMailNamesFollowTheRelaysRule(t *testing.T) {
	for name, valid := range map[string]bool{
		"egoist": true, "k7f3m9q2": true, "a.b-c": true, "abc": true,
		"ab": false, "-abc": false, "abc.": false, "a..b": false, "a+b": false, "Ab": false,
		"abcdefghijklmnopqrstuvwxyz0123456": false,
	} {
		if ValidMailName(name) != valid {
			t.Errorf("%q valid = %v", name, !valid)
		}
	}
	if NormalizeMailName("  Egoist ") != "egoist" {
		t.Error("names are trimmed and lowercased")
	}
}

func TestMailStatusFromTheSnapshot(t *testing.T) {
	var snapshot WireSnapshot
	if err := json.Unmarshal([]byte(`{"mail":null}`), &snapshot); err != nil || snapshot.Mail != nil {
		t.Fatalf("no status yet: %+v %v", snapshot.Mail, err)
	}
	data := `{"mail":{"available":true,"domain":"bots.lorca.app","address":{"name":"k7f3m9q2","email":"k7f3m9q2@bots.lorca.app","state":"suspended"},"lead_bot_id":"bot-nova","bots":[{"bot_id":"bot-scout","email":"k7f3m9q2+researcher@bots.lorca.app"}]}}`
	if err := json.Unmarshal([]byte(data), &snapshot); err != nil {
		t.Fatal(err)
	}
	mail := snapshot.Mail
	if mail == nil || !mail.IsSuspended() || mail.BotEmail("bot-scout") != "k7f3m9q2+researcher@bots.lorca.app" || mail.BotEmail("bot-nova") != "" {
		t.Fatalf("status %+v", mail)
	}
}
