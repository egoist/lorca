package model

import (
	"encoding/json"
	"errors"
	"slices"
	"sort"
	"strings"
)

// Pointer allowlists distinguish full access (absent) from an explicit empty deny list.
type BotPermissions struct {
	Connections *map[string]ConnectionPermissions `json:"connections,omitempty"`
	Tools       *[]string                         `json:"tools,omitempty"`
	Filesystem  string                            `json:"filesystem"`
	Shell       bool                              `json:"shell"`
}

type ConnectionPermissions struct {
	Capabilities []string  `json:"capabilities"`
	Tools        *[]string `json:"tools,omitempty"`
}

func (p *BotPermissions) UnmarshalJSON(data []byte) error {
	type plain BotPermissions
	value := plain{Filesystem: "write", Shell: true}
	if err := json.Unmarshal(data, &value); err != nil {
		return err
	}
	*p = BotPermissions(value)
	return nil
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

func (p *BotPermissions) Summary() string {
	connections, tools, shell := L("All connections"), L("All local tools"), L("Shell allowed")
	if p != nil {
		if p.Connections != nil {
			count := 0
			for _, grant := range *p.Connections {
				if len(grant.Capabilities) > 0 {
					count++
				}
			}
			connections = L("%d connections", count)
		}
		if p.Tools != nil {
			tools = L("%d local tools", len(*p.Tools))
		}
		if !p.Shell {
			shell = L("Shell denied")
		}
	}
	return strings.Join([]string{connections, tools, shell}, " · ")
}

type PermissionTool struct {
	Name        string `json:"name"`
	Description string `json:"description"`
	Capability  string `json:"capability"`
	Hidden      bool   `json:"hidden"`
}

type PermissionConnection struct {
	ID    string           `json:"id"`
	Name  string           `json:"name"`
	Tools []PermissionTool `json:"tools"`
}

type BotPermissionCatalog struct {
	LocalTools  []string               `json:"local_tools"`
	Connections []PermissionConnection `json:"connections"`
}

// AccessDraft owns editable values across native build passes and catalog replies.
type AccessDraft struct {
	AllConnections, AllTools bool
	Filesystem               string
	Shell                    bool
	Connections              map[string]*AccessConnectionDraft
	Tools                    map[string]*AccessToolDraft
}

type AccessToolDraft struct {
	Name, Description, Capability string
	Selected, Hidden              bool
}

type AccessConnectionDraft struct {
	ID, Name           string
	Read, Draft, Write bool
	AllTools           bool
	Tools              map[string]*AccessToolDraft
}

func NewAccessDraft(policy *BotPermissions) *AccessDraft {
	d := &AccessDraft{AllConnections: true, AllTools: true, Filesystem: "write", Shell: true,
		Connections: map[string]*AccessConnectionDraft{}, Tools: map[string]*AccessToolDraft{}}
	if policy == nil {
		return d
	}
	d.Filesystem, d.Shell = policy.Filesystem, policy.Shell
	if policy.Tools != nil {
		d.AllTools = false
		for _, name := range *policy.Tools {
			d.Tools[name] = &AccessToolDraft{Name: name, Selected: true}
		}
	}
	if policy.Connections != nil {
		d.AllConnections = false
		for id, grant := range *policy.Connections {
			c := &AccessConnectionDraft{ID: id, Name: id, AllTools: grant.Tools == nil, Tools: map[string]*AccessToolDraft{},
				Read: slices.Contains(grant.Capabilities, "read"), Draft: slices.Contains(grant.Capabilities, "draft"), Write: slices.Contains(grant.Capabilities, "write")}
			if grant.Tools != nil {
				for _, name := range *grant.Tools {
					c.Tools[name] = &AccessToolDraft{Name: name, Selected: true}
				}
			}
			d.Connections[id] = c
		}
	}
	return d
}

// MergeCatalog preserves edited fields, saved grants for unavailable instances/tools, and
// unknown exact local names. New instances are denied once the user chooses an allowlist.
func (d *AccessDraft) MergeCatalog(catalog BotPermissionCatalog) {
	for _, name := range catalog.LocalTools {
		if d.Tools[name] == nil {
			d.Tools[name] = &AccessToolDraft{Name: name, Selected: d.AllTools}
		}
	}
	for _, connection := range catalog.Connections {
		c := d.Connections[connection.ID]
		if c == nil {
			c = &AccessConnectionDraft{ID: connection.ID, AllTools: true, Read: d.AllConnections, Draft: d.AllConnections,
				Write: d.AllConnections, Tools: map[string]*AccessToolDraft{}}
			d.Connections[connection.ID] = c
		}
		c.Name = connection.Name
		if c.Name == "" {
			c.Name = c.ID
		}
		for _, tool := range connection.Tools {
			t := c.Tools[tool.Name]
			if t == nil {
				t = &AccessToolDraft{Name: tool.Name, Selected: c.AllTools}
				c.Tools[tool.Name] = t
			}
			t.Description, t.Capability, t.Hidden = tool.Description, tool.Capability, tool.Hidden
			if t.Capability == "" {
				t.Capability = "write"
			}
		}
	}
}

func sortedAccessTools(tools map[string]*AccessToolDraft) []*AccessToolDraft {
	result := make([]*AccessToolDraft, 0, len(tools))
	for _, tool := range tools {
		if !tool.Hidden || tool.Selected {
			result = append(result, tool)
		}
	}
	slices.SortFunc(result, func(a, b *AccessToolDraft) int { return strings.Compare(a.Name, b.Name) })
	return result
}
func (d *AccessDraft) LocalTools() []*AccessToolDraft             { return sortedAccessTools(d.Tools) }
func (c *AccessConnectionDraft) OfferedTools() []*AccessToolDraft { return sortedAccessTools(c.Tools) }
func (d *AccessDraft) Instances() []*AccessConnectionDraft {
	result := make([]*AccessConnectionDraft, 0, len(d.Connections))
	for _, c := range d.Connections {
		result = append(result, c)
	}
	slices.SortFunc(result, func(a, b *AccessConnectionDraft) int {
		if name := strings.Compare(a.Name, b.Name); name != 0 {
			return name
		}
		return strings.Compare(a.ID, b.ID)
	})
	return result
}

func selectedAccessTools(tools map[string]*AccessToolDraft) *[]string {
	names := []string{}
	for name, tool := range tools {
		if tool.Selected {
			names = append(names, name)
		}
	}
	sort.Strings(names)
	return &names
}
func (d *AccessDraft) Policy() *BotPermissions {
	p := &BotPermissions{Filesystem: d.Filesystem, Shell: d.Shell}
	if !d.AllTools {
		p.Tools = selectedAccessTools(d.Tools)
	}
	if !d.AllConnections {
		connections := map[string]ConnectionPermissions{}
		for id, c := range d.Connections {
			grant := ConnectionPermissions{Capabilities: []string{}}
			if c.Read {
				grant.Capabilities = append(grant.Capabilities, "read")
			}
			if c.Draft {
				grant.Capabilities = append(grant.Capabilities, "draft")
			}
			if c.Write {
				grant.Capabilities = append(grant.Capabilities, "write")
			}
			if !c.AllTools {
				grant.Tools = selectedAccessTools(c.Tools)
			}
			connections[id] = grant
		}
		p.Connections = &connections
	}
	return p
}

func (s *Store) BotPermissionCatalog(id string, done func(BotPermissionCatalog, error)) {
	if s.IsMock {
		catalog := MockBotPermissionCatalog()
		s.post(func() { done(catalog, nil) })
		return
	}
	Async(s, func() (BotPermissionCatalog, error) {
		return call[BotPermissionCatalog](s, "bots.permissions", map[string]any{"id": id})
	}, done)
}

// SetBotPermissions confirms through the CLI and updates only the policy. A later roster
// event wins over an older save response; all callbacks land on the ordered main-thread queue.
func (s *Store) SetBotPermissions(id string, policy *BotPermissions, done func(error)) {
	bot := s.Bot(id)
	if bot == nil {
		if done != nil {
			s.post(func() { done(errors.New(L("Unknown bot"))) })
		}
		return
	}
	prior, desired := bot.Permissions.Clone(), policy.Clone()
	if desired == nil {
		if done != nil {
			s.post(func() { done(errors.New(L("Use an explicit policy to change access."))) })
		}
		return
	}
	finish := func(saved *BotPermissions, err error) {
		if err == nil {
			current := s.Bot(id)
			if current == nil {
				err = errors.New(L("Unknown bot"))
			} else if saved == nil {
				err = errors.New(L("The CLI did not return Access settings. Update the Runner and try again."))
			} else if !EqualBotPermissions(current.Permissions, prior) && !EqualBotPermissions(current.Permissions, saved) {
				err = errors.New(L("Access changed while this sheet was open. Reopen it to review the current settings."))
			} else {
				current.Permissions = saved.Clone()
				s.rosterTouched(id)
			}
		}
		if done != nil {
			done(err)
		}
	}
	if s.IsMock {
		s.post(func() { finish(desired, nil) })
		return
	}
	Async(s, func() (*BotPermissions, error) {
		reply, err := call[struct {
			Bot WireBot `json:"bot"`
		}](s, "bots.update", map[string]any{"id": id, "permissions": desired})
		return reply.Bot.Permissions, err
	}, finish)
}

// The demo supplies invented instance labels and declarations, never provider credentials.
func MockBotPermissionCatalog() BotPermissionCatalog {
	tools := []PermissionTool{{Name: "list_messages", Capability: "read"}, {Name: "create_draft", Capability: "draft"}, {Name: "send_message", Capability: "write"}}
	return BotPermissionCatalog{
		LocalTools:  []string{"codemode", "read", "write", "edit", "grep", "find", "ls", "bash", "bash_input", "bash_output", "recall", "memory_update", "memory_log", "list_teammates", "message_bot", "create_bot", "edit_bot", "routines", "stage_review", "search_plugins", "install_plugin", "connect_plugin"},
		Connections: []PermissionConnection{{ID: "gmail-11111111111111111111111111111111", Name: "Gmail · Work", Tools: tools}, {ID: "gmail-22222222222222222222222222222222", Name: "Gmail · Personal", Tools: tools}},
	}
}
