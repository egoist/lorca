package model

import (
	"net/url"
	"slices"
	"strings"
	"time"
)

// A group's shared project context, after the macOS app's ProjectContext.swift: the briefs, goals,
// constraints, decisions, facts, links, and files every bot in the group can read. The CLI owns
// scope, immutable revisions, source checks, and sync; the app lists the current entries and sends
// the user's changes.

// ProjectKinds are the kinds in the order the inspector lists them; the CLI calls a link
// `document` and a file `asset`.
var ProjectKinds = []string{"brief", "goal", "constraint", "decision", "fact", "document", "asset"}

// ProjectKindTitle is the words a kind goes by.
func ProjectKindTitle(kind string) string {
	switch kind {
	case "brief":
		return L("Brief")
	case "goal":
		return L("Goal")
	case "constraint":
		return L("Constraint")
	case "decision":
		return L("Decision")
	case "fact":
		return L("Fact")
	case "document":
		return L("Link")
	}
	return L("File")
}

// ProjectKindNewTitle is a new entry's sheet title.
func ProjectKindNewTitle(kind string) string {
	switch kind {
	case "brief":
		return L("New Brief")
	case "goal":
		return L("New Goal")
	case "constraint":
		return L("New Constraint")
	case "decision":
		return L("New Decision")
	case "fact":
		return L("New Fact")
	case "document":
		return L("New Link")
	}
	return L("New File")
}

// ProjectKindSymbol is the SF Symbol name a kind's row shows.
func ProjectKindSymbol(kind string) string {
	switch kind {
	case "brief":
		return "doc.text"
	case "goal":
		return "flag"
	case "constraint":
		return "hand.raised"
	case "decision":
		return "checkmark.seal"
	case "fact":
		return "info.circle"
	case "document":
		return "link"
	}
	return "paperclip"
}

// ProjectOutput names another subject's immutable output version an entry cites.
type ProjectOutput struct {
	ChatID    string `json:"chat_id"`
	MessageID string `json:"message_id"`
	OutputID  string `json:"output_id"`
	Version   uint32 `json:"version"`
}

// ProjectSource is where an entry came from, kept whole so a correction carries a message or
// output reference along unchanged.
type ProjectSource struct {
	Kind      string         `json:"kind"`
	Label     string         `json:"label"`
	URL       string         `json:"url,omitempty"`
	MessageID string         `json:"message_id,omitempty"`
	Output    *ProjectOutput `json:"output,omitempty"`
}

// ProjectEntry is one entry as `projects.get` lists it.
type ProjectEntry struct {
	ID           string          `json:"id"`
	Kind         string          `json:"kind"`
	Title        string          `json:"title"`
	Text         string          `json:"text"`
	Source       ProjectSource   `json:"source"`
	Verification string          `json:"verification"`
	Freshness    string          `json:"freshness"`
	UpdatedAt    int64           `json:"updated_at"`
	VerifiedAt   *int64          `json:"verified_at,omitempty"`
	FetchedAt    *int64          `json:"fetched_at,omitempty"`
	Asset        *WireAttachment `json:"asset,omitempty"`
	RefreshError string          `json:"refresh_error,omitempty"`
	Supersedes   []string        `json:"supersedes,omitempty"`
	Current      bool            `json:"current"`
	Removed      bool            `json:"removed"`
}

// Updated is when this version was saved.
func (e *ProjectEntry) Updated() time.Time { return time.Unix(e.UpdatedAt, 0) }

// IsSuggestion is a bot's proposal, waiting for the user to accept it.
func (e *ProjectEntry) IsSuggestion() bool {
	return e.Verification == "unverified" && e.Source.Kind == "bot"
}

// Checked is the last time the link was read or confirmed.
func (e *ProjectEntry) Checked() (time.Time, bool) {
	var latest int64
	for _, at := range []*int64{e.FetchedAt, e.VerifiedAt} {
		if at != nil && *at > latest {
			latest = *at
		}
	}
	return time.Unix(latest, 0), latest > 0
}

// CanCheckLink: a bot reads the link again before it relies on it; an agreed decision is the
// user's to change, so only its own source is checked.
func (e *ProjectEntry) CanCheckLink() bool {
	return e.Source.URL != "" && !(e.Kind == "decision" && e.Verification == "agreed")
}

// Host is the host of the entry's link, for its row.
func (e *ProjectEntry) Host() string {
	parsed, err := url.Parse(e.Source.URL)
	if e.Source.URL == "" || err != nil {
		return ""
	}
	return strings.TrimPrefix(parsed.Hostname(), "www.")
}

// ProjectDay is "Oct 8", or "10月8日".
func ProjectDay(t time.Time) string { return monthDay(t.Local()) }

// ProjectContext is a group's current entries, in the order the inspector lists them, and the
// entries that are two versions of one (two Devices changed it at once).
type ProjectContext struct {
	Entries   []ProjectEntry
	Conflicts [][]string
}

// OtherVersions are the other current versions of the entry, when two Devices changed it at once.
func (p ProjectContext) OtherVersions(id string) []string {
	for _, versions := range p.Conflicts {
		if slices.Contains(versions, id) {
			return slices.DeleteFunc(slices.Clone(versions), func(other string) bool { return other == id })
		}
	}
	return nil
}

// OrderProjectEntries lists briefs first and links and files last, newest first within a kind.
func OrderProjectEntries(entries []ProjectEntry) {
	slices.SortStableFunc(entries, func(a, b ProjectEntry) int {
		if ka, kb := slices.Index(ProjectKinds, a.Kind), slices.Index(ProjectKinds, b.Kind); ka != kb {
			return ka - kb
		}
		if a.UpdatedAt != b.UpdatedAt {
			if a.UpdatedAt > b.UpdatedAt {
				return -1
			}
			return 1
		}
		return strings.Compare(a.ID, b.ID)
	})
}

// StaleProjectEntry is what the CLI answers when the entry being saved changed on another Device
// first.
const StaleProjectEntry = "This entry changed on another Device."

type projectPage struct {
	Entries   []ProjectEntry      `json:"entries"`
	HasMore   bool                `json:"has_more"`
	Conflicts map[string][]string `json:"conflicts"`
}

// projectEntries reads every page of the group's entries, or of its whole history.
func (s *Store) projectEntries(chatID string, history bool) ([]ProjectEntry, [][]string, error) {
	var entries []ProjectEntry
	var conflicts [][]string
	after := ""
	for {
		params := map[string]any{"chat_id": chatID, "history": history, "limit": 100}
		if after != "" {
			params["after"] = after
		}
		page, err := call[projectPage](s, "projects.get", params)
		if err != nil {
			return nil, nil, err
		}
		entries = append(entries, page.Entries...)
		conflicts = conflicts[:0]
		for _, versions := range page.Conflicts {
			conflicts = append(conflicts, versions)
		}
		if !page.HasMore || len(page.Entries) == 0 {
			return entries, conflicts, nil
		}
		after = page.Entries[len(page.Entries)-1].ID
	}
}

// ProjectContext asks for the group's current entries.
func (s *Store) ProjectContext(chatID string, done func(ProjectContext, error)) {
	if s.IsMock {
		context := mockProjectContext(chatID)
		s.post(func() { done(context, nil) })
		return
	}
	Async(s, func() (ProjectContext, error) {
		entries, conflicts, err := s.projectEntries(chatID, false)
		OrderProjectEntries(entries)
		return ProjectContext{Entries: entries, Conflicts: conflicts}, err
	}, done)
}

// ProjectSave is an entry as the user wrote it: what it replaces (and any other versions of that),
// and a link the user typed, which becomes its source.
type ProjectSave struct {
	Kind, Title, Text, Link string
	Replacing               *ProjectEntry
	AlsoReplacing           []string
}

// SaveProjectEntry adds the entry, or saves a new version of the one it replaces. What the user
// writes is agreed.
func (s *Store) SaveProjectEntry(chatID string, save ProjectSave, done func(error)) {
	if s.IsMock {
		s.post(func() { done(nil) })
		return
	}
	params := map[string]any{"chat_id": chatID, "kind": save.Kind, "title": save.Title, "text": save.Text, "verification": "agreed"}
	if base := save.Replacing; base != nil {
		params["supersedes"] = append([]string{base.ID}, save.AlsoReplacing...)
	}
	switch {
	case save.Replacing != nil && save.Replacing.Source.URL == save.Link:
		params["source"] = save.Replacing.Source
	case save.Link != "":
		label := save.Link
		if parsed, err := url.Parse(save.Link); err == nil && parsed.Hostname() != "" {
			label = parsed.Hostname()
		}
		params["source"] = ProjectSource{Kind: "url", Label: label, URL: save.Link}
	}
	Async(s, func() (struct{}, error) {
		_, err := call[ProjectEntry](s, "projects.save", params)
		return struct{}{}, err
	}, func(_ struct{}, err error) { done(err) })
}

// RemoveProjectEntry takes the entry out of the group's context; its history stays.
func (s *Store) RemoveProjectEntry(chatID string, entry ProjectEntry, done func(error)) {
	if s.IsMock {
		s.post(func() { done(nil) })
		return
	}
	params := map[string]any{"chat_id": chatID, "kind": entry.Kind, "title": entry.Title, "source": entry.Source, "supersedes": []string{entry.ID}, "removed": true}
	Async(s, func() (struct{}, error) {
		_, err := call[ProjectEntry](s, "projects.save", params)
		return struct{}{}, err
	}, func(_ struct{}, err error) { done(err) })
}

// CurrentProjectEntry is what became of an entry another Device changed while it was open here: the
// current version it led to, or false when it was removed.
func (s *Store) CurrentProjectEntry(chatID, id string, done func(ProjectEntry, bool, error)) {
	if s.IsMock {
		s.post(func() { done(ProjectEntry{}, false, nil) })
		return
	}
	type found struct {
		entry ProjectEntry
		ok    bool
	}
	Async(s, func() (found, error) {
		all, _, err := s.projectEntries(chatID, true)
		if err != nil {
			return found{}, err
		}
		frontier, seen := []string{id}, map[string]bool{id: true}
		for len(frontier) > 0 {
			older := frontier[len(frontier)-1]
			frontier = frontier[:len(frontier)-1]
			for _, entry := range all {
				if !slices.Contains(entry.Supersedes, older) || seen[entry.ID] {
					continue
				}
				seen[entry.ID] = true
				if entry.Current && !entry.Removed {
					return found{entry, true}, nil
				}
				frontier = append(frontier, entry.ID)
			}
		}
		return found{}, nil
	}, func(result found, err error) { done(result.entry, result.ok, err) })
}

// CheckProjectLink reads the entry's link again; the answer is its new version, read or not.
func (s *Store) CheckProjectLink(chatID, entryID string, done func(ProjectEntry, error)) {
	if s.IsMock {
		s.post(func() { done(ProjectEntry{}, nil) })
		return
	}
	Async(s, func() (ProjectEntry, error) {
		return call[ProjectEntry](s, "projects.refresh", map[string]any{"chat_id": chatID, "entry_id": entryID})
	}, done)
}

// AddProjectFile stores and encrypts a file for the group.
func (s *Store) AddProjectFile(chatID, path string, done func(error)) {
	if s.IsMock {
		s.post(func() { done(nil) })
		return
	}
	Async(s, func() (struct{}, error) {
		_, err := call[ProjectEntry](s, "projects.asset", map[string]any{"chat_id": chatID, "file": map[string]any{"path": path}})
		return struct{}{}, err
	}, func(_ struct{}, err error) { done(err) })
}

// ProjectFile is the file's path on this Device, fetched first when another Device added it.
func (s *Store) ProjectFile(chatID, entryID string, done func(string, error)) {
	Async(s, func() (string, error) {
		reply, err := call[struct {
			Path string `json:"path"`
		}](s, "projects.asset_path", map[string]any{"chat_id": chatID, "entry_id": entryID})
		return reply.Path, err
	}, done)
}

// mockProjectContext is the demo's Launch room context, as the macOS app's mock shows it.
func mockProjectContext(chatID string) ProjectContext {
	if chatID != "chat-relay" {
		return ProjectContext{}
	}
	entry := func(id, kind, title, text string, source ProjectSource, verification string, hoursAgo int64, asset *WireAttachment) ProjectEntry {
		at := Now().Unix() - hoursAgo*3600
		e := ProjectEntry{ID: id, Kind: kind, Title: title, Text: text, Source: source, Verification: verification, Freshness: verification, UpdatedAt: at, Asset: asset, Current: true}
		if verification == "agreed" {
			e.VerifiedAt = &at
		}
		if verification == "fetched" {
			e.FetchedAt = &at
		}
		return e
	}
	you := ProjectSource{Kind: "user", Label: "User"}
	entries := []ProjectEntry{
		entry("ctx-brief", "brief", "Relay launch", "Move every Device to the TLS relay and announce it on Friday. The release notes go out with the build.", you, "agreed", 50, nil),
		entry("ctx-decision", "decision", "Go/no-go on Friday at 10:00", "Nova makes the call after the smoke test passes on Mac, Linux, and the phone.", you, "agreed", 26, nil),
		entry("ctx-constraint", "constraint", "No downtime for paired Devices", "Old relays keep answering until every Device has moved.", you, "agreed", 30, nil),
		entry("ctx-fact", "fact", "Relay p95 latency is 180 ms", "Measured on the staging relay last week.", ProjectSource{Kind: "bot", Label: "Scout"}, "unverified", 3, nil),
		entry("ctx-link", "document", "Release checklist", "", ProjectSource{Kind: "url", Label: "docs.example.com", URL: "https://docs.example.com/relay/checklist"}, "fetched", 5, nil),
		entry("ctx-file", "asset", "Launch announcement", "Writer's final draft.", you, "agreed", 8, &WireAttachment{ID: "att-announcement", Name: "announcement.pdf", Mime: "application/pdf", Size: 248_000}),
	}
	OrderProjectEntries(entries)
	return ProjectContext{Entries: entries}
}
