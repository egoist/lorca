package model

import (
	"fmt"
	"maps"
	"slices"
)

// BrowserPluginID is the Browser plugin, whose sheet lists a bot's browser profiles.
const BrowserPluginID = "playwright"

// BrowserProfile is one of a bot's browser profiles, as its Runner reports it. Each keeps its own
// sign-ins.
type BrowserProfile struct {
	ID    string `json:"id"`
	Name  string `json:"name"`
	State string `json:"state"`
	// Revision goes up with every change of control; Return to Bot sends the one this app last saw.
	Revision uint64 `json:"revision"`
	// Recording while the user records a workflow in it for the bot to learn, with the browser in
	// hand.
	Recording bool `json:"recording"`
}

// How a profile's browser stands.
const (
	BrowserStopped    = "stopped"
	BrowserBot        = "bot"
	BrowserTakingOver = "taking_over"
	BrowserHuman      = "human"
)

// BrowserProfiles asks the bot's Runner for its profiles, oldest first: this computer's CLI, or a
// sealed request to the Runner.
func (s *Store) BrowserProfiles(botID string, done func([]BrowserProfile, error)) {
	if s.IsMock {
		profiles := slices.Clone(s.mockBrowser[botID])
		s.post(func() { done(profiles, nil) })
		return
	}
	Async(s, func() ([]BrowserProfile, error) {
		list, err := call[struct {
			Sessions []BrowserProfile `json:"sessions"`
		}](s, "browser.sessions", map[string]any{"bot_id": botID})
		return list.Sessions, err
	}, done)
}

// BrowserProfileAction sends browser.create (name), browser.open, browser.takeover, browser.resume
// (revision), browser.stop, browser.delete, browser.screenshot (chat_id), or browser.record for one
// of the bot's profiles (session_id).
func (s *Store) BrowserProfileAction(method, botID string, params map[string]any, done func(error)) {
	params = maps.Clone(params)
	params["bot_id"] = botID
	if s.IsMock {
		s.demoBrowser(method, botID, params)
	}
	s.simple(done, method, params)
}

// StopBrowserRecording stops the profile's recording and sends it to the bot in the chat with the
// user's words. Sent is false when the user did nothing in the browser, so nothing went.
func (s *Store) StopBrowserRecording(botID, sessionID, chatID, text string, done func(sent bool, err error)) {
	params := map[string]any{"bot_id": botID, "session_id": sessionID, "chat_id": chatID, "text": text}
	if s.IsMock {
		s.demoBrowser("browser.stop_recording", botID, params)
		s.post(func() { done(true, nil) })
		return
	}
	Async(s, func() (bool, error) {
		stopped, err := call[struct {
			MessageID *string `json:"message_id"`
		}](s, "browser.stop_recording", params)
		return stopped.MessageID != nil, err
	}, done)
}

// demoBrowser is the demo's profiles: in memory, with no browser behind them.
func (s *Store) demoBrowser(method, botID string, params map[string]any) {
	if s.mockBrowser == nil {
		s.mockBrowser = map[string][]BrowserProfile{}
	}
	list := s.mockBrowser[botID]
	id, _ := params["session_id"].(string)
	set := func(state string, recording bool) {
		for i := range list {
			if list[i].ID == id {
				list[i].State, list[i].Revision, list[i].Recording = state, list[i].Revision+1, recording
			}
		}
	}
	switch method {
	case "browser.create":
		name, _ := params["name"].(string)
		list = append(list, BrowserProfile{ID: fmt.Sprintf("browser-demo-%d", len(list)+1), Name: name, State: BrowserStopped, Revision: 1})
	case "browser.open", "browser.takeover", "browser.stop_recording":
		set(BrowserHuman, false)
	case "browser.record":
		set(BrowserHuman, true)
	case "browser.resume":
		set(BrowserBot, false)
	case "browser.stop":
		set(BrowserStopped, false)
	case "browser.delete":
		list = slices.DeleteFunc(list, func(profile BrowserProfile) bool { return profile.ID == id })
	}
	s.mockBrowser[botID] = list
}
