package model

import (
	"errors"
	"maps"
	"slices"
	"strings"
)

// Portable template content is a CLI view, not an account/connection or schedule store. The CLI
// validates the file and its review digest; this model never reads or publishes template bytes.
type TemplateProfile struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	SymbolName  string `json:"symbol_name"`
	Accent      string `json:"accent"`
}

type TemplateResource struct {
	Path string `json:"path"`
	Text string `json:"text"`
}

type TemplateSkill struct {
	Name         string             `json:"name"`
	Description  string             `json:"description"`
	Instructions string             `json:"instructions"`
	Examples     string             `json:"examples"`
	References   []TemplateResource `json:"references"`
	Scripts      []TemplateResource `json:"scripts"`
}

type PortableRoutine struct {
	Name            string `json:"name"`
	Schedule        string `json:"schedule"`
	Prompt          string `json:"prompt"`
	Check           string `json:"check"`
	Timezone        string `json:"timezone"`
	MissedRunPolicy string `json:"missed_run_policy"`
}

type TemplateRequirement struct {
	ServiceID string `json:"service_id"`
}

type TemplateDocument struct {
	Format       string                `json:"format"`
	Version      int                   `json:"version"`
	Profile      *TemplateProfile      `json:"profile"`
	Skills       []TemplateSkill       `json:"skills"`
	Memories     []string              `json:"memories"`
	Routines     []PortableRoutine     `json:"routines"`
	Requirements []TemplateRequirement `json:"requirements"`
}

type TemplateItem[T any] struct {
	ID      string `json:"id"`
	Content T      `json:"content"`
}

type TemplateContents struct {
	Profile      TemplateProfile                 `json:"profile"`
	Skills       []TemplateItem[TemplateSkill]   `json:"skills"`
	Memories     []TemplateItem[string]          `json:"memories"`
	Routines     []TemplateItem[PortableRoutine] `json:"routines"`
	Requirements []TemplateRequirement           `json:"requirements"`
	Notes        []string                        `json:"notes"`
}

type TemplateSelection struct {
	Profile        bool     `json:"profile"`
	SkillIDs       []string `json:"skill_ids,omitempty"`
	MemoryIDs      []string `json:"memory_ids,omitempty"`
	RoutineIDs     []string `json:"routine_ids,omitempty"`
	RequirementIDs []string `json:"requirement_ids,omitempty"`
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

type TemplateWarning struct {
	Path    string `json:"path"`
	Message string `json:"message"`
}

type TemplateConnection struct {
	ID          string `json:"id"`
	Name        string `json:"name"`
	State       string `json:"state"`
	Detail      string `json:"detail"`
	ServiceID   string `json:"service_id"`
	AccountName string `json:"account_name"`
}

type TemplateMappingChoice struct {
	ServiceID  string               `json:"service_id"`
	Candidates []TemplateConnection `json:"candidates"`
	Selected   string               `json:"selected"`
}

type TemplatePreview struct {
	Template     *TemplateDocument       `json:"template"`
	Digest       string                  `json:"digest"`
	CanImport    bool                    `json:"can_import"`
	Issues       []string                `json:"issues"`
	Warnings     []TemplateWarning       `json:"warnings"`
	Requirements []TemplateMappingChoice `json:"requirements"`
	Visibility   string                  `json:"visibility"`
}

func (preview TemplatePreview) Name() string {
	if preview.Template != nil && preview.Template.Profile != nil {
		return preview.Template.Profile.Name
	}
	return ""
}

// Text renders every selected resource without JSON/schema details. CLI warnings/issues retain
// their original wording; headings follow the app's language on each build pass.
func (preview TemplatePreview) Text() string {
	var parts []string
	if len(preview.Warnings) > 0 {
		lines := []string{L("Review before sharing")}
		for _, warning := range preview.Warnings {
			lines = append(lines, "• "+warning.Message)
		}
		parts = append(parts, strings.Join(lines, "\n"))
	}
	if preview.Template == nil {
		return strings.Join(parts, "\n\n")
	}
	doc := preview.Template
	if profile := doc.Profile; profile != nil {
		parts = append(parts, strings.Join([]string{L("Profile"), profile.Name, profile.Description, L("Look: %@ · %@", profile.SymbolName, profile.Accent)}, "\n"))
	}
	for _, skill := range doc.Skills {
		lines := []string{L("Skill: %@", skill.Name), skill.Description, skill.Instructions}
		if skill.Examples != "" {
			lines = append(lines, L("Examples"), skill.Examples)
		}
		for _, resources := range [][]TemplateResource{skill.References, skill.Scripts} {
			for _, resource := range resources {
				lines = append(lines, resource.Path, resource.Text)
			}
		}
		parts = append(parts, strings.Join(lines, "\n"))
	}
	for _, memory := range doc.Memories {
		parts = append(parts, L("Memory")+"\n"+memory)
	}
	for _, routine := range doc.Routines {
		lines := []string{L("Routine: %@", routine.Name), routine.Schedule, routine.Prompt}
		if routine.Timezone != "" {
			lines = append(lines, L("Time zone: %@", routine.Timezone))
		}
		if routine.MissedRunPolicy != "" {
			lines = append(lines, L("Missed runs: %@", routine.MissedRunPolicy))
		}
		if routine.Check != "" {
			lines = append(lines, L("Check script"), routine.Check)
		}
		parts = append(parts, strings.Join(append(lines, L("Imported paused")), "\n"))
	}
	if len(doc.Requirements) > 0 {
		lines := []string{L("Integration requirements")}
		for _, requirement := range doc.Requirements {
			lines = append(lines, requirement.ServiceID)
		}
		parts = append(parts, strings.Join(lines, "\n"))
	}
	return strings.Join(parts, "\n\n")
}

type TemplateExportOptions struct {
	BotID          string            `json:"bot_id"`
	Selection      TemplateSelection `json:"selection"`
	Path           string            `json:"path"`
	ExpectedDigest string            `json:"expected_digest"`
	Reviewed       bool              `json:"reviewed"`
	Overwrite      bool              `json:"overwrite"`
}

type TemplateExportResult struct {
	Path       string `json:"path"`
	Digest     string `json:"digest"`
	Visibility string `json:"visibility"`
}

type TemplateImportOptions struct {
	Path           string            `json:"path"`
	RunnerID       string            `json:"runner_id"`
	Name           *string           `json:"name,omitempty"`
	Provider       ProviderKind      `json:"provider,omitempty"`
	Mappings       map[string]string `json:"mappings"`
	ExpectedDigest string            `json:"expected_digest,omitempty"`
	Reviewed       bool              `json:"reviewed"`
}

func (options TemplateImportOptions) clone() TemplateImportOptions {
	options.Mappings = maps.Clone(options.Mappings)
	if options.Mappings == nil {
		options.Mappings = map[string]string{}
	}
	if options.Name != nil {
		name := *options.Name
		options.Name = &name
	}
	return options
}

func (s *Store) TemplateContents(botID string, done func(TemplateContents, error)) {
	Async(s, func() (TemplateContents, error) {
		return call[TemplateContents](s, "templates.contents", map[string]any{"bot_id": botID})
	}, done)
}

func (s *Store) PreviewTemplateExport(botID string, selection TemplateSelection, done func(TemplatePreview, error)) {
	selection = selection.Clone()
	Async(s, func() (TemplatePreview, error) {
		return call[TemplatePreview](s, "templates.export.preview", struct {
			BotID     string            `json:"bot_id"`
			Selection TemplateSelection `json:"selection"`
		}{botID, selection})
	}, done)
}

func (s *Store) ExportTemplate(options TemplateExportOptions, done func(TemplateExportResult, error)) {
	options.Selection = options.Selection.Clone()
	Async(s, func() (TemplateExportResult, error) {
		return call[TemplateExportResult](s, "templates.export", options)
	}, done)
}

func (s *Store) PreviewTemplateImport(options TemplateImportOptions, done func(TemplatePreview, error)) {
	options = options.clone()
	Async(s, func() (TemplatePreview, error) {
		return call[TemplatePreview](s, "templates.import.preview", options)
	}, done)
}

// ImportTemplate reconciles only the new bot/DM returned by the CLI, on the ordered main-thread
// reply queue. It installs no plugins, copies no conversations, and creates no local routines.
func (s *Store) ImportTemplate(options TemplateImportOptions, done func(string, error)) {
	options = options.clone()
	type reply struct {
		Bot            WireBot `json:"bot"`
		ChatID         string  `json:"chat_id"`
		RoutinesPaused bool    `json:"routines_paused"`
	}
	Async(s, func() (reply, error) { return call[reply](s, "templates.import", options) }, func(result reply, err error) {
		if err != nil {
			done("", err)
			return
		}
		if result.Bot.ID == "" || result.ChatID == "" || !result.RoutinesPaused {
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
