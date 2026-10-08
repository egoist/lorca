package model

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"slices"
	"strings"
	"time"
)

// A bot's or a group's skills, after the macOS app's Playbooks.swift. The CLI owns them: the
// roster lists them without their bodies, and a body is fetched when one opens.

// PlaybookScope is whose skill it is: one bot's, in every chat it is in, or a group's, for the bots
// in that group.
type PlaybookScope struct {
	Kind string `json:"kind"`
	ID   string `json:"id"`
}

func BotScope(id string) PlaybookScope   { return PlaybookScope{Kind: "bot", ID: id} }
func GroupScope(id string) PlaybookScope { return PlaybookScope{Kind: "project", ID: id} }

func (s PlaybookScope) IsGroup() bool { return s.Kind == "project" }

// SkillScope is whose skills a chat shows: the bot's in a DM, the group's in a group.
func SkillScope(chat *Chat, members []*Bot) (PlaybookScope, bool) {
	if chat == nil {
		return PlaybookScope{}, false
	}
	if chat.IsGroup() {
		return GroupScope(chat.ID), true
	}
	if len(members) == 0 {
		return PlaybookScope{}, false
	}
	return BotScope(members[0].ID), true
}

// PlaybookFile is a reference or a script the skill carries, as text: `references/checklist.md`.
type PlaybookFile struct {
	Path string `json:"path"`
	Text string `json:"text"`
}

type PlaybookContent struct {
	Name         string         `json:"name"`
	Description  string         `json:"description"`
	Instructions string         `json:"instructions"`
	Examples     string         `json:"examples"`
	References   []PlaybookFile `json:"references"`
	Scripts      []PlaybookFile `json:"scripts"`
}

func (c PlaybookContent) clone() PlaybookContent {
	c.References, c.Scripts = slices.Clone(c.References), slices.Clone(c.Scripts)
	if c.References == nil {
		c.References = []PlaybookFile{}
	}
	if c.Scripts == nil {
		c.Scripts = []PlaybookFile{}
	}
	return c
}

// PlaybookSummary is a skill as the roster lists it, without its body.
type PlaybookSummary struct {
	ID          string        `json:"id"`
	Scope       PlaybookScope `json:"scope"`
	Name        string        `json:"name"`
	Description string        `json:"description"`
	Status      string        `json:"status"`
	Revision    uint64        `json:"revision"`
	Hash        string        `json:"hash"`
	UpdatedAt   float64       `json:"updated_at"`
}

func (s PlaybookSummary) IsDraft() bool { return s.Status == "draft" }

// PlaybookRevision is one step of a skill's history: who changed it where, and what it said then.
type PlaybookRevision struct {
	ID         string           `json:"id"`
	Revision   uint64           `json:"revision"`
	Status     string           `json:"status"`
	Content    *PlaybookContent `json:"content"`
	Provenance struct {
		Kind       string   `json:"kind"`
		ChatID     *string  `json:"chat_id"`
		MessageIDs []string `json:"message_ids"`
	} `json:"provenance"`
	DeviceID  string  `json:"device_id"`
	CreatedAt float64 `json:"created_at"`
}

// PlaybookRecord is a skill with its body and history, fetched when it opens.
type PlaybookRecord struct {
	ID        string             `json:"id"`
	Scope     PlaybookScope      `json:"scope"`
	Status    string             `json:"status"`
	Revision  uint64             `json:"revision"`
	Hash      string             `json:"hash"`
	Content   *PlaybookContent   `json:"content"`
	Revisions []PlaybookRevision `json:"revisions"`
}

func (r PlaybookRecord) IsDraft() bool { return r.Status == "draft" }

func (r PlaybookRecord) summary() PlaybookSummary {
	s := PlaybookSummary{ID: r.ID, Scope: r.Scope, Status: r.Status, Revision: r.Revision, Hash: r.Hash}
	if r.Content != nil {
		s.Name, s.Description = r.Content.Name, r.Content.Description
	}
	for _, step := range r.Revisions {
		s.UpdatedAt = max(s.UpdatedAt, step.CreatedAt)
	}
	return s
}

// ErrPlaybookChanged is a save or a delete refused because the skill changed on another Device.
var ErrPlaybookChanged = errors.New("This skill changed on another Device")

func playbookError(err error) error {
	if err != nil && strings.Contains(err.Error(), "changed since") {
		return ErrPlaybookChanged
	}
	return err
}

// Skills are a bot's or a group's skills: drafts waiting for review first, then the latest changed.
func (s *Store) Skills(scope PlaybookScope) []PlaybookSummary {
	var skills []PlaybookSummary
	for _, skill := range s.Playbooks {
		if skill.Scope == scope {
			skills = append(skills, skill)
		}
	}
	slices.SortStableFunc(skills, func(a, b PlaybookSummary) int {
		if a.IsDraft() != b.IsDraft() {
			if a.IsDraft() {
				return -1
			}
			return 1
		}
		switch {
		case a.UpdatedAt > b.UpdatedAt:
			return -1
		case a.UpdatedAt < b.UpdatedAt:
			return 1
		}
		return 0
	})
	return skills
}

func (s *Store) Playbook(scope PlaybookScope, id string, done func(PlaybookRecord, error)) {
	if s.IsMock {
		record, ok := PlaybookRecord{}, false
		for _, each := range s.mockPlaybooks {
			if each.ID == id {
				record, ok = each, true
			}
		}
		s.post(func() {
			if !ok {
				done(record, errors.New(L("This skill was deleted.")))
				return
			}
			done(record, nil)
		})
		return
	}
	Async(s, func() (PlaybookRecord, error) {
		return call[PlaybookRecord](s, "playbooks.get", map[string]any{"scope": scope, "id": id})
	}, done)
}

// SavePlaybook saves a new skill, or a new revision over the one `over` holds. The CLI refuses it
// with ErrPlaybookChanged when the skill changed since `over` was read.
func (s *Store) SavePlaybook(scope PlaybookScope, content PlaybookContent, over *PlaybookRecord, done func(PlaybookRecord, error)) {
	if s.IsMock {
		saved := s.saveMockPlaybook(content.clone(), scope, over, "saved")
		s.post(func() { done(saved, nil) })
		return
	}
	params := map[string]any{"scope": scope, "content": content.clone(), "expected_revision": uint64(0), "expected_hash": ""}
	if over != nil {
		params["id"], params["expected_revision"], params["expected_hash"] = over.ID, over.Revision, over.Hash
		params["provenance"] = map[string]any{"kind": "edit"}
	}
	Async(s, func() (PlaybookRecord, error) {
		record, err := call[PlaybookRecord](s, "playbooks.save", params)
		return record, playbookError(err)
	}, done)
}

func (s *Store) RemovePlaybook(record PlaybookRecord, done func(error)) {
	if s.IsMock {
		s.mockPlaybooks = slices.DeleteFunc(slices.Clone(s.mockPlaybooks), func(each PlaybookRecord) bool { return each.ID == record.ID })
		s.mockPlaybooksChanged()
		s.post(func() { done(nil) })
		return
	}
	params := map[string]any{"scope": record.Scope, "id": record.ID, "expected_revision": record.Revision, "expected_hash": record.Hash}
	Async(s, func() (struct{}, error) {
		_, err := call[json.RawMessage](s, "playbooks.remove", params)
		return struct{}{}, playbookError(err)
	}, func(_ struct{}, err error) { done(err) })
}

// DraftPlaybook has the bot's provider write a draft from the messages picked in the chat; the
// draft is used once it is saved.
func (s *Store) DraftPlaybook(scope PlaybookScope, botID, chatID, kind string, messageIDs []string, done func(PlaybookRecord, error)) {
	if s.IsMock {
		s.later(1200*time.Millisecond, func() {
			content := mockDraftedSkill(kind)
			for _, skill := range s.Skills(scope) {
				if skill.Name == content.Name {
					content.Name += "-2"
				}
			}
			done(s.saveMockPlaybook(content, scope, nil, "draft"), nil)
		})
		return
	}
	params := map[string]any{"scope": scope, "bot_id": botID, "chat_id": chatID, "kind": kind, "message_ids": slices.Clone(messageIDs)}
	Async(s, func() (PlaybookRecord, error) { return call[PlaybookRecord](s, "playbooks.draft", params) }, done)
}

// ExportPlaybook is the skill as a portable file: its content and bundled files, nothing about the
// account, indented.
func (s *Store) ExportPlaybook(scope PlaybookScope, id string, done func([]byte, error)) {
	indent := func(data []byte) ([]byte, error) {
		var out bytes.Buffer
		err := json.Indent(&out, data, "", "  ")
		return out.Bytes(), err
	}
	if s.IsMock {
		s.Playbook(scope, id, func(record PlaybookRecord, err error) {
			if err != nil {
				done(nil, err)
				return
			}
			data, _ := json.Marshal(map[string]any{"format": "lorca-playbook", "version": 1, "content": record.Content})
			done(indent(data))
		})
		return
	}
	Async(s, func() ([]byte, error) {
		data, err := call[json.RawMessage](s, "playbooks.export", map[string]any{"scope": scope, "id": id})
		if err != nil {
			return nil, err
		}
		return indent(data)
	}, done)
}

func (s *Store) saveMockPlaybook(content PlaybookContent, scope PlaybookScope, over *PlaybookRecord, status string) PlaybookRecord {
	record := PlaybookRecord{ID: fmt.Sprintf("playbook-%d", time.Now().UnixNano()), Scope: scope}
	kind := "manual"
	if status == "draft" {
		kind = "workflow"
	}
	if over != nil {
		record, kind = *over, "edit"
		record.Revisions = slices.Clone(over.Revisions)
	}
	record.Revision++
	record.Status, record.Hash, record.Content = status, fmt.Sprintf("hash-%d", time.Now().UnixNano()), &content
	step := PlaybookRevision{ID: fmt.Sprintf("%s-%d", record.ID, record.Revision), Revision: record.Revision, Status: status, Content: &content, CreatedAt: float64(time.Now().Unix())}
	step.Provenance.Kind = kind
	if device := s.ThisDevice(); device != nil {
		step.DeviceID = device.ID
	}
	record.Revisions = append(record.Revisions, step)
	s.mockPlaybooks = append(slices.DeleteFunc(slices.Clone(s.mockPlaybooks), func(each PlaybookRecord) bool { return each.ID == record.ID }), record)
	s.mockPlaybooksChanged()
	return record
}

func (s *Store) mockPlaybooksChanged() {
	s.Playbooks = s.Playbooks[:0:0]
	for _, record := range s.mockPlaybooks {
		s.Playbooks = append(s.Playbooks, record.summary())
	}
	s.emit(Event{Kind: EventRosterChanged})
}

// CaptureSources are the messages Save as Skill or Save as Standing Instruction offer around the
// one clicked: up to it, the user's and the bot's for a workflow, the user's alone for
// corrections, the last twelve. The clicked message starts picked, and for a workflow the request
// before it. The CLI checks the picks again.
func CaptureSources(chat *Chat, clicked *Message) (botID, kind string, sources []*Message, picked map[string]bool) {
	if chat == nil || clicked == nil || !clicked.CanBeQuoted() || strings.TrimSpace(clicked.Body.Text) == "" {
		return
	}
	botID, kind = clicked.Author.BotID, "workflow"
	if clicked.Author.Kind == AuthorYou {
		botID, kind = chat.Owner(), "corrections"
		if botID == "" && len(chat.BotIDs) > 0 {
			botID = chat.BotIDs[0]
		}
	}
	if !slices.Contains(chat.BotIDs, botID) {
		return "", "", nil, nil
	}
	for _, m := range chat.Messages {
		if !m.CanBeQuoted() || strings.TrimSpace(m.Body.Text) == "" {
			continue
		}
		if m.Author.Kind == AuthorYou || (kind == "workflow" && m.Author.BotID == botID) {
			sources = append(sources, m)
		}
		if m.ID == clicked.ID {
			break
		}
	}
	if len(sources) > 12 {
		sources = sources[len(sources)-12:]
	}
	picked = map[string]bool{clicked.ID: true}
	if kind == "workflow" {
		for i := len(sources) - 1; i >= 0; i-- {
			if sources[i].Author.Kind == AuthorYou {
				picked[sources[i].ID] = true
				break
			}
		}
	}
	return
}
