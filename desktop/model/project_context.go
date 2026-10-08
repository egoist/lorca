package model

import (
	"errors"
)

// ProjectContext is a view of the CLI's encrypted group context. The desktop owns no project
// record: identity, membership, immutable revisions, verification and assets remain CLI-owned.
type ProjectContext struct {
	ChatID          string              `json:"chat_id"`
	Revision        string              `json:"revision"`
	Entries         []ProjectEntry      `json:"entries"`
	HasMore         bool                `json:"has_more"`
	Conflicts       map[string][]string `json:"conflicts"`
	MaxContextBytes int                 `json:"max_context_bytes"`
}

type ProjectOutputReference struct {
	ChatID    string `json:"chat_id"`
	MessageID string `json:"message_id"`
	OutputID  string `json:"output_id"`
	Version   uint32 `json:"version"`
	TaskID    string `json:"task_id,omitempty"`
}

type ProjectSource struct {
	Kind      string                  `json:"kind"`
	Label     string                  `json:"label"`
	URL       string                  `json:"url,omitempty"`
	MessageID string                  `json:"message_id,omitempty"`
	Output    *ProjectOutputReference `json:"output,omitempty"`
}

type ProjectEntry struct {
	ID             string          `json:"id"`
	Kind           string          `json:"kind"`
	Title          string          `json:"title"`
	Text           string          `json:"text"`
	Source         ProjectSource   `json:"source"`
	Verification   string          `json:"verification"`
	Freshness      string          `json:"freshness"`
	UpdatedAt      int64           `json:"updated_at"`
	VerifiedAt     *int64          `json:"verified_at,omitempty"`
	FetchedAt      *int64          `json:"fetched_at,omitempty"`
	MaxAgeSecs     *int64          `json:"max_age_secs,omitempty"`
	Supersedes     []string        `json:"supersedes,omitempty"`
	Current        bool            `json:"current"`
	Removed        bool            `json:"removed"`
	Asset          *WireAttachment `json:"asset,omitempty"`
	AssetAvailable bool            `json:"asset_available"`
	RefreshError   string          `json:"refresh_error,omitempty"`
}

type ProjectContextSave struct {
	Kind             string        `json:"kind"`
	Title            string        `json:"title"`
	Text             string        `json:"text"`
	Source           ProjectSource `json:"source"`
	Verification     string        `json:"verification"`
	MaxAgeSecs       *int64        `json:"max_age_secs,omitempty"`
	Supersedes       []string      `json:"supersedes,omitempty"`
	Removed          bool          `json:"removed"`
	ExpectedRevision string        `json:"expected_revision,omitempty"`
}

func (s *Store) projectGroup(chatID string) error {
	if chat := s.Chat(chatID); chat == nil || !chat.IsGroup() {
		return errors.New(L("Project context requires an existing group chat"))
	}
	return nil
}

// ProjectContext loads all pages, checking the group and revision throughout. Async returns its
// answer on the same ordered main-thread queue as the other store methods.
func (s *Store) ProjectContext(chatID string, history bool, done func(ProjectContext, error)) {
	if err := s.projectGroup(chatID); err != nil {
		s.post(func() { done(ProjectContext{}, err) })
		return
	}
	Async(s, func() (ProjectContext, error) {
		var context ProjectContext
		after := ""
		for {
			params := map[string]any{"chat_id": chatID, "history": history, "limit": 100}
			if after != "" {
				params["after"] = after
			}
			page, err := call[ProjectContext](s, "projects.get", params)
			if err != nil {
				return ProjectContext{}, err
			}
			if page.ChatID != chatID {
				return ProjectContext{}, errors.New(L("Project response belongs to another group"))
			}
			if context.Revision != "" && context.Revision != page.Revision {
				return ProjectContext{}, errors.New(L("Project context changed while loading. Reload to read the current revisions."))
			}
			context.ChatID, context.Revision, context.Conflicts, context.MaxContextBytes = page.ChatID, page.Revision, page.Conflicts, page.MaxContextBytes
			context.Entries = append(context.Entries, page.Entries...)
			if !page.HasMore {
				return context, nil
			}
			if len(page.Entries) == 0 || page.Entries[len(page.Entries)-1].ID <= after {
				return ProjectContext{}, errors.New(L("Project context page did not advance"))
			}
			after = page.Entries[len(page.Entries)-1].ID
		}
	}, done)
}

func (s *Store) SaveProjectContext(chatID string, input ProjectContextSave, done func(ProjectEntry, error)) {
	if err := s.projectGroup(chatID); err != nil {
		s.post(func() { done(ProjectEntry{}, err) })
		return
	}
	params := struct {
		ChatID string `json:"chat_id"`
		ProjectContextSave
	}{chatID, input}
	Async(s, func() (ProjectEntry, error) { return call[ProjectEntry](s, "projects.save", params) }, done)
}

func (s *Store) RefreshProjectContext(chatID, entryID string, done func(ProjectEntry, error)) {
	if err := s.projectGroup(chatID); err != nil {
		s.post(func() { done(ProjectEntry{}, err) })
		return
	}
	Async(s, func() (ProjectEntry, error) {
		return call[ProjectEntry](s, "projects.refresh", map[string]any{"chat_id": chatID, "entry_id": entryID})
	}, done)
}

func (s *Store) AddProjectAsset(chatID, path string, done func(ProjectEntry, error)) {
	if err := s.projectGroup(chatID); err != nil {
		s.post(func() { done(ProjectEntry{}, err) })
		return
	}
	Async(s, func() (ProjectEntry, error) {
		return call[ProjectEntry](s, "projects.asset", map[string]any{"chat_id": chatID, "file": map[string]any{"path": path}})
	}, done)
}

func (s *Store) ProjectAssetPath(chatID, entryID string, done func(string, error)) {
	if err := s.projectGroup(chatID); err != nil {
		s.post(func() { done("", err) })
		return
	}
	Async(s, func() (string, error) {
		reply, err := call[struct {
			Path string `json:"path"`
		}](s, "projects.asset_path", map[string]any{"chat_id": chatID, "entry_id": entryID})
		return reply.Path, err
	}, done)
}
