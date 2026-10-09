package model

import (
	"encoding/json"
	"slices"
)

// BotPermissions is a bot's Access, as the CLI keeps it in the encrypted roster: the plugins it
// may use and how, and what it may do on its Runner. A bot with no policy has full access.
type BotPermissions struct {
	// Connections nil is every plugin on the Runner, one installed later included; a map denies
	// the rest.
	Connections *map[string]ConnectionPermissions `json:"connections,omitempty"`
	Filesystem  AccessLevel                       `json:"filesystem"`
	Shell       bool                              `json:"shell"`
}

type ConnectionPermissions struct {
	Capabilities []string `json:"capabilities"`
	// Tools are original tool names; nil is every tool the plugin has, one added later included.
	Tools *[]string `json:"tools,omitempty"`
}

func (p *BotPermissions) UnmarshalJSON(data []byte) error {
	type plain BotPermissions
	value := plain{Filesystem: AccessWrite, Shell: true}
	if err := json.Unmarshal(data, &value); err != nil {
		return err
	}
	*p = BotPermissions(value)
	return nil
}

// FullAccess is the explicit policy of a bot with every plugin, files, and shell.
func FullAccess() *BotPermissions {
	return &BotPermissions{Filesystem: AccessWrite, Shell: true}
}

// Level is what the bot may do with a plugin: the most its grant reaches.
func (p *BotPermissions) Level(pluginID string) AccessLevel {
	if p == nil || p.Connections == nil {
		return AccessWrite
	}
	return LevelOf((*p.Connections)[pluginID].Capabilities)
}

// Summary is the Profile card's word for it.
func (p *BotPermissions) Summary() string {
	if p == nil || (p.Connections == nil && p.Filesystem == AccessWrite && p.Shell) {
		return L("Full access")
	}
	return L("Limited")
}

func (p *BotPermissions) Clone() *BotPermissions {
	if p == nil {
		return nil
	}
	data, _ := json.Marshal(p)
	var copy BotPermissions
	_ = json.Unmarshal(data, &copy)
	return &copy
}

func EqualBotPermissions(a, b *BotPermissions) bool {
	left, _ := json.Marshal(a)
	right, _ := json.Marshal(b)
	return string(left) == string(right)
}

// AccessLevel is how far a grant reaches, each level taking in the ones below it: a plugin's
// read, draft, and write capabilities, or files read or changed.
type AccessLevel string

const (
	AccessWrite AccessLevel = "write"
	AccessDraft AccessLevel = "draft"
	AccessRead  AccessLevel = "read"
	AccessNone  AccessLevel = "none"
)

// AccessLevels are the levels, the widest first, as the pop-ups list them.
var AccessLevels = []AccessLevel{AccessWrite, AccessDraft, AccessRead, AccessNone}

func LevelOf(capabilities []string) AccessLevel {
	switch {
	case slices.Contains(capabilities, "write"):
		return AccessWrite
	case slices.Contains(capabilities, "draft"):
		return AccessDraft
	case slices.Contains(capabilities, "read"):
		return AccessRead
	}
	return AccessNone
}

func (l AccessLevel) Capabilities() []string {
	switch l {
	case AccessWrite:
		return []string{"read", "draft", "write"}
	case AccessDraft:
		return []string{"read", "draft"}
	case AccessRead:
		return []string{"read"}
	}
	return []string{}
}

func (l AccessLevel) Allows(capability string) bool {
	return slices.Contains(l.Capabilities(), capability)
}

func (l AccessLevel) Title() string {
	switch l {
	case AccessWrite:
		return L("Read and write")
	case AccessDraft:
		return L("Read and draft")
	case AccessRead:
		return L("Read only")
	}
	return L("No access")
}

// AccessCatalog is the plugins on a bot's Runner and the tools each offered when it last
// connected, for the Access sheet.
type AccessCatalog struct {
	Connections []AccessPlugin `json:"connections"`
}

type AccessPlugin struct {
	ID    string       `json:"id"`
	Name  string       `json:"name"`
	Tools []AccessTool `json:"tools"`
}

type AccessTool struct {
	Name        string `json:"name"`
	Title       string `json:"title"`
	Description string `json:"description"`
	// Capability is what the tool does, as its Runner last read it: read, draft, or write.
	Capability string `json:"capability"`
}

// Shown is the tool's title, or its name when it has none.
func (t AccessTool) Shown() string {
	if t.Title != "" {
		return t.Title
	}
	return t.Name
}

// BotAccessCatalog asks the bot's Runner for its plugins and their tools.
func (s *Store) BotAccessCatalog(id string, done func(AccessCatalog, error)) {
	if s.IsMock {
		catalog := MockAccessCatalog()
		s.post(func() { done(catalog, nil) })
		return
	}
	Async(s, func() (AccessCatalog, error) {
		return call[AccessCatalog](s, "bots.permissions", map[string]any{"id": id})
	}, done)
}

// SetBotPermissions is only ever the user's: the CLI also dismisses the access requests the bot
// left.
func (s *Store) SetBotPermissions(id string, policy *BotPermissions) {
	bot := s.Bot(id)
	if bot == nil || policy == nil {
		return
	}
	bot.Permissions = policy.Clone()
	s.rosterTouched(id)
	s.perform("bots.update", map[string]any{"id": id, "permissions": policy})
}

// MockAccessCatalog is what Workbench's plugins offered when they last connected.
func MockAccessCatalog() AccessCatalog {
	catalog := AccessCatalog{Connections: []AccessPlugin{
		{ID: "github", Name: "GitHub", Tools: []AccessTool{
			{Name: "search_issues", Title: "Search issues", Capability: "read"},
			{Name: "get_pull_request", Title: "Get a pull request", Capability: "read"},
			{Name: "create_pull_request_review", Title: "Draft a review", Capability: "draft"},
			{Name: "create_issue", Title: "Create an issue", Capability: "write"},
			{Name: "merge_pull_request", Title: "Merge a pull request", Capability: "write"},
		}},
		{ID: "linear", Name: "Linear", Tools: []AccessTool{
			{Name: "list_issues", Title: "List issues", Capability: "read"},
			{Name: "create_issue", Title: "Create an issue", Capability: "write"},
		}},
	}}
	for _, server := range mockMcpServers() {
		if !server.Enabled {
			continue
		}
		plugin := AccessPlugin{ID: server.ID, Name: server.Name}
		for _, tool := range server.Tools {
			capability := "write"
			if tool.ReadOnly {
				capability = "read"
			}
			plugin.Tools = append(plugin.Tools, AccessTool{Name: tool.Name, Description: tool.Description, Capability: capability})
		}
		catalog.Connections = append(catalog.Connections, plugin)
	}
	return catalog
}

// mockWriterAccess lets the Writer read GitHub and its notes folder, draft reviews, and run no
// commands.
func mockWriterAccess() *BotPermissions {
	tools := []string{"create_pull_request_review", "get_pull_request", "search_issues"}
	connections := map[string]ConnectionPermissions{
		"github":     {Capabilities: AccessDraft.Capabilities(), Tools: &tools},
		"filesystem": {Capabilities: AccessWrite.Capabilities()},
		"deepwiki":   {Capabilities: AccessRead.Capabilities()},
	}
	return &BotPermissions{Connections: &connections, Filesystem: AccessWrite, Shell: false}
}
