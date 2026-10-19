package model

import (
	"encoding/json"
	"slices"
	"time"
)

// ListedAgent is a coding agent on the account, whichever bot, chat, or Runner it belongs to, as
// `coding.list` answers it from the CLI's own store: its card, the row that carries it, and the bot
// and chat it belongs to.
type ListedAgent struct {
	ChatID    string
	ChatTitle string
	MessageID string
	BotID     string
	BotName   string
	Agent     *AgentRun
}

// IsLive is an agent the header's Coding agents button counts: one that has not ended and, when
// done, changed in the last three days, as the chat's Coding section keeps it.
func (a ListedAgent) IsLive(now time.Time) bool {
	if a.Agent == nil || !a.Agent.IsOpen() {
		return false
	}
	return a.Agent.IsRunning() || now.Sub(a.Agent.UpdatedAt) <= 3*24*time.Hour
}

type wireListedAgent struct {
	ChatID    string     `json:"chat_id"`
	ChatTitle string     `json:"chat_title"`
	MessageID string     `json:"message_id"`
	BotID     string     `json:"bot_id"`
	BotName   string     `json:"bot_name"`
	Agent     *WireAgent `json:"agent"`
}

// ToListedAgents reads `coding.list`'s answer, in its order.
func ToListedAgents(raw json.RawMessage) ([]ListedAgent, error) {
	var answer struct {
		Agents []wireListedAgent `json:"agents"`
	}
	if err := json.Unmarshal(raw, &answer); err != nil {
		return nil, err
	}
	list := make([]ListedAgent, 0, len(answer.Agents))
	for _, wire := range answer.Agents {
		if wire.Agent == nil {
			continue
		}
		list = append(list, ListedAgent{
			ChatID: wire.ChatID, ChatTitle: wire.ChatTitle, MessageID: wire.MessageID,
			BotID: wire.BotID, BotName: wire.BotName, Agent: ToAgentRun(wire.Agent),
		})
	}
	return list, nil
}

// codingList is the account's coding agents and where fetching them stands: one `coding.list`
// at a time, and once more when something changed while it was on its way.
type codingList struct {
	agents   []ListedAgent
	fetching bool
	again    bool
}

// AccountAgents are the account's coding agents, open first, then the latest. The demo builds
// them from its chats' rows as they stand.
func (s *Store) AccountAgents() []ListedAgent {
	if s.IsMock {
		return s.mockAccountAgents()
	}
	return s.coding.agents
}

// LiveAgents are the account's agents the header's button counts.
func (s *Store) LiveAgents(now time.Time) int {
	count := 0
	for _, agent := range s.AccountAgents() {
		if agent.IsLive(now) {
			count++
		}
	}
	return count
}

// refreshAccountAgents asks the CLI for the account's coding agents again: at once, or after the
// request on its way when one is.
func (s *Store) refreshAccountAgents() {
	if s.IsMock || s.transport == nil {
		return
	}
	if s.coding.fetching {
		s.coding.again = true
		return
	}
	s.coding.fetching = true
	Async(s, func() (json.RawMessage, error) {
		return s.transport.Request("coding.list", map[string]any{})
	}, func(raw json.RawMessage, err error) {
		s.coding.fetching = false
		if err == nil {
			if list, decodeErr := ToListedAgents(raw); decodeErr == nil {
				s.coding.agents = list
				s.emit(Event{Kind: EventCodingAgentsChanged})
			}
		}
		if s.coding.again {
			s.coding.again = false
			s.refreshAccountAgents()
		}
	})
}

// mockAccountAgents is the demo's list: every started agent in its chats, ordered as the CLI
// orders it.
func (s *Store) mockAccountAgents() []ListedAgent {
	var list []ListedAgent
	cutoff := Now().Add(-30 * 24 * time.Hour)
	for _, chat := range s.Chats {
		for _, message := range chat.Messages {
			agent := AgentOf(message)
			if agent == nil || !agent.Started() || !agent.IsOpen() && agentUpdatedAt(message).Before(cutoff) {
				continue
			}
			listed := ListedAgent{ChatID: chat.ID, ChatTitle: s.Title(chat), MessageID: message.ID, BotID: message.Author.BotID, Agent: agent}
			if bot := s.Bot(message.Author.BotID); bot != nil {
				listed.BotName = bot.Name
			}
			list = append(list, listed)
		}
	}
	slices.SortStableFunc(list, func(a, b ListedAgent) int {
		if aOpen, bOpen := a.Agent.IsOpen(), b.Agent.IsOpen(); aOpen != bOpen {
			if aOpen {
				return -1
			}
			return 1
		}
		return b.Agent.UpdatedAt.Compare(a.Agent.UpdatedAt)
	})
	return list
}
