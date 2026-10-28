package model

import (
	"errors"
	"maps"
	"slices"
)

// Bot templates, after the macOS app's TemplateSharing: the CLI reads and writes the files and
// checks them, and answers with the contents as the file holds them.

type TemplateProfile struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	SymbolName  string `json:"symbol_name"`
	Accent      string `json:"accent"`
}

type TemplateSkill struct {
	Name        string `json:"name"`
	Description string `json:"description"`
}

type PortableRoutine struct {
	Name     string `json:"name"`
	Schedule string `json:"schedule"`
	Prompt   string `json:"prompt"`
	// ScheduleText is the schedule in words, in an import preview.
	ScheduleText string `json:"schedule_text"`
}

type TemplateDocument struct {
	Profile  *TemplateProfile  `json:"profile"`
	Skills   []TemplateSkill   `json:"skills"`
	Memories []string          `json:"memories"`
	Routines []PortableRoutine `json:"routines"`
}

// TemplateItem is a piece of a bot as the export offers it, with what a reader should look at
// before sharing it: "email", "phone", "path", "link", or "credential" (a key it redacted).
type TemplateItem[T any] struct {
	ID      string   `json:"id"`
	Content T        `json:"content"`
	Flags   []string `json:"flags"`
}

// TemplateService is a plugin the bot's Runner has, offered as a requirement.
type TemplateService struct {
	ServiceID string `json:"service_id"`
	Name      string `json:"name"`
}

type TemplateContents struct {
	Profile      TemplateItem[TemplateProfile]   `json:"profile"`
	Skills       []TemplateItem[TemplateSkill]   `json:"skills"`
	Memories     []TemplateItem[string]          `json:"memories"`
	Routines     []TemplateItem[PortableRoutine] `json:"routines"`
	Requirements []TemplateService               `json:"requirements"`
}

// TemplateSelection is what goes in a template besides the profile, by the ids TemplateContents
// gave. A list left out is none.
type TemplateSelection struct {
	Profile        bool     `json:"profile"`
	SkillIDs       []string `json:"skill_ids,omitempty"`
	MemoryIDs      []string `json:"memory_ids,omitempty"`
	RoutineIDs     []string `json:"routine_ids,omitempty"`
	RequirementIDs []string `json:"requirement_ids,omitempty"`
}

// SharedLink is a bot the account shares as a link. The roster carries it to every Device, which
// lists, updates, and revokes it.
type SharedLink struct {
	ID string `json:"id"`
	// URL is the whole address, the key in its fragment.
	URL   string `json:"url"`
	BotID string `json:"bot_id"`
	// Name is the bot's name when it was last shared.
	Name      string            `json:"name"`
	Selection TemplateSelection `json:"selection"`
	UpdatedAt float64           `json:"updated_at"`
}

// SharedLinkFor is the link the bot was last shared as.
func (s *Store) SharedLinkFor(botID string) *SharedLink {
	for i := len(s.SharedLinks) - 1; i >= 0; i-- {
		if s.SharedLinks[i].BotID == botID {
			return &s.SharedLinks[i]
		}
	}
	return nil
}

// TemplateShareOptions is a reviewed template to share as a link, or to update LinkID's with.
type TemplateShareOptions struct {
	BotID          string            `json:"bot_id"`
	Selection      TemplateSelection `json:"selection"`
	ExpectedDigest string            `json:"expected_digest"`
	Reviewed       bool              `json:"reviewed"`
	LinkID         string            `json:"link_id,omitempty"`
}

// ShareTemplate puts the template on the relay behind a link and answers it.
func (s *Store) ShareTemplate(options TemplateShareOptions, done func(SharedLink, error)) {
	options.Selection = options.Selection.Clone()
	type reply struct {
		Link SharedLink `json:"link"`
	}
	Async(s, func() (reply, error) { return call[reply](s, "templates.share", options) }, func(result reply, err error) {
		done(result.Link, err)
	})
}

// RevokeLink takes the link down: whoever opens it sees that it no longer works.
func (s *Store) RevokeLink(id string, done func(error)) {
	Async(s, func() (struct{}, error) {
		_, err := call[struct{}](s, "templates.unshare", map[string]any{"link_id": id})
		return struct{}{}, err
	}, func(_ struct{}, err error) { done(err) })
}

func (selection TemplateSelection) Clone() TemplateSelection {
	selection.SkillIDs = slices.Clone(selection.SkillIDs)
	selection.MemoryIDs = slices.Clone(selection.MemoryIDs)
	selection.RoutineIDs = slices.Clone(selection.RoutineIDs)
	selection.RequirementIDs = slices.Clone(selection.RequirementIDs)
	return selection
}

func (selection TemplateSelection) Empty() bool {
	return !selection.Profile && len(selection.SkillIDs)+len(selection.MemoryIDs)+len(selection.RoutineIDs)+len(selection.RequirementIDs) == 0
}

// TemplateConnection is one of a Runner's own connections for a plugin a template uses.
type TemplateConnection struct {
	ID    string `json:"id"`
	Name  string `json:"name"`
	State string `json:"state"`
}

// TemplatePlugin is a plugin a template uses, the Runner's connections for it, and the one picked:
// the user's choice, else the only ready one.
type TemplatePlugin struct {
	ServiceID  string               `json:"service_id"`
	Name       string               `json:"name"`
	Candidates []TemplateConnection `json:"candidates"`
	Selected   string               `json:"selected"`
}

// Ready is whether the picked connection can be used now.
func (plugin TemplatePlugin) Ready() bool {
	return slices.ContainsFunc(plugin.Candidates, func(c TemplateConnection) bool { return c.ID == plugin.Selected && c.State == "ready" })
}

// TemplatePreview is what the CLI built for an export, or read from a file for an import.
type TemplatePreview struct {
	Template     *TemplateDocument `json:"template"`
	Digest       string            `json:"digest"`
	CanImport    bool              `json:"can_import"`
	Issues       []string          `json:"issues"`
	Requirements []TemplatePlugin  `json:"requirements"`
}

type TemplateExportOptions struct {
	BotID          string            `json:"bot_id"`
	Selection      TemplateSelection `json:"selection"`
	Path           string            `json:"path"`
	ExpectedDigest string            `json:"expected_digest"`
	Reviewed       bool              `json:"reviewed"`
	Overwrite      bool              `json:"overwrite"`
}

type TemplateImportOptions struct {
	Path           string            `json:"path,omitempty"`
	Link           string            `json:"link,omitempty"`
	RunnerID       string            `json:"runner_id,omitempty"`
	Name           string            `json:"name,omitempty"`
	Provider       ProviderKind      `json:"provider,omitempty"`
	Mappings       map[string]string `json:"mappings"`
	ExpectedDigest string            `json:"expected_digest,omitempty"`
	Reviewed       bool              `json:"reviewed,omitempty"`
}

func (s *Store) TemplateContents(botID string, done func(TemplateContents, error)) {
	Async(s, func() (TemplateContents, error) {
		return call[TemplateContents](s, "templates.contents", map[string]any{"bot_id": botID})
	}, done)
}

func (s *Store) PreviewTemplateExport(botID string, selection TemplateSelection, done func(TemplatePreview, error)) {
	selection = selection.Clone()
	Async(s, func() (TemplatePreview, error) {
		return call[TemplatePreview](s, "templates.export.preview", map[string]any{"bot_id": botID, "selection": selection})
	}, done)
}

func (s *Store) ExportTemplate(options TemplateExportOptions, done func(error)) {
	options.Selection = options.Selection.Clone()
	Async(s, func() (struct{}, error) {
		_, err := call[struct{}](s, "templates.export", options)
		return struct{}{}, err
	}, func(_ struct{}, err error) { done(err) })
}

func (s *Store) PreviewTemplateImport(options TemplateImportOptions, done func(TemplatePreview, error)) {
	options.Mappings = maps.Clone(options.Mappings)
	Async(s, func() (TemplatePreview, error) {
		return call[TemplatePreview](s, "templates.import.preview", options)
	}, done)
}

// ImportTemplate adds the bot the CLI made and its direct chat, ahead of the roster that brings
// them from another Runner, and answers the chat.
func (s *Store) ImportTemplate(options TemplateImportOptions, done func(string, error)) {
	options.Mappings = maps.Clone(options.Mappings)
	type reply struct {
		Bot    WireBot `json:"bot"`
		ChatID string  `json:"chat_id"`
	}
	Async(s, func() (reply, error) { return call[reply](s, "templates.import", options) }, func(result reply, err error) {
		if err != nil {
			done("", err)
			return
		}
		if result.Bot.ID == "" || result.ChatID == "" {
			done("", errors.New(L("Couldn't read the imported bot.")))
			return
		}
		bot := ToBot(result.Bot)
		if s.Bot(bot.ID) == nil {
			s.Bots = append(s.Bots, bot)
		}
		if s.Chat(result.ChatID) == nil {
			s.Chats = append(s.Chats, &Chat{ID: result.ChatID, Kind: ChatDM, BotIDs: []string{bot.ID}, CreatedAt: bot.CreatedAt})
		}
		s.sortChats()
		s.emit(Event{Kind: EventRosterChanged})
		s.emit(Event{Kind: EventChatsChanged})
		done(result.ChatID, nil)
	})
}
