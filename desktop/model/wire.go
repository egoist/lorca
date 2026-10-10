package model

import (
	"cmp"
	"slices"
	"strings"
	"time"
)

// Wire shapes the CLI sends over 127.0.0.1, and their mapping into the app's model, after the
// macOS app's Protocol.swift. Keys are snake_case on the wire; a field the CLI may leave out or
// send as null is a pointer.

// WireRelayProblem is why the last try to connect to the relay failed.
type WireRelayProblem struct {
	// Message is the error as it came: the relay's answer, or why none came.
	Message        string `json:"message"`
	UnknownMachine bool   `json:"unknown_machine,omitempty"`
}

type WirePluginStatus struct {
	ID          string  `json:"id"`
	Name        string  `json:"name"`
	Description string  `json:"description"`
	Version     string  `json:"version"`
	Icon        string  `json:"icon"`
	State       string  `json:"state"`
	Detail      string  `json:"detail"`
	Source      *string `json:"source"`
	ServiceID   *string `json:"service_id"`
	AccountName *string `json:"account_name"`
}

type WireMcpTool struct {
	Name        string  `json:"name"`
	Title       *string `json:"title"`
	Description *string `json:"description"`
	ReadOnly    *bool   `json:"read_only"`
	Hidden      *bool   `json:"hidden"`
}

// WireMcpServer is a server of a Runner's mcp.json, as `mcp.list` and `mcp.get` answer.
type WireMcpServer struct {
	Name      string            `json:"name"`
	ID        string            `json:"id"`
	Enabled   bool              `json:"enabled"`
	Config    map[string]any    `json:"config"`
	Problem   *string           `json:"problem"`
	Status    *WirePluginStatus `json:"status"`
	SignsIn   *bool             `json:"signs_in"`
	SignedIn  *bool             `json:"signed_in"`
	ToolCount *int              `json:"tool_count"`
	Tools     []WireMcpTool     `json:"tools"`
}

type WireMcpFile struct {
	Path    string          `json:"path"`
	Error   *string         `json:"error"`
	Servers []WireMcpServer `json:"servers"`
}

// WireParsedServers is `mcp.parse`: the servers pasted JSON holds.
type WireParsedServers struct {
	Servers []struct {
		Name    *string        `json:"name"`
		Config  map[string]any `json:"config"`
		Problem *string        `json:"problem"`
	} `json:"servers"`
}

type WireDeviceUpdate struct {
	Auto   bool    `json:"auto"`
	Latest *string `json:"latest"`
	State  *string `json:"state"`
	Error  *string `json:"error"`
}

type WireDevice struct {
	ID           string             `json:"id"`
	Name         string             `json:"name"`
	Model        string             `json:"model"`
	OS           string             `json:"os"`
	OSVersion    string             `json:"os_version"`
	MachineKey   string             `json:"machine_key"`
	IsThisDevice bool               `json:"is_this_device"`
	Status       string             `json:"status"`
	LastSeen     float64            `json:"last_seen"`
	Plugins      []WirePluginStatus `json:"plugins"`
	Channels     []WireChannel      `json:"channels"`
	// Unknown is a machine the relay lists that never sent its `machine` blob: no name, no OS.
	Unknown bool              `json:"unknown"`
	Version *string           `json:"version"`
	Update  *WireDeviceUpdate `json:"update"`
}

type WireAttachment struct {
	ID     string `json:"id"`
	Name   string `json:"name"`
	Mime   string `json:"mime"`
	Size   int64  `json:"size"`
	Width  *int   `json:"width"`
	Height *int   `json:"height"`
}

type WireBot struct {
	ID          string          `json:"id"`
	Name        string          `json:"name"`
	Description string          `json:"description"`
	SymbolName  string          `json:"symbol_name"`
	Accent      string          `json:"accent"`
	RunnerID    string          `json:"runner_id"`
	Provider    string          `json:"provider"`
	Model       *string         `json:"model"`
	Thinking    *string         `json:"thinking"`
	Avatar      *WireAttachment `json:"avatar"`
	Permissions *BotPermissions `json:"permissions"`
	CreatedAt   float64         `json:"created_at"`
}

type WireRun struct {
	SessionID  *string `json:"session_id"`
	Command    *string `json:"command"`
	State      string  `json:"state"`
	Prompt     *string `json:"prompt"`
	Output     *string `json:"output"`
	Device     *string `json:"device"`
	Reason     *string `json:"reason"`
	Rule       *string `json:"rule"`
	HandedOver *bool   `json:"handed_over"`
	Background *bool   `json:"background"`
}

type WireAgent struct {
	ID       string  `json:"id"`
	Kind     string  `json:"kind"`
	Host     *string `json:"host"`
	Task     *string `json:"task"`
	Folder   *string `json:"folder"`
	Branch   *string `json:"branch"`
	State    string  `json:"state"`
	Stalled  *bool   `json:"stalled"`
	Question *struct {
		Kind    string   `json:"kind"`
		Text    *string  `json:"text"`
		Command *string  `json:"command"`
		Choices []string `json:"choices"`
		Reason  *string  `json:"reason"`
		Rule    *string  `json:"rule"`
	} `json:"question"`
	Output  *string `json:"output"`
	Outcome *string `json:"outcome"`
	Device  *string `json:"device"`
}

type WireAuthor struct {
	Kind  string  `json:"kind"`
	BotID *string `json:"bot_id"`
	Name  *string `json:"name"`
}

// WireChannel is a channel in its Runner's record.
type WireChannel struct {
	ID        string `json:"id"`
	BotID     string `json:"bot_id"`
	Name      string `json:"name"`
	Service   string `json:"service"`
	AccountID string `json:"account_id"`
	Chats     []struct {
		ID    string `json:"id"`
		Title string `json:"title"`
	} `json:"chats"`
	Listen       ChannelListen `json:"listen"`
	Task         string        `json:"task"`
	State        string        `json:"state"`
	Detail       string        `json:"detail"`
	HeldDelivery string        `json:"held_delivery"`
}

// WireChatChannel is where a channel's conversation happens.
type WireChatChannel struct {
	ChannelID string  `json:"channel_id"`
	Service   string  `json:"service"`
	AccountID string  `json:"account_id"`
	ChatID    string  `json:"chat_id"`
	ThreadID  *string `json:"thread_id"`
}

type WireBody struct {
	Kind          string           `json:"kind"`
	Text          *string          `json:"text"`
	Attachments   []WireAttachment `json:"attachments"`
	Name          *string          `json:"name"`
	Summary       *string          `json:"summary"`
	Detail        *string          `json:"detail"`
	IsRunning     *bool            `json:"is_running"`
	Description   *string          `json:"description"`
	TargetBotID   *string          `json:"target_bot_id"`
	ScriptCommand *string          `json:"script_command"`
	From          *string          `json:"from"`
	To            *string          `json:"to"`
	Reason        *string          `json:"reason"`
	PluginID      *string          `json:"plugin_id"`
	PluginName    *string          `json:"plugin_name"`
	Tool          *string          `json:"tool"`
	Decision      *string          `json:"decision"`
	Link          *string          `json:"link"`
	Code          *string          `json:"code"`
	Rule          *string          `json:"rule"`
	Command       *string          `json:"command"`
	Run           *WireRun         `json:"run"`
	Agent         *WireAgent       `json:"agent"`
	ReviewID      *string          `json:"review_id"`
	Version       *uint64          `json:"version"`
	State         *string          `json:"state"`
	Account       *string          `json:"account"`
	Draft         *DraftFields     `json:"draft"`
	Note          *string          `json:"note"`
	Direct        *bool            `json:"direct"`
	Secret        *WireSecretAsk   `json:"secret"`
	ReplyTo       *struct {
		MessageID string     `json:"message_id"`
		Author    WireAuthor `json:"author"`
		Text      string     `json:"text"`
	} `json:"reply_to"`
}

// WireSecretAsk is a secret request's ask, in the CLI's shape.
type WireSecretAsk struct {
	Use    string `json:"use"`
	Site   string `json:"site"`
	Fields []struct {
		Name  string `json:"name"`
		Label string `json:"label"`
	} `json:"fields"`
}

// WireSecret is one of a Runner's secrets, as `secrets.list` answers it.
type WireSecret struct {
	ID        string  `json:"id"`
	BotID     string  `json:"bot_id"`
	Name      string  `json:"name"`
	Label     string  `json:"label"`
	Use       string  `json:"use"`
	Site      string  `json:"site"`
	UpdatedAt float64 `json:"updated_at"`
}

func (w WireSecret) model() SavedSecret {
	return SavedSecret{ID: w.ID, BotID: w.BotID, Name: w.Name, Label: w.Label, Use: SecretUse(w.Use), Site: w.Site, UpdatedAt: w.UpdatedAt}
}

type WireMessage struct {
	ID     string     `json:"id"`
	ChatID string     `json:"chat_id"`
	Author WireAuthor `json:"author"`
	Body   WireBody   `json:"body"`
	State  struct {
		Kind  string  `json:"kind"`
		Error *string `json:"error"`
	} `json:"state"`
	CreatedAt    float64         `json:"created_at"`
	Queued       *bool           `json:"queued"`
	Output       *Output         `json:"output"`
	Notification NotificationTag `json:"notification"`
}

type WireChatUsage struct {
	ContextTokens           int      `json:"context_tokens"`
	ContextWindow           int      `json:"context_window"`
	InputTokens             int      `json:"input_tokens"`
	OutputTokens            int      `json:"output_tokens"`
	CacheReadTokens         int      `json:"cache_read_tokens"`
	CostUSD                 float64  `json:"cost_usd"`
	Turns                   int      `json:"turns"`
	Model                   string   `json:"model"`
	APICostUSD              float64  `json:"api_cost_usd"`
	SubscriptionEstimateUSD float64  `json:"subscription_estimate_usd"`
	UnknownPriceCalls       uint64   `json:"unknown_price_calls"`
	PricingKinds            []string `json:"pricing_kinds"`
}

type WireChat struct {
	ID          string           `json:"id"`
	Kind        string           `json:"kind"`
	Title       *string          `json:"title"`
	BotIDs      []string         `json:"bot_ids"`
	OwnerBotID  *string          `json:"owner_bot_id"`
	Description *string          `json:"description"`
	IsPinned    bool             `json:"is_pinned"`
	SectionID   *string          `json:"section_id"`
	IsHidden    bool             `json:"is_hidden"`
	Mute        *WireMute        `json:"mute"`
	CreatedAt   float64          `json:"created_at"`
	Messages    []WireMessage    `json:"messages"`
	UnreadCount *int             `json:"unread_count"`
	Usage       *WireChatUsage   `json:"usage"`
	HasMore     *bool            `json:"has_more"`
	Channel     *WireChatChannel `json:"channel"`
}

type WireMute struct {
	Until *float64 `json:"until"`
}

type WireSection struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	Collapsed bool   `json:"collapsed"`
}

// ToSections are the sidebar's sections as the roster lists them.
func ToSections(wire []WireSection) []*Section {
	sections := make([]*Section, 0, len(wire))
	for _, section := range wire {
		sections = append(sections, &Section{ID: section.ID, Name: section.Name, Collapsed: section.Collapsed})
	}
	return sections
}

type WireRoutine struct {
	ID           string   `json:"id"`
	BotID        string   `json:"bot_id"`
	Name         string   `json:"name"`
	Prompt       string   `json:"prompt"`
	Schedule     string   `json:"schedule"`
	ScheduleText *string  `json:"schedule_text"`
	IsEnabled    bool     `json:"is_enabled"`
	PausedReason *string  `json:"paused_reason"`
	LastRunAt    *float64 `json:"last_run_at"`
	LastOutcome  *string  `json:"last_outcome"`
	NextRunAt    *float64 `json:"next_run_at"`
	IsRunning    *bool    `json:"is_running"`
	Check        *string  `json:"check"`
	CreatedAt    float64  `json:"created_at"`

	Timezone        *string            `json:"timezone"`
	MissedRunPolicy *string            `json:"missed_run_policy"`
	State           *string            `json:"state"`
	Health          *WireRoutineHealth `json:"health"`

	OnceAt      *float64 `json:"once_at"`
	PullRequest *struct {
		Repo   string  `json:"repo"`
		Number int     `json:"number"`
		Title  *string `json:"title"`
		URL    *string `json:"url"`
	} `json:"pull_request"`
	Calendar *struct {
		Account   *string `json:"account"`
		Matching  *string `json:"matching"`
		Minutes   *int    `json:"minutes"`
		After     *bool   `json:"after"`
		NextEvent *struct {
			Title *string `json:"title"`
		} `json:"next_event"`
	} `json:"calendar"`
}

type WireAutoReview struct {
	IsEnabled bool `json:"is_enabled"`
	Rules     []struct {
		ID       string  `json:"id"`
		Text     string  `json:"text"`
		Behavior string  `json:"behavior"`
		Tool     *string `json:"tool"`
	} `json:"rules"`
	// Provider is the provider that reviews, absent for the bot's own; Models the review models
	// picked by provider.
	Provider *string           `json:"provider"`
	Models   map[string]string `json:"models"`
}

type WireProvider struct {
	Kind        string            `json:"kind"`
	IsConnected bool              `json:"is_connected"`
	Detail      string            `json:"detail"`
	ReviewModel *string           `json:"review_model"`
	BaseURL     *string           `json:"base_url"`
	Name        *string           `json:"name"`
	API         *string           `json:"api"`
	Models      []WireCustomModel `json:"models"`
}

// WireCustomModel is a custom provider's model, with what its server's model list said of it, and
// in a status the thinking levels it takes.
type WireCustomModel struct {
	ID            string   `json:"id"`
	Name          *string  `json:"name"`
	ContextWindow *int     `json:"context_window"`
	MaxOutput     *int     `json:"max_output"`
	Images        *bool    `json:"images"`
	Levels        []string `json:"levels"`
}

// WireModelList is `providers.list_models`: the models a server lists that its protocol can run,
// in its order. Listed is false when the server publishes no list.
type WireModelList struct {
	Listed bool              `json:"listed"`
	Models []WireCustomModel `json:"models"`
}

type WireRunningTurn struct {
	JobID     string  `json:"job_id"`
	ChatID    string  `json:"chat_id"`
	BotID     string  `json:"bot_id"`
	RoutineID *string `json:"routine_id"`
}

// WireModel is a model the CLI's catalog offers, in the catalog's order: each provider's first
// that does not decide is its default.
type WireModel struct {
	Provider string   `json:"provider"`
	ID       string   `json:"id"`
	Name     string   `json:"name"`
	Levels   []string `json:"levels"`
	Decides  bool     `json:"decides"`
}

type WireSnapshot struct {
	Attention           *AttentionView    `json:"attention"`
	Budgets             []BudgetState     `json:"budgets"`
	Version             string            `json:"version"`
	HasIdentity         bool              `json:"has_identity"`
	IsIdentityDevice    bool              `json:"is_identity_device"`
	IdentityID          *string           `json:"identity_id"`
	ThisDeviceID        *string           `json:"this_device_id"`
	RelayURL            *string           `json:"relay_url"`
	RelayConnected      bool              `json:"relay_connected"`
	RelayUpdateRequired *bool             `json:"relay_update_required"`
	RelayError          *WireRelayProblem `json:"relay_error"`
	Devices             []WireDevice      `json:"devices"`
	Bots                []WireBot         `json:"bots"`
	Chats               []WireChat        `json:"chats"`
	Sections            []WireSection     `json:"sections"`
	Routines            []WireRoutine     `json:"routines"`
	Reviews             []ReviewItem      `json:"reviews"`
	Tasks               []DurableTask     `json:"tasks"`
	AutoReview          *WireAutoReview   `json:"auto_review"`
	SharedLinks         []SharedLink      `json:"shared_links"`
	Providers           []WireProvider    `json:"providers"`
	Models              []WireModel       `json:"models"`
	Playbooks           []PlaybookSummary `json:"playbooks"`
	RunningChatIDs      []string          `json:"running_chat_ids"`
	RunningTurns        []WireRunningTurn `json:"running_turns"`
}

type WireRosterChanged struct {
	Devices     []WireDevice    `json:"devices"`
	Bots        []WireBot       `json:"bots"`
	Chats       []WireChat      `json:"chats"`
	Sections    []WireSection   `json:"sections"`
	Routines    []WireRoutine   `json:"routines"`
	AutoReview  *WireAutoReview `json:"auto_review"`
	SharedLinks []SharedLink    `json:"shared_links"`
	Providers   []WireProvider  `json:"providers"`
	// Models are the catalog's models again, so a newer catalog the CLI installs reaches the pickers.
	Models    []WireModel        `json:"models"`
	Playbooks *[]PlaybookSummary `json:"playbooks"`
}

type WireMessagePage struct {
	Messages []WireMessage `json:"messages"`
	HasMore  bool          `json:"has_more"`
}

type WireSearchResults struct {
	Chats []struct {
		ChatID  string `json:"chat_id"`
		Snippet string `json:"snippet"`
	} `json:"chats"`
	Messages []struct {
		ChatID    string     `json:"chat_id"`
		MessageID string     `json:"message_id"`
		Snippet   string     `json:"snippet"`
		Author    WireAuthor `json:"author"`
		CreatedAt float64    `json:"created_at"`
	} `json:"messages"`
}

type WireMarketplaceServer struct {
	Type    string   `json:"type"`
	URL     *string  `json:"url"`
	Command *string  `json:"command"`
	Args    []string `json:"args"`
	Auth    *struct {
		Type string `json:"type"`
	} `json:"auth"`
}

type WireMarketplacePlugin struct {
	ID          string                           `json:"id"`
	Name        string                           `json:"name"`
	Description *string                          `json:"description"`
	Icon        *string                          `json:"icon"`
	Homepage    *string                          `json:"homepage"`
	Author      *string                          `json:"author"`
	Category    *string                          `json:"category"`
	Featured    *bool                            `json:"featured"`
	Tags        []string                         `json:"tags"`
	Servers     map[string]WireMarketplaceServer `json:"servers"`
	Variables   []struct {
		Name        string  `json:"name"`
		Description *string `json:"description"`
		Secret      *bool   `json:"secret"`
		Required    *bool   `json:"required"`
	} `json:"variables"`
	Skills []struct {
		Name        string  `json:"name"`
		Description *string `json:"description"`
	} `json:"skills"`
	InstalledOn   []string `json:"installed_on"`
	NamedAccounts *bool    `json:"named_accounts"`
}

type WireTemplateRoutine struct {
	Name         string  `json:"name"`
	Schedule     string  `json:"schedule"`
	ScheduleText *string `json:"schedule_text"`
	Prompt       string  `json:"prompt"`
}

type WireBotTemplate struct {
	ID          string                `json:"id"`
	Name        string                `json:"name"`
	Summary     *string               `json:"summary"`
	Description string                `json:"description"`
	SymbolName  *string               `json:"symbol_name"`
	Accent      *string               `json:"accent"`
	Category    *string               `json:"category"`
	Featured    *bool                 `json:"featured"`
	Author      *string               `json:"author"`
	Plugins     []string              `json:"plugins"`
	Routines    []WireTemplateRoutine `json:"routines"`
	Memory      []string              `json:"memory"`
}

type WireMarketplace struct {
	Packs   []WorkflowPack          `json:"packs"`
	Plugins []WireMarketplacePlugin `json:"plugins"`
	Bots    []WireBotTemplate       `json:"bots"`
}

type WirePluginDetail struct {
	Manifest struct {
		Homepage *string `json:"homepage"`
	} `json:"manifest"`
	Status    WirePluginStatus `json:"status"`
	Variables []struct {
		Name        string  `json:"name"`
		Description *string `json:"description"`
		Secret      bool    `json:"secret"`
		Required    bool    `json:"required"`
		IsSet       bool    `json:"is_set"`
		Value       *string `json:"value"`
	} `json:"variables"`
	Servers []struct {
		Name string `json:"name"`
		Kind string `json:"kind"`
		Auth struct {
			URL      *string `json:"url"`
			OAuth    *bool   `json:"oauth"`
			SignedIn *bool   `json:"signed_in"`
			Code     *string `json:"code"`
			Link     *string `json:"link"`
		} `json:"auth"`
	} `json:"servers"`
	Skills []struct {
		Name        string  `json:"name"`
		Description *string `json:"description"`
	} `json:"skills"`
}

type WireBotMemory struct {
	BotID  string  `json:"bot_id"`
	Here   bool    `json:"here"`
	Runner string  `json:"runner"`
	Path   *string `json:"path"`
	Index  *struct {
		Text      string `json:"text"`
		Hash      string `json:"hash"`
		Lines     int    `json:"lines"`
		Bytes     int    `json:"bytes"`
		Truncated bool   `json:"truncated"`
		MaxLines  int    `json:"max_lines"`
		MaxBytes  int    `json:"max_bytes"`
	} `json:"index"`
	Topics []string `json:"topics"`
	Logs   []string `json:"logs"`
}

type WirePairStart struct {
	Nonce         string `json:"nonce"`
	PairingString string `json:"pairing_string"`
}

type WirePairStatus struct {
	State string  `json:"state"`
	Error *string `json:"error"`
	// Device is the Device that joined, once `completed`.
	Device *struct {
		ID   string `json:"id"`
		Name string `json:"name"`
	} `json:"device"`
}

// WireSyncAccount is `sync.account`: the account's providers once this Device's first pull has
// its credentials.
type WireSyncAccount struct {
	Providers []WireProvider `json:"providers"`
}

type WireJobEvent struct {
	ChatID    string  `json:"chat_id"`
	BotID     string  `json:"bot_id"`
	JobID     string  `json:"job_id"`
	RoutineID *string `json:"routine_id"`
}

type WireJobRetry struct {
	ChatID      string `json:"chat_id"`
	BotID       string `json:"bot_id"`
	Attempt     int    `json:"attempt"`
	MaxAttempts int    `json:"max_attempts"`
	DelayMS     int    `json:"delay_ms"`
	Error       string `json:"error"`
}

type WireRelayStatus struct {
	Connected      bool              `json:"connected"`
	URL            *string           `json:"url"`
	UpdateRequired *bool             `json:"update_required"`
	Error          *WireRelayProblem `json:"error"`
}

// MARK: - Mapping

func seconds(value float64) time.Time {
	if value == 0 {
		return time.Time{}
	}
	return time.UnixMilli(int64(value * 1000))
}

func str(value *string) string {
	if value == nil {
		return ""
	}
	return *value
}

func flag(value *bool) bool { return value != nil && *value }

func number(value *int) int {
	if value == nil {
		return 0
	}
	return *value
}

func ToPlugin(wire WirePluginStatus) InstalledPlugin {
	state := PluginState(wire.State)
	switch state {
	case PluginReady, PluginNeedsSetup, PluginNeedsAuth, PluginInsufficientAccess, PluginConnecting, PluginError:
	default:
		state = PluginUnknown
	}
	return InstalledPlugin{
		ID:          wire.ID,
		Name:        wire.Name,
		Description: wire.Description,
		Version:     wire.Version,
		Icon:        wire.Icon,
		State:       state,
		Detail:      wire.Detail,
		Source:      str(wire.Source),
		ServiceID:   str(wire.ServiceID),
		AccountName: str(wire.AccountName),
	}
}

func ToMcpServer(wire WireMcpServer) McpServer {
	server := McpServer{
		Name:      wire.Name,
		ID:        wire.ID,
		Enabled:   wire.Enabled,
		Entry:     ToMcpEntry(wire.Config),
		Problem:   str(wire.Problem),
		SignsIn:   flag(wire.SignsIn),
		SignedIn:  flag(wire.SignedIn),
		ToolCount: -1,
	}
	if wire.Status != nil {
		status := ToPlugin(*wire.Status)
		server.Status = &status
	}
	if wire.ToolCount != nil {
		server.ToolCount = *wire.ToolCount
	}
	for _, tool := range wire.Tools {
		server.Tools = append(server.Tools, McpTool{Name: tool.Name, Title: str(tool.Title), Description: str(tool.Description), ReadOnly: flag(tool.ReadOnly), Hidden: flag(tool.Hidden)})
	}
	return server
}

func ToMcpFile(wire WireMcpFile) McpFile {
	file := McpFile{Path: wire.Path, Error: str(wire.Error)}
	for _, server := range wire.Servers {
		file.Servers = append(file.Servers, ToMcpServer(server))
	}
	return file
}

func ToParsedServers(wire WireParsedServers) []ParsedServer {
	var out []ParsedServer
	for _, server := range wire.Servers {
		parsed := ParsedServer{Name: str(server.Name), Problem: str(server.Problem)}
		if server.Config != nil {
			parsed.Entry = ToMcpEntry(server.Config)
		}
		out = append(out, parsed)
	}
	return out
}

func ToDevice(wire WireDevice) *Device {
	device := &Device{
		ID:           wire.ID,
		Name:         wire.Name,
		Model:        wire.Model,
		OS:           DeviceOS(wire.OS),
		OSVersion:    wire.OSVersion,
		IsThisDevice: wire.IsThisDevice,
		Status:       StatusOffline,
		LastSeen:     seconds(wire.LastSeen),
		MachineKey:   wire.MachineKey,
		Version:      str(wire.Version),
	}
	switch device.OS {
	case OSMacOS, OSLinux, OSWindows, OSIOS, OSIPadOS, OSAndroid:
	default:
		device.OS = OSLinux
	}
	if wire.Unknown {
		device.Name, device.OS = L("Unknown Device"), OSUnknown
	}
	switch wire.Status {
	case "online":
		device.Status = StatusOnline
	case "pairing":
		device.Status = StatusPairing
	}
	for _, plugin := range wire.Plugins {
		device.Plugins = append(device.Plugins, ToPlugin(plugin))
	}
	for _, channel := range wire.Channels {
		device.Channels = append(device.Channels, ToChannel(channel))
	}
	if wire.Update != nil {
		update := &DeviceUpdate{Auto: wire.Update.Auto, Latest: str(wire.Update.Latest), Error: str(wire.Update.Error)}
		switch state := str(wire.Update.State); state {
		case "installing", "restarting", "installed":
			update.State = state
		}
		device.Update = update
	}
	return device
}

func ToAttachment(wire WireAttachment) Attachment {
	return Attachment{ID: wire.ID, Name: wire.Name, Mime: wire.Mime, Size: wire.Size, Width: number(wire.Width), Height: number(wire.Height)}
}

func ToBot(wire WireBot) *Bot {
	bot := &Bot{
		ID:          wire.ID,
		Name:        wire.Name,
		Description: wire.Description,
		SymbolName:  wire.SymbolName,
		Accent:      "indigo",
		RunnerID:    wire.RunnerID,
		Provider:    "deepseek",
		Model:       str(wire.Model),
		Thinking:    str(wire.Thinking),
		Permissions: wire.Permissions.Clone(),
		CreatedAt:   seconds(wire.CreatedAt),
	}
	if IsAccent(wire.Accent) {
		bot.Accent = Accent(wire.Accent)
	}
	if IsProviderKind(wire.Provider) {
		bot.Provider = wire.Provider
	}
	if wire.Avatar != nil {
		avatar := ToAttachment(*wire.Avatar)
		bot.Avatar = &avatar
	}
	return bot
}

func toAuthor(wire WireAuthor) Author {
	switch wire.Kind {
	case "you":
		return You
	case "bot":
		return BotAuthor(str(wire.BotID))
	case "contact":
		return Author{Kind: AuthorContact, Name: str(wire.Name)}
	}
	return System
}

// ToChannel reads a channel from its Runner's record.
func ToChannel(wire WireChannel) Channel {
	channel := Channel{
		ID: wire.ID, BotID: wire.BotID, Name: wire.Name, Service: wire.Service, AccountID: wire.AccountID,
		Listen: wire.Listen, Task: wire.Task, State: ChannelState(wire.State), Detail: wire.Detail, HeldDelivery: wire.HeldDelivery,
	}
	switch channel.State {
	case ChannelListening, ChannelPaused, ChannelHeld, ChannelOffline:
	default:
		channel.State = ChannelListening
	}
	for _, chat := range wire.Chats {
		channel.Chats = append(channel.Chats, ChannelChat{ID: chat.ID, Title: chat.Title})
	}
	return channel
}

func ToMessage(wire WireMessage) *Message {
	message := &Message{
		ID:           wire.ID,
		Author:       toAuthor(wire.Author),
		CreatedAt:    seconds(wire.CreatedAt),
		Queued:       flag(wire.Queued),
		Output:       wire.Output,
		Notification: wire.Notification,
	}
	body := wire.Body
	switch body.Kind {
	case "tool":
		tool := &ToolInvocation{
			Name:          str(body.Name),
			Summary:       str(body.Summary),
			Detail:        str(body.Detail),
			IsRunning:     flag(body.IsRunning),
			Description:   str(body.Description),
			TargetBotID:   str(body.TargetBotID),
			ScriptCommand: str(body.ScriptCommand),
		}
		if tool.Name == "" {
			tool.Name = "tool"
		}
		if run := body.Run; run != nil {
			state := CommandState(run.State)
			switch state {
			case CommandChecking, CommandAsking, CommandRunning, CommandWaiting, CommandExited, CommandFailed, CommandStopped, CommandDenied, CommandExpired, CommandDismissed:
			default:
				state = CommandStopped
			}
			tool.Run = &CommandRun{
				SessionID:  str(run.SessionID),
				Command:    str(run.Command),
				State:      state,
				Prompt:     str(run.Prompt),
				Output:     str(run.Output),
				Device:     str(run.Device),
				Reason:     str(run.Reason),
				Rule:       str(run.Rule),
				HasRule:    run.Rule != nil,
				HandedOver: flag(run.HandedOver),
				Background: flag(run.Background),
			}
		}
		if agent := body.Agent; agent != nil {
			state := AgentState(agent.State)
			switch state {
			case AgentChecking, AgentAsking, AgentStarting, AgentWorking, AgentIdle, AgentExited, AgentFailed, AgentStopped, AgentDenied, AgentExpired, AgentDismissed:
			default:
				state = AgentStopped
			}
			card := &AgentRun{
				ID: agent.ID, Kind: agent.Kind, Host: str(agent.Host), Task: str(agent.Task), Folder: str(agent.Folder),
				Branch: str(agent.Branch), State: state, Stalled: flag(agent.Stalled), Output: str(agent.Output),
				Outcome: str(agent.Outcome), Device: str(agent.Device),
			}
			if q := agent.Question; q != nil {
				switch q.Kind {
				case "start", "command", "choices", "text":
					card.Question = &AgentQuestion{
						Kind: q.Kind, Text: str(q.Text), Command: str(q.Command), Choices: q.Choices,
						Reason: str(q.Reason), Rule: str(q.Rule), HasRule: q.Rule != nil,
					}
				}
			}
			tool.Agent = card
		}
		message.Body = Body{Kind: BodyTool, Tool: tool}
	case "handoff":
		message.Body = Body{Kind: BodyHandoff, Handoff: Handoff{From: str(body.From), To: str(body.To), Reason: str(body.Reason)}}
	case "notice":
		message.Body = Body{Kind: BodyNotice, Text: str(body.Text)}
	case "permission":
		decision := PermissionDecision(str(body.Decision))
		switch decision {
		case DecisionPending, DecisionAllowed, DecisionAlways, DecisionDenied, DecisionExpired, DecisionDismissed, DecisionConnected, DecisionFailed:
		default:
			decision = DecisionPending
		}
		message.Body = Body{Kind: BodyPermission, Request: &PermissionRequest{
			PluginID:   str(body.PluginID),
			PluginName: str(body.PluginName),
			Tool:       str(body.Tool),
			Summary:    str(body.Summary),
			Decision:   decision,
			Link:       str(body.Link),
			Code:       str(body.Code),
			Reason:     str(body.Reason),
			Rule:       str(body.Rule),
			HasRule:    body.Rule != nil,
			Command:    str(body.Command),
			Secret:     secretAsk(body.Secret),
		}}
	case "draft":
		card := &DraftCard{ReviewID: str(body.ReviewID), State: str(body.State), PluginID: str(body.PluginID),
			Account: str(body.Account), Note: str(body.Note), Direct: flag(body.Direct)}
		if card.State == "" {
			card.State = "pending"
		}
		if body.Version != nil {
			card.Version = *body.Version
		}
		if body.Draft != nil {
			card.Fields = body.Draft.Clone()
		}
		message.Body = Body{Kind: BodyDraft, Draft: card}
	default:
		message.Body = Body{Kind: BodyText, Text: str(body.Text)}
	}
	switch wire.State.Kind {
	case "thinking":
		message.State = MessageState{Kind: StateThinking}
	case "streaming":
		message.State = MessageState{Kind: StateStreaming}
	case "failed":
		message.State = MessageState{Kind: StateFailed, Error: str(wire.State.Error)}
		if wire.State.Error == nil {
			message.State.Error = L("Failed")
		}
	}
	for _, attachment := range body.Attachments {
		message.Attachments = append(message.Attachments, ToAttachment(attachment))
	}
	if reply := body.ReplyTo; reply != nil {
		message.ReplyTo = &ReplyQuote{MessageID: reply.MessageID, Author: toAuthor(reply.Author), Text: reply.Text}
	}
	return message
}

func ToUsage(wire WireChatUsage) *ChatUsage {
	return &ChatUsage{
		ContextTokens:           wire.ContextTokens,
		ContextWindow:           wire.ContextWindow,
		InputTokens:             wire.InputTokens,
		OutputTokens:            wire.OutputTokens,
		CacheReadTokens:         wire.CacheReadTokens,
		CostUSD:                 wire.CostUSD,
		Turns:                   wire.Turns,
		Model:                   wire.Model,
		APICostUSD:              wire.APICostUSD,
		SubscriptionEstimateUSD: wire.SubscriptionEstimateUSD,
		UnknownPriceCalls:       wire.UnknownPriceCalls,
		PricingKinds:            slices.Clone(wire.PricingKinds),
	}
}

// ToChat is a chat from a snapshot (with its newest messages) or a roster event (without: the chat
// keeps the messages this app has, from `existing`).
func ToChat(wire WireChat, existing *Chat) *Chat {
	chat := &Chat{
		ID:        wire.ID,
		Kind:      ChatGroup,
		BotIDs:    slices.Clone(wire.BotIDs),
		IsPinned:  wire.IsPinned,
		CreatedAt: seconds(wire.CreatedAt),
	}
	if wire.Kind == "dm" {
		chat.Kind = ChatDM
	} else {
		chat.CustomTitle = str(wire.Title)
		chat.GroupDescription = str(wire.Description)
	}
	if wire.Channel != nil {
		// A channel's conversation is named after where it happens.
		chat.CustomTitle = str(wire.Title)
		chat.Channel = &ChatChannel{
			ChannelID: wire.Channel.ChannelID, Service: wire.Channel.Service, AccountID: wire.Channel.AccountID,
			ChatID: wire.Channel.ChatID, ThreadID: str(wire.Channel.ThreadID),
		}
	}
	chat.OwnerBotID = str(wire.OwnerBotID)
	chat.SectionID = str(wire.SectionID)
	chat.IsHidden = wire.IsHidden
	if wire.Mute != nil {
		chat.Mute = &ChatMute{}
		if wire.Mute.Until != nil {
			chat.Mute.Until = seconds(*wire.Mute.Until)
		}
	}
	if wire.Messages != nil {
		for _, message := range wire.Messages {
			chat.Messages = append(chat.Messages, ToMessage(message))
		}
	} else if existing != nil {
		chat.Messages = existing.Messages
	}
	switch {
	case wire.UnreadCount != nil:
		chat.UnreadCount = *wire.UnreadCount
	case existing != nil:
		chat.UnreadCount = existing.UnreadCount
	}
	switch {
	case wire.HasMore != nil:
		chat.HasMore = *wire.HasMore
	case existing != nil:
		chat.HasMore = existing.HasMore
	}
	if wire.Usage != nil {
		chat.Usage = ToUsage(*wire.Usage)
	} else if existing != nil {
		chat.Usage = existing.Usage
	}
	return chat
}

func ToRoutine(wire WireRoutine) *Routine {
	routine := &Routine{
		ID:           wire.ID,
		BotID:        wire.BotID,
		Name:         wire.Name,
		Prompt:       wire.Prompt,
		Schedule:     wire.Schedule,
		ScheduleText: Schedule(cmp.Or(str(wire.ScheduleText), wire.Schedule)),
		IsEnabled:    wire.IsEnabled,
		PausedReason: str(wire.PausedReason),
		LastOutcome:  str(wire.LastOutcome),
		IsRunning:    flag(wire.IsRunning),
		Check:        str(wire.Check),
		HasCheck:     wire.Check != nil,
		CreatedAt:    seconds(wire.CreatedAt),
		// The CLI names the zone; "Local" reads as this computer's.
		Timezone:        cmp.Or(str(wire.Timezone), "Local"),
		MissedRunPolicy: cmp.Or(str(wire.MissedRunPolicy), "coalesce"),
		State:           str(wire.State),
		Health:          toRoutineHealth(wire.Health),
	}
	if routine.State == "" {
		routine.State = "paused"
		if wire.IsEnabled {
			routine.State = "on"
		}
	}
	if wire.LastRunAt != nil {
		routine.LastRunAt = seconds(*wire.LastRunAt)
	}
	if wire.NextRunAt != nil {
		routine.NextRunAt = seconds(*wire.NextRunAt)
	}
	// A one-time routine, a watch, and a routine around events are worded here, in the app's
	// language, from what the CLI says of them.
	switch {
	case wire.PullRequest != nil:
		routine.PullRequest = &RoutineWatch{Repo: wire.PullRequest.Repo, Number: wire.PullRequest.Number, Title: str(wire.PullRequest.Title), URL: str(wire.PullRequest.URL)}
		routine.ScheduleText = L("Watches %@", routine.PullRequest.Label())
	case wire.Calendar != nil:
		events := &RoutineCalendar{Account: str(wire.Calendar.Account), Matching: str(wire.Calendar.Matching)}
		if wire.Calendar.Minutes != nil {
			events.Minutes = *wire.Calendar.Minutes
		}
		events.After = flag(wire.Calendar.After)
		if wire.Calendar.NextEvent != nil {
			events.NextEventTitle = str(wire.Calendar.NextEvent.Title)
		}
		routine.Calendar = events
		routine.ScheduleText = AroundEvents(events.Minutes, events.After, events.Matching)
	case wire.OnceAt != nil:
		routine.OnceAt = seconds(*wire.OnceAt)
		routine.ScheduleText = Once(routine.OnceAt, routine.Timezone)
	}
	return routine
}

func ToAutoReview(wire *WireAutoReview) AutoReview {
	if wire == nil {
		return AutoReview{IsEnabled: true}
	}
	review := AutoReview{IsEnabled: wire.IsEnabled}
	if provider := str(wire.Provider); IsProviderKind(provider) {
		review.Provider = provider
	}
	for kind, model := range wire.Models {
		if IsProviderKind(kind) && model != "" {
			if review.Models == nil {
				review.Models = map[ProviderKind]string{}
			}
			review.Models[kind] = model
		}
	}
	for _, rule := range wire.Rules {
		behavior := "allow"
		if rule.Behavior == "ask" {
			behavior = "ask"
		}
		review.Rules = append(review.Rules, AutoReviewRule{ID: rule.ID, Text: rule.Text, Behavior: behavior, Tool: str(rule.Tool)})
	}
	return review
}

// ToProviders is the built-in providers, then the custom ones in the order they were added. A kind
// this build does not know is left out.
func ToProviders(wire []WireProvider) []ProviderCredential {
	var out []ProviderCredential
	for _, provider := range wire {
		if !IsProviderKind(provider.Kind) {
			continue
		}
		credential := ProviderCredential{Kind: provider.Kind, IsConnected: provider.IsConnected, Detail: provider.Detail, BaseURL: str(provider.BaseURL), ReviewModel: str(provider.ReviewModel)}
		if IsCustomKind(provider.Kind) {
			credential.Name = str(provider.Name)
			if api := str(provider.API); IsCustomAPI(api) {
				credential.API = CustomAPI(api)
			}
			credential.Models = []CustomModel{}
			for _, model := range provider.Models {
				credential.Models = append(credential.Models, ToCustomModel(model))
			}
		}
		out = append(out, credential)
	}
	return out
}

func ToCustomModel(wire WireCustomModel) CustomModel {
	return CustomModel{
		ID:            wire.ID,
		Name:          str(wire.Name),
		ContextWindow: number(wire.ContextWindow),
		MaxOutput:     number(wire.MaxOutput),
		Images:        wire.Images,
		Levels:        wire.Levels,
	}
}

func ToModels(wire []WireModel) []ProviderModel {
	out := make([]ProviderModel, 0, len(wire))
	for _, model := range wire {
		out = append(out, ProviderModel{Provider: model.Provider, ID: model.ID, Label: model.Name, Levels: model.Levels, Decides: model.Decides})
	}
	return out
}

func ToMarketplacePlugin(wire WireMarketplacePlugin) MarketplacePlugin {
	plugin := MarketplacePlugin{
		ID:            wire.ID,
		Name:          wire.Name,
		Description:   str(wire.Description),
		Icon:          str(wire.Icon),
		Homepage:      str(wire.Homepage),
		Author:        str(wire.Author),
		Category:      str(wire.Category),
		IsFeatured:    flag(wire.Featured),
		Tags:          wire.Tags,
		InstalledOn:   wire.InstalledOn,
		NamedAccounts: flag(wire.NamedAccounts),
	}
	names := make([]string, 0, len(wire.Servers))
	for name := range wire.Servers {
		names = append(names, name)
	}
	slices.Sort(names)
	for _, name := range names {
		server := wire.Servers[name]
		// A server Lorca answers itself (Telegram's, Slack's bot) is Lorca, not one to list.
		if server.Type == "builtin" {
			continue
		}
		address := str(server.URL)
		if address == "" {
			address = strings.Join(append([]string{str(server.Command)}, server.Args...), " ")
		}
		plugin.Servers = append(plugin.Servers, MarketplaceServer{
			Name:     name,
			Address:  address,
			IsRemote: server.Type == "http",
			SignsIn:  server.Auth != nil && server.Auth.Type == "oauth",
		})
	}
	for _, skill := range wire.Skills {
		plugin.Skills = append(plugin.Skills, NamedText{Name: skill.Name, Description: str(skill.Description)})
	}
	for _, variable := range wire.Variables {
		plugin.Variables = append(plugin.Variables, PluginVariable{Name: variable.Name, Description: str(variable.Description), Secret: flag(variable.Secret), Required: flag(variable.Required)})
	}
	return plugin
}

func ToBotTemplate(wire WireBotTemplate) BotTemplate {
	template := BotTemplate{
		ID:          wire.ID,
		Name:        wire.Name,
		Summary:     str(wire.Summary),
		Description: wire.Description,
		SymbolName:  cmp.Or(str(wire.SymbolName), "sparkles"),
		Accent:      "indigo",
		Category:    str(wire.Category),
		IsFeatured:  flag(wire.Featured),
		Author:      str(wire.Author),
		Plugins:     wire.Plugins,
		Memory:      wire.Memory,
	}
	if IsAccent(str(wire.Accent)) {
		template.Accent = Accent(str(wire.Accent))
	}
	for _, routine := range wire.Routines {
		template.Routines = append(template.Routines, TemplateRoutine{Name: routine.Name, ScheduleText: cmp.Or(str(routine.ScheduleText), routine.Schedule), Prompt: routine.Prompt})
	}
	return template
}

func ToMarketplace(wire WireMarketplace) Marketplace {
	out := Marketplace{Packs: wire.Packs}
	for _, plugin := range wire.Plugins {
		out.Plugins = append(out.Plugins, ToMarketplacePlugin(plugin))
	}
	for _, template := range wire.Bots {
		out.Bots = append(out.Bots, ToBotTemplate(template))
	}
	return out
}

func ToPluginDetail(wire WirePluginDetail) PluginDetail {
	detail := PluginDetail{Status: ToPlugin(wire.Status), Homepage: str(wire.Manifest.Homepage)}
	for _, variable := range wire.Variables {
		detail.Variables = append(detail.Variables, PluginDetailVariable{
			Name: variable.Name, Description: str(variable.Description), Secret: variable.Secret, Required: variable.Required, IsSet: variable.IsSet, Value: str(variable.Value),
		})
	}
	for _, server := range wire.Servers {
		detail.Servers = append(detail.Servers, PluginDetailServer{
			Name: server.Name, Kind: server.Kind, URL: str(server.Auth.URL), OAuth: flag(server.Auth.OAuth), SignedIn: flag(server.Auth.SignedIn), Code: str(server.Auth.Code), Link: str(server.Auth.Link),
		})
	}
	for _, skill := range wire.Skills {
		detail.Skills = append(detail.Skills, NamedText{Name: skill.Name, Description: str(skill.Description)})
	}
	return detail
}

func ToBotMemory(wire WireBotMemory) BotMemory {
	memory := BotMemory{BotID: wire.BotID, Here: wire.Here, Runner: wire.Runner, Path: str(wire.Path), MaxLines: 200, MaxBytes: 24_000, Topics: wire.Topics, Logs: wire.Logs}
	if index := wire.Index; index != nil {
		memory.Text, memory.Hash, memory.Lines, memory.Bytes, memory.Truncated = index.Text, index.Hash, index.Lines, index.Bytes, index.Truncated
		memory.MaxLines, memory.MaxBytes = index.MaxLines, index.MaxBytes
	}
	return memory
}

func secretAsk(wire *WireSecretAsk) *SecretAsk {
	if wire == nil {
		return nil
	}
	ask := &SecretAsk{Use: SecretUse(wire.Use), Site: wire.Site}
	for _, field := range wire.Fields {
		ask.Fields = append(ask.Fields, SecretField{Name: field.Name, Label: field.Label})
	}
	return ask
}
