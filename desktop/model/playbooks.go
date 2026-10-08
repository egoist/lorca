package model

import (
	"encoding/json"
	"slices"
	"strings"
)

// Playbook records are owned by the CLI. These are wire/presentation values, not a second store.
type PlaybookScope struct {
	Kind string `json:"kind"`
	ID   string `json:"id"`
}

func (s PlaybookScope) Key() string { return s.Kind + ":" + s.ID }

type PlaybookResource struct {
	Path string `json:"path"`
	Text string `json:"text"`
}

type PlaybookContent struct {
	Name         string             `json:"name"`
	Description  string             `json:"description"`
	Instructions string             `json:"instructions"`
	Examples     string             `json:"examples"`
	References   []PlaybookResource `json:"references,omitempty"`
	Scripts      []PlaybookResource `json:"scripts,omitempty"`
}

func (c PlaybookContent) Clone() PlaybookContent {
	c.References, c.Scripts = slices.Clone(c.References), slices.Clone(c.Scripts)
	return c
}

type PlaybookProvenance struct {
	Kind       string   `json:"kind"`
	ChatID     *string  `json:"chat_id,omitempty"`
	MessageIDs []string `json:"message_ids"`
	Note       string   `json:"note"`
}

type PlaybookSummary struct {
	ID          string        `json:"id"`
	Scope       PlaybookScope `json:"scope"`
	Name        string        `json:"name"`
	Description string        `json:"description"`
	Path        string        `json:"path"`
	Revision    uint64        `json:"revision"`
	Hash        string        `json:"hash"`
	Status      string        `json:"status"`
}

type PlaybookRevision struct {
	Revision   uint64             `json:"revision"`
	Status     string             `json:"status"`
	Content    *PlaybookContent   `json:"content"`
	Provenance PlaybookProvenance `json:"provenance"`
	CreatedAt  float64            `json:"created_at"`
}

type PlaybookRecord struct {
	PlaybookSummary
	Content    *PlaybookContent   `json:"content"`
	Provenance PlaybookProvenance `json:"provenance"`
	Revisions  []PlaybookRevision `json:"revisions"`
}

func (s *Store) Playbooks(scope PlaybookScope, done func([]PlaybookSummary, error)) {
	Async(s, func() ([]PlaybookSummary, error) {
		reply, err := call[struct {
			Items []PlaybookSummary `json:"items"`
		}](s, "playbooks.list", map[string]any{"scope": scope, "include_drafts": true})
		return reply.Items, err
	}, done)
}

func (s *Store) Playbook(scope PlaybookScope, id string, done func(PlaybookRecord, error)) {
	Async(s, func() (PlaybookRecord, error) {
		return call[PlaybookRecord](s, "playbooks.get", map[string]any{"scope": scope, "id": id})
	}, done)
}

// SavePlaybook always sends both optimistic guards. Only the reviewed Save action calls it.
func (s *Store) SavePlaybook(scope PlaybookScope, content PlaybookContent, previous *PlaybookRecord, done func(PlaybookRecord, error)) {
	params := map[string]any{"scope": scope, "content": content.Clone(), "expected_revision": uint64(0), "expected_hash": ""}
	if previous != nil {
		params["id"], params["expected_revision"], params["expected_hash"] = previous.ID, previous.Revision, previous.Hash
		provenance := previous.Provenance
		provenance.MessageIDs = slices.Clone(provenance.MessageIDs)
		if provenance.MessageIDs == nil {
			provenance.MessageIDs = []string{}
		}
		provenance.Kind, provenance.Note = "reviewed_edit", L("Saved after user review")
		params["provenance"] = provenance
	}
	Async(s, func() (PlaybookRecord, error) { return call[PlaybookRecord](s, "playbooks.save", params) }, done)
}

func (s *Store) RemovePlaybook(item PlaybookSummary, done func(PlaybookRecord, error)) {
	params := map[string]any{"scope": item.Scope, "id": item.ID, "expected_revision": item.Revision, "expected_hash": item.Hash}
	Async(s, func() (PlaybookRecord, error) { return call[PlaybookRecord](s, "playbooks.remove", params) }, done)
}

// ExportPlaybook returns exactly the CLI's portable content-only package; it adds no UI context.
func (s *Store) ExportPlaybook(item PlaybookSummary, done func(json.RawMessage, error)) {
	Async(s, func() (json.RawMessage, error) {
		return call[json.RawMessage](s, "playbooks.export", map[string]any{"scope": item.Scope, "id": item.ID})
	}, done)
}

func (s *Store) DraftPlaybook(scope PlaybookScope, botID, chatID, kind string, ids []string, done func(PlaybookRecord, error)) {
	params := map[string]any{"scope": scope, "bot_id": botID, "chat_id": chatID, "kind": kind, "message_ids": slices.Clone(ids)}
	Async(s, func() (PlaybookRecord, error) { return call[PlaybookRecord](s, "playbooks.draft", params) }, done)
}

func (s *Store) PlaybookScopes(chatID string) []PlaybookScope {
	chat := s.Chat(chatID)
	if chat == nil {
		return nil
	}
	var scopes []PlaybookScope
	if chat.IsGroup() {
		scopes = append(scopes, PlaybookScope{"project", chat.ID})
	}
	for _, id := range chat.BotIDs {
		if s.Bot(id) != nil {
			scopes = append(scopes, PlaybookScope{"bot", id})
		}
	}
	return scopes
}

// CaptureSources mirrors the CLI's visible source constraints. The CLI validates evidence again.
func CaptureSources(chat *Chat, selected *Message) (botID, kind string, sources []*Message, picked map[string]bool) {
	if chat == nil || selected == nil || !selected.CanBeQuoted() || strings.TrimSpace(selected.Body.Text) == "" {
		return
	}
	botID, kind = selected.Author.BotID, "workflow"
	if selected.Author.Kind == AuthorYou {
		botID, kind = chat.Owner(), "corrections"
		if botID == "" && len(chat.BotIDs) > 0 {
			botID = chat.BotIDs[0]
		}
	}
	if !slices.Contains(chat.BotIDs, botID) {
		return "", "", nil, nil
	}
	preceding := ""
	for _, m := range chat.Messages {
		if !m.CanBeQuoted() || strings.TrimSpace(m.Body.Text) == "" {
			continue
		}
		if m.Author.Kind != AuthorYou && (kind == "corrections" || m.Author.BotID != botID) {
			continue
		}
		sources = append(sources, m)
		if m.Author.Kind == AuthorYou && m.CreatedAt.Before(selected.CreatedAt) {
			preceding = m.ID
		}
	}
	if len(sources) > 20 {
		sources = sources[len(sources)-20:]
	}
	if !slices.ContainsFunc(sources, func(m *Message) bool { return m.ID == selected.ID }) {
		sources = append([]*Message{selected}, sources...)
	}
	picked = map[string]bool{selected.ID: true}
	if kind == "workflow" && preceding != "" {
		picked[preceding] = true
	}
	return
}
