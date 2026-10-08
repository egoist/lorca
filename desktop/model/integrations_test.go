package model

import (
	"encoding/json"
	"errors"
	"testing"
	"time"
)

type integrationRequest struct {
	method string
	params map[string]any
}
type integrationTransport struct {
	calls chan integrationRequest
	reply string
	err   error
}

func (rpc *integrationTransport) Reconnect() {}
func (rpc *integrationTransport) Request(method string, params any) (json.RawMessage, error) {
	raw, _ := json.Marshal(params)
	var body map[string]any
	_ = json.Unmarshal(raw, &body)
	rpc.calls <- integrationRequest{method, body}
	return json.RawMessage(rpc.reply), rpc.err
}

func TestNamedAccountWireKeepsInstanceServiceAndRecoveryState(t *testing.T) {
	var wire WirePluginStatus
	err := json.Unmarshal([]byte(`{"id":"gmail-0123456789abcdef0123456789abcdef","name":"Gmail · Work","service_id":"gmail","account_name":"Work","state":"insufficient_access","detail":"Sign in again"}`), &wire)
	if err != nil {
		t.Fatal(err)
	}
	account := ToPlugin(wire)
	if account.MarketplaceID() != "gmail" || account.ID == account.ServiceID || account.AccountName != "Work" || account.State != PluginInsufficientAccess || account.State.Tone() != ToneOrange {
		t.Fatalf("account: %+v", account)
	}
	var legacy WirePluginStatus
	_ = json.Unmarshal([]byte(`{"id":"github","name":"GitHub","state":"ready"}`), &legacy)
	if ToPlugin(legacy).MarketplaceID() != "github" || ToPlugin(legacy).ServiceID != "" {
		t.Fatal("legacy status changed")
	}
	var marketplace WireMarketplacePlugin
	_ = json.Unmarshal([]byte(`{"id":"gmail","name":"Gmail","named_accounts":true}`), &marketplace)
	if !ToMarketplacePlugin(marketplace).NamedAccounts {
		t.Fatal("named account manifest flag missing")
	}
}

func TestAccountRequestsSelectRunnerAndInstanceAndPublishOnReplyQueue(t *testing.T) {
	posts := make(chan func(), 8)
	rpc := &integrationTransport{calls: make(chan integrationRequest, 8), reply: `{"status":{"id":"gmail-instance","name":"Gmail · Work","service_id":"gmail","account_name":"Work","state":"ready"}}`}
	s := NewStore(rpc, func(fn func()) { posts <- fn }, false)
	s.Devices = []*Device{{ID: "runner", OS: OSLinux}}
	called := false
	s.InstallPluginAccount("gmail", "runner", "Work", func(account InstalledPlugin, err error) {
		called = true
		if err != nil || account.ID != "gmail-instance" {
			t.Errorf("install: %+v %v", account, err)
		}
	})
	request := <-rpc.calls
	if request.method != "plugins.install" || request.params["plugin_id"] != "gmail" || request.params["runner_id"] != "runner" || request.params["account_name"] != "Work" {
		t.Fatal(request)
	}
	var reply func()
	select {
	case reply = <-posts:
	case <-time.After(3 * time.Second):
		t.Fatal("no posted reply")
	}
	if called || len(s.accounts("gmail", "runner")) != 0 {
		t.Fatal("worker mutated model before the posted reply")
	}
	reply()
	if !called || len(s.accounts("gmail", "runner")) != 1 {
		t.Fatal("reply did not publish the instance")
	}

	rpc.reply = `{"status":{"id":"gmail-instance","name":"Gmail · Office","service_id":"gmail","account_name":"Office","state":"ready"}}`
	s.RenamePluginAccount("gmail-instance", "runner", "Office", func(account InstalledPlugin, err error) {
		if err != nil || account.ID != "gmail-instance" {
			t.Errorf("rename: %+v %v", account, err)
		}
	})
	request = <-rpc.calls
	if request.method != "plugins.rename" || request.params["plugin_id"] != "gmail-instance" || request.params["account_name"] != "Office" || request.params["runner_id"] != "runner" {
		t.Fatal(request)
	}
	(<-posts)()
	if s.accounts("gmail", "runner")[0].AccountName != "Office" {
		t.Fatal("renamed status not retained")
	}
	rpc.err = errors.New("denied by runner")
	s.RenamePluginAccount("gmail-instance", "runner", "Personal", func(_ InstalledPlugin, err error) {
		if err == nil {
			t.Error("error lost")
		}
	})
	<-rpc.calls
	(<-posts)()
	if s.accounts("gmail", "runner")[0].AccountName != "Office" {
		t.Fatal("failed rename replaced the account")
	}
	rpc.err = nil
	for _, method := range []string{"plugins.connect", "plugins.sign_out"} {
		if method == "plugins.connect" {
			s.ConnectPlugin("gmail-instance", "runner", func(err error) {
				if err != nil {
					t.Error(err)
				}
			})
		} else {
			s.SignOutPlugin("gmail-instance", "runner", "gmail", func(err error) {
				if err != nil {
					t.Error(err)
				}
			})
		}
		request = <-rpc.calls
		if request.method != method || request.params["plugin_id"] != "gmail-instance" || request.params["runner_id"] != "runner" {
			t.Fatal(request)
		}
		(<-posts)()
	}
}
