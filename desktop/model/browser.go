package model

import (
	"fmt"
	"slices"
	"strings"
	"time"
)

// Browser profiles and authority live on the assigned Runner. These are the
// CLI's views of them, including explicit capabilities for this requesting Device.
type BrowserSession struct {
	ID        string  `json:"id"`
	BotID     string  `json:"bot_id"`
	RunnerID  string  `json:"runner_id"`
	Account   string  `json:"account"`
	Profile   string  `json:"profile"`
	State     string  `json:"state"`
	Selected  bool    `json:"selected"`
	Revision  uint64  `json:"revision"`
	CreatedAt float64 `json:"created_at"`
}

const (
	BrowserStopped    = "stopped"
	BrowserBotControl = "bot"
	BrowserTakingOver = "taking_over"
	BrowserHuman      = "human"
)

type BrowserCapabilities struct {
	VisibleOpen    bool `json:"visible_open"`
	LocalInput     bool `json:"local_input"`
	Pause          bool `json:"pause"`
	Resume         bool `json:"resume"`
	Stop           bool `json:"stop"`
	Screenshot     bool `json:"screenshot"`
	RemoteLiveView bool `json:"remote_live_view"`
	RemoteInput    bool `json:"remote_input"`
	NativeInput    bool `json:"native_input"`
}

type BrowserSessionList struct {
	Sessions     []BrowserSession    `json:"sessions"`
	Capabilities BrowserCapabilities `json:"capabilities"`
}

type BrowserReply struct {
	Session      *BrowserSession     `json:"session"`
	MessageID    string              `json:"message_id"`
	SessionID    string              `json:"session_id"`
	Capabilities BrowserCapabilities `json:"capabilities"`
}

type BrowserOperation string

const (
	BrowserCreate     BrowserOperation = "browser.create"
	BrowserOpen       BrowserOperation = "browser.open"
	BrowserTakeover   BrowserOperation = "browser.takeover"
	BrowserResume     BrowserOperation = "browser.resume"
	BrowserStop       BrowserOperation = "browser.stop"
	BrowserScreenshot BrowserOperation = "browser.screenshot"
)

// BrowserRefreshAfter posts the next visible sheet refresh to the same ordered
// main-thread queue as CLI replies. The sheet stops this timer on dismissal.
func (s *Store) BrowserRefreshAfter(fn func()) *time.Timer { return s.later(2*time.Second, fn) }

// BrowserSessions asks only the local CLI. Its sealed Runner request determines
// capabilities; the client never infers remote input from the Device's OS.
func (s *Store) BrowserSessions(botID string, done func(BrowserSessionList, error)) {
	if s.IsMock {
		list := s.demoBrowserList(botID)
		s.post(func() { done(list, nil) })
		return
	}
	Async(s, func() (BrowserSessionList, error) {
		return call[BrowserSessionList](s, "browser.sessions", map[string]any{"bot_id": botID})
	}, done)
}

// BrowserAction sends a snapshot of the selected session and current revision,
// with the sheet's bound bot/chat. The Runner rechecks assignment and ownership,
// permissions and input control; this client creates no account or task records.
func (s *Store) BrowserAction(op BrowserOperation, botID, chatID string, session BrowserSession, account, profile string, done func(BrowserReply, error)) {
	params := map[string]any{"bot_id": botID}
	if op == BrowserCreate {
		params["account"], params["profile"] = account, profile
	} else {
		params["session_id"] = session.ID
	}
	switch op {
	case BrowserCreate, BrowserOpen, BrowserTakeover, BrowserStop:
	case BrowserResume:
		params["revision"] = session.Revision
	case BrowserScreenshot:
		params["chat_id"] = chatID
	default:
		s.post(func() { done(BrowserReply{}, &RequestError{L("Unknown browser session action")}) })
		return
	}
	if op != BrowserCreate && (session.ID == "" || session.BotID != botID) {
		s.post(func() {
			done(BrowserReply{}, &RequestError{L("This browser session belongs to another bot or Runner.")})
		})
		return
	}
	if s.IsMock {
		reply, err := s.demoBrowserAction(op, botID, session, account, profile)
		s.post(func() { done(reply, err) })
		return
	}
	Async(s, func() (BrowserReply, error) { return call[BrowserReply](s, string(op), params) }, done)
}

// The in-process demo retains synthetic profiles only in memory. It opens no
// browser and produces no screenshot evidence.
func (s *Store) demoBrowserList(botID string) BrowserSessionList {
	bot := s.Bot(botID)
	if bot == nil {
		return BrowserSessionList{}
	}
	if s.mockBrowser == nil {
		s.mockBrowser = map[string][]BrowserSession{}
	}
	if _, ok := s.mockBrowser[botID]; !ok {
		s.mockBrowser[botID] = []BrowserSession{{ID: "browser-demo-1", BotID: botID, RunnerID: bot.RunnerID, Account: "Demo work account", Profile: "Research", State: BrowserStopped, Selected: true, Revision: 1}}
	}
	runner := s.Device(bot.RunnerID)
	local := runner != nil && runner.IsThisDevice
	return BrowserSessionList{
		Sessions:     slices.Clone(s.mockBrowser[botID]),
		Capabilities: BrowserCapabilities{VisibleOpen: local, LocalInput: local, Pause: true, Resume: true, Stop: true, Screenshot: true},
	}
}

func (s *Store) demoBrowserAction(op BrowserOperation, botID string, selected BrowserSession, account, profile string) (BrowserReply, error) {
	list := s.demoBrowserList(botID)
	bot := s.Bot(botID)
	if bot == nil {
		return BrowserReply{}, &RequestError{L("Unknown bot")}
	}
	if op == BrowserCreate {
		account, profile = strings.TrimSpace(account), strings.TrimSpace(profile)
		if account == "" || profile == "" {
			return BrowserReply{}, &RequestError{L("Account and profile labels require 1–100 printable characters.")}
		}
		for i := range list.Sessions {
			list.Sessions[i].Selected = false
		}
		next := BrowserSession{ID: fmt.Sprintf("browser-demo-%d", len(list.Sessions)+1), BotID: botID, RunnerID: bot.RunnerID, Account: account, Profile: profile, State: BrowserStopped, Selected: true, Revision: 1}
		s.mockBrowser[botID] = append(list.Sessions, next)
		return BrowserReply{Session: &next, Capabilities: list.Capabilities}, nil
	}
	i := slices.IndexFunc(list.Sessions, func(each BrowserSession) bool { return each.ID == selected.ID })
	if i < 0 || list.Sessions[i].RunnerID != bot.RunnerID {
		return BrowserReply{}, &RequestError{L("This browser session belongs to another bot or Runner.")}
	}
	next := list.Sessions[i]
	switch op {
	case BrowserOpen:
		if !list.Capabilities.VisibleOpen {
			return BrowserReply{}, &RequestError{L("Visible open requires the local Runner.")}
		}
		next.State = BrowserHuman
	case BrowserTakeover:
		if next.State == BrowserStopped {
			return BrowserReply{}, &RequestError{L("Open the session on its Runner before taking over.")}
		}
		next.State = BrowserHuman
	case BrowserResume:
		if next.State != BrowserHuman || next.Revision != selected.Revision {
			return BrowserReply{}, &RequestError{L("Browser control changed. Refresh before returning control.")}
		}
		next.State = BrowserBotControl
	case BrowserStop:
		next.State = BrowserStopped
	case BrowserScreenshot:
		return BrowserReply{}, &RequestError{L("Screenshot capture requires a live Runner.")}
	}
	next.Revision++
	if op == BrowserOpen || op == BrowserResume {
		for j := range list.Sessions {
			list.Sessions[j].Selected = j == i
		}
		next.Selected = true
	}
	list.Sessions[i] = next
	s.mockBrowser[botID] = list.Sessions
	return BrowserReply{Session: &next, Capabilities: list.Capabilities}, nil
}
