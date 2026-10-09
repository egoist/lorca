package model

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"time"
)

// The seeded demo (`LORCA_MOCK=1`), after the macOS app's MockData: the snapshot the CLI would
// send, for screenshots and for working on the views without a CLI. The marketplace is the CLI's
// own bundled index, read from the checkout the demo runs in.

var nonAlnum = regexp.MustCompile(`[^a-z0-9]+`)

// minutesAgo is a demo time on the formatters' clock, so a test that sets `Now` sees the same
// stamps every run.
func minutesAgo(minutes float64) time.Time {
	return Now().Add(-time.Duration(minutes * float64(time.Minute)))
}

func mockMessage(author Author, body Body, at time.Time) *Message {
	return &Message{ID: NewMessageID(), Author: author, Body: body, CreatedAt: at}
}

func textBody(text string) Body { return Body{Kind: BodyText, Text: text} }

func mockPlugins() []InstalledPlugin {
	plugins := []InstalledPlugin{
		{ID: "github", Name: "GitHub", Description: "Issues, pull requests, code search, and repositories on GitHub.", Version: "1", Icon: "chevron.left.forwardslash.chevron.right", State: PluginReady, Detail: "Ready"},
		{ID: "linear", Name: "Linear", Description: "Issues, projects, and cycles in Linear.", Version: "1", Icon: "line.3.horizontal.decrease.circle", State: PluginNeedsAuth, Detail: "Sign in"},
	}
	for _, server := range mockMcpServers() {
		if server.Status != nil {
			plugins = append(plugins, *server.Status)
		}
	}
	return plugins
}

// mockMcpServers is Workbench's mcp.json: a command, a remote server, one waiting for its sign-in,
// and one off.
func mockMcpServers() []McpServer {
	plugin := func(id, name, description, icon string, state PluginState, detail string) *InstalledPlugin {
		return &InstalledPlugin{ID: id, Name: name, Description: description, Icon: icon, State: state, Detail: detail, Source: "mcp.json"}
	}
	return []McpServer{
		{
			Name: "filesystem", ID: "filesystem", Enabled: true,
			Entry:  McpEntry{"command": "npx", "args": []string{"-y", "@modelcontextprotocol/server-filesystem", "~/Documents/Notes"}, "description": "My notes folder"},
			Status: plugin("filesystem", "filesystem", "My notes folder", "terminal", PluginReady, "Ready"), ToolCount: 4,
			Tools: []McpTool{
				{Name: "read_text_file", Description: "Read the complete contents of a file as text.", ReadOnly: true},
				{Name: "list_directory", Description: "Get a detailed listing of all files and directories in a path.", ReadOnly: true},
				{Name: "search_files", Description: "Recursively search for files and directories matching a pattern.", ReadOnly: true},
				{Name: "write_file", Description: "Create a new file or completely overwrite an existing file."},
			},
		},
		{
			Name: "deepwiki", ID: "deepwiki", Enabled: true,
			Entry:  McpEntry{"type": "http", "url": "https://mcp.deepwiki.com/mcp"},
			Status: plugin("deepwiki", "deepwiki", "Remote MCP server · mcp.deepwiki.com", "globe", PluginReady, "Ready"), ToolCount: 3,
			Tools: []McpTool{
				{Name: "read_wiki_structure", Description: "Get a list of documentation topics for a GitHub repository.", ReadOnly: true},
				{Name: "read_wiki_contents", Description: "View documentation about a GitHub repository.", ReadOnly: true},
				{Name: "ask_question", Description: "Ask any question about a GitHub repository.", ReadOnly: true},
			},
		},
		{
			Name: "sentry", ID: "sentry", Enabled: true,
			Entry:   McpEntry{"type": "http", "url": "https://mcp.sentry.dev/mcp"},
			Status:  plugin("sentry", "sentry", "Remote MCP server · mcp.sentry.dev", "globe", PluginNeedsAuth, "Sign in"),
			SignsIn: true, ToolCount: -1,
		},
		{
			Name: "postgres", ID: "postgres", Enabled: false,
			Entry:     McpEntry{"command": "uvx", "args": []string{"postgres-mcp", "--access-mode=restricted"}, "env": map[string]string{"DATABASE_URI": "postgresql://localhost/app"}, "disabled": true},
			ToolCount: -1,
		},
	}
}

func mockDevices() []*Device {
	return []*Device{
		{ID: "dev-workbench", Name: "Workbench", Model: "ThinkPad X1 Carbon Gen 13", OS: OSLinux, OSVersion: "Ubuntu 26.04", IsThisDevice: true, Status: StatusOnline, LastSeen: time.Now(), MachineKey: "mk_7c41…a09f", Plugins: mockPlugins(), Version: "0.1.11"},
		{ID: "dev-studio", Name: "Studio", Model: "Mac Studio (M3 Ultra)", OS: OSMacOS, OSVersion: "macOS 27.0", Status: StatusOnline, LastSeen: minutesAgo(1), MachineKey: "mk_1f88…23bd", Version: "0.1.10", Update: &DeviceUpdate{Auto: true, Latest: "0.1.11"}},
		{ID: "dev-closet", Name: "Closet PC", Model: "Desktop", OS: OSWindows, OSVersion: "Windows 11 25H2", Status: StatusOffline, LastSeen: minutesAgo(184), MachineKey: "mk_c052…77e1", Version: "0.1.11"},
		{ID: "dev-phone", Name: "iPhone", Model: "iPhone 17 Pro", OS: OSIOS, OSVersion: "iOS 27.0", Status: StatusOnline, LastSeen: minutesAgo(12), MachineKey: "mk_9e3d…51c8"},
	}
}

func mockProviders() []ProviderCredential {
	return []ProviderCredential{
		{Kind: "deepseek", IsConnected: true, Detail: "sk-live…4f2c", ReviewModel: "deepseek-flash"},
		{Kind: "anthropic", IsConnected: true, Detail: "sk-ant…8d1a", ReviewModel: "claude-haiku-4-5"},
		{Kind: "opencode", Detail: "Not connected", ReviewModel: "deepseek-v4.1-flash"},
		{Kind: "opencode-go", Detail: "Not connected", ReviewModel: "glm-5.3-flash"},
		{Kind: "chatgpt", IsConnected: true, Detail: "you@lorca.app", ReviewModel: "gpt-6-luna"},
		{Kind: "grok", Detail: "Not connected", ReviewModel: "grok-4.7"},
		{
			Kind: "custom:ollama", IsConnected: true, Detail: "http://localhost:11434/v1", BaseURL: "http://localhost:11434/v1", Name: "Ollama", API: APIChatCompletions,
			Models:      []CustomModel{{ID: "qwen3:8b", Levels: []string{"low", "medium", "high"}}, {ID: "llava", Levels: []string{"low", "medium", "high"}}},
			ReviewModel: "qwen3:8b",
		},
		{
			Kind: "custom:openrouter-decisions", IsConnected: true, Detail: "sk-or…9c0e · https://openrouter.ai/api/alpha/decisions", BaseURL: "https://openrouter.ai/api/alpha/decisions",
			Name: "OpenRouter Decisions", API: APISystemOne,
			Models:      []CustomModel{{ID: "typesafe/jev-1.13", Name: "TypeSafe: Jev 1.13"}, {ID: "perplexity/pplx-decider-v1.1-27b", Name: "Perplexity: Decider V1.1 27B"}},
			ReviewModel: "typesafe/jev-1.13",
		},
	}
}

func yes() *bool  { v := true; return &v }
func nope() *bool { v := false; return &v }

// mockListedModels is what a custom provider's server lists in the demo: a gateway's decision
// models or its catalog, a local server's few models, or no list at all.
func mockListedModels(baseURL string) ([]CustomModel, bool) {
	switch {
	case strings.Contains(baseURL, "openrouter.ai/api/alpha/decisions"):
		return []CustomModel{
			{ID: "typesafe/jev-1.13", Name: "TypeSafe: Jev 1.13", ContextWindow: 64_000},
			{ID: "openai/gpt-6-luna-decisions", Name: "OpenAI: GPT-6 Luna Decisions", ContextWindow: 1_050_000, Images: yes()},
			{ID: "perplexity/pplx-decider-v1.1-27b", Name: "Perplexity: Decider V1.1 27B", ContextWindow: 262_144, Images: yes()},
			{ID: "cloudflare/clef-flash", Name: "Cloudflare: Clef Flash", ContextWindow: 65_536, Images: yes()},
		}, true
	case strings.Contains(baseURL, "openrouter"):
		return []CustomModel{
			{ID: "anthropic/claude-sonnet-5", Name: "Anthropic: Claude Sonnet 5", ContextWindow: 1_000_000, Images: yes()},
			{ID: "openai/gpt-6-sol", Name: "OpenAI: GPT-6 Sol", ContextWindow: 1_050_000, Images: yes()},
			{ID: "deepseek/deepseek-v4.1-flash", Name: "DeepSeek: V4.1 Flash", ContextWindow: 1_000_000, Images: yes()},
			{ID: "moonshotai/kimi-k3", Name: "MoonshotAI: Kimi K3", ContextWindow: 1_048_576, Images: yes()},
			{ID: "qwen/qwen3.8-flash", Name: "Qwen: Qwen3.8 Flash", ContextWindow: 1_000_000, Images: yes()},
			{ID: "z-ai/glm-5.3-flash", Name: "Z.ai: GLM 5.3 Flash", ContextWindow: 200_000, Images: nope()},
			{ID: "meta-llama/llama-4-maverick", Name: "Meta: Llama 4 Maverick", ContextWindow: 1_048_576, Images: yes()},
			{ID: "mistralai/mistral-large-3", Name: "Mistral: Mistral Large 3", ContextWindow: 262_144, Images: yes()},
			{ID: "google/gemini-3.8-flash", Name: "Google: Gemini 3.8 Flash", ContextWindow: 1_048_576, Images: yes()},
		}, true
	case strings.Contains(baseURL, "localhost"), strings.Contains(baseURL, "127.0.0.1"):
		return []CustomModel{
			{ID: "qwen3:8b", ContextWindow: 40_960},
			{ID: "gemma3:12b", ContextWindow: 131_072, Images: yes()},
			{ID: "llava:13b", ContextWindow: 4_096, Images: yes()},
		}, true
	}
	return nil, false
}

// mockModels are a few of the catalog's models for each provider, with their thinking levels, and
// OpenCode Zen's decision models.
func mockModels() []ProviderModel {
	all := []string{"off", "low", "medium", "high", "xhigh", "max"}
	on := []string{"low", "medium", "high", "xhigh", "max"}
	return []ProviderModel{
		{"deepseek", "deepseek-flash", "DeepSeek V4.1 Flash", all, false},
		{"anthropic", "claude-opus-5", "Claude Opus 5", all, false},
		{"anthropic", "claude-fable-5-1", "Claude Fable 5.1", on, false},
		{"anthropic", "claude-haiku-4-5", "Claude Haiku 4.5", []string{"off", "minimal", "low", "medium", "high"}, false},
		{"opencode", "deepseek-v4.1-flash", "DeepSeek V4.1 Flash", []string{"low", "high", "max"}, false},
		{"opencode", "kimi-k3", "Kimi K3", []string{"max"}, false},
		{"opencode", "big-pickle", "Big Pickle", nil, false},
		{"opencode", "jev-1.13", "Jev 1.13", nil, true},
		{"opencode", "jev-1.13-free", "Jev 1.13 Free", nil, true},
		{"opencode-go", "glm-5.3-flash", "GLM-5.3 Flash", []string{"low", "high", "max"}, false},
		{"chatgpt", "gpt-6.1-sol", "GPT-6.1 Sol", on, false},
		{"chatgpt", "gpt-6-luna", "GPT-6 Luna", on, false},
		{"grok", "grok-4.7", "Grok 4.7", []string{"low", "medium", "high", "xhigh"}, false},
	}
}

// mockScheduleTexts are each template's routines in words, as the CLI says them.
var mockScheduleTexts = map[string][]string{
	"morning-briefing":   {"Weekdays at 8:30 AM"},
	"pr-reviewer":        {"Weekdays at 9:00 AM"},
	"lookout":            {"Every 2 hours"},
	"issue-triager":      {"Weekdays at 10:00 AM and 4:00 PM"},
	"error-watch":        {"Weekdays at 9:00 AM, 1:00 PM, and 5:00 PM"},
	"competitor-watcher": {"Mondays at 9:00 AM"},
	"docs-keeper":        {"Fridays at 3:00 PM"},
}

// marketplaceIndex finds the CLI's bundled index in the checkout: `LORCA_MARKETPLACE_INDEX`, else
// crates/cli/marketplace/index.json above the working directory.
func marketplaceIndex() []byte {
	if path := os.Getenv("LORCA_MARKETPLACE_INDEX"); path != "" {
		if data, err := os.ReadFile(path); err == nil {
			return data
		}
	}
	dir, _ := os.Getwd()
	for range 6 {
		if data, err := os.ReadFile(filepath.Join(dir, "crates", "cli", "marketplace", "index.json")); err == nil {
			return data
		}
		dir = filepath.Dir(dir)
	}
	return nil
}

// mockMarketplace is the bundled index's plugins and bots, as the CLI serves them.
func mockMarketplace() Marketplace {
	var index WireMarketplace
	if data := marketplaceIndex(); data != nil {
		_ = json.Unmarshal(data, &index)
	}
	for i := range index.Plugins {
		if id := index.Plugins[i].ID; id == "github" || id == "linear" {
			index.Plugins[i].InstalledOn = []string{"dev-workbench"}
		}
	}
	for i := range index.Bots {
		template := &index.Bots[i]
		for j := range template.Routines {
			if texts := mockScheduleTexts[template.ID]; j < len(texts) {
				text := texts[j]
				template.Routines[j].ScheduleText = &text
			}
		}
	}
	return ToMarketplace(index)
}

// mockSharedLinks is Writer, shared as a link a day ago.
func mockSharedLinks() []SharedLink {
	return []SharedLink{{ID: "mock-link", URL: "https://lorca.app/t/mock-link#dGhpcyBpcyBub3QgYSByZWFsIGtleSwganVzdCBhIGRlbW8",
		BotID: "bot-quill", Name: "Writer", Selection: TemplateSelection{Profile: true, MemoryIDs: []string{"memory-voice"}},
		UpdatedAt: float64(minutesAgo(60 * 26).Unix())}}
}

func mockAutoReview() AutoReview {
	return AutoReview{
		IsEnabled: true,
		Rules: []AutoReviewRule{
			{ID: "ar-1", Text: "use GitHub create_issue", Behavior: "allow", Tool: "github/create_issue"},
			{ID: "ar-2", Text: "comment on a pull request", Behavior: "ask"},
		},
		Provider: "custom:openrouter-decisions",
		Models:   map[ProviderKind]string{"custom:openrouter-decisions": "perplexity/pplx-decider-v1.1-27b", "anthropic": "claude-opus-5"},
	}
}

func nextNineAM() time.Time {
	now := time.Now()
	next := time.Date(now.Year(), now.Month(), now.Day(), 9, 0, 0, 0, now.Location())
	if !next.After(now) {
		next = next.AddDate(0, 0, 1)
	}
	return next
}

func mockRoutines() []*Routine {
	return []*Routine{
		{
			ID: "rt-brief", BotID: "bot-nova", Name: "Morning brief",
			Prompt:   "Read the recent messages in every chat you are in and the launch checklist in the workspace. Post a short brief: what changed, what needs a decision, and what the team will do first.",
			Schedule: "0 9 * * 1-5", ScheduleText: Schedule("Weekdays at 9:00 AM"), IsEnabled: true,
			LastRunAt: minutesAgo(190), LastOutcome: "sent", NextRunAt: nextNineAM(), CreatedAt: minutesAgo(60 * 24 * 12),
		},
		{
			ID: "rt-checklist", BotID: "bot-nova", Name: "Launch checklist",
			Prompt:   "Review the launch checklist in the workspace and the latest team replies. Report new blockers or completed milestones, or PASS when nothing changed.",
			Schedule: "every 2h", ScheduleText: Schedule("Every 2 hours"),
			LastRunAt: minutesAgo(60 * 30), LastOutcome: "pass", CreatedAt: minutesAgo(60 * 24 * 3),
		},
		{
			ID: "rt-reviews", BotID: "bot-nova", Name: "Review requests",
			Prompt:   "Read the pull requests your check found and tell me which need my review first, with one line on each.",
			Schedule: "every 10m", ScheduleText: Schedule("Every 10 minutes"), IsEnabled: true,
			LastRunAt: minutesAgo(60 * 5), LastOutcome: "sent", NextRunAt: time.Now().Add(4 * time.Minute),
			Check: strings.Join([]string{
				"const seen = load('seen') ?? [];",
				"const result = await tools.github__search_pull_requests({ query: 'is:open review-requested:@me' });",
				"const fresh = result.structuredContent.items.filter((pr) => !seen.includes(pr.number));",
				"store('seen', [...seen, ...fresh.map((pr) => pr.number)]);",
				"return fresh.map((pr) => `#${pr.number} ${pr.title}`).join('\\n');",
			}, "\n"),
			HasCheck: true, CreatedAt: minutesAgo(60 * 24 * 2),
		},
	}
}

func mockBots() []*Bot {
	return []*Bot{
		{ID: "bot-nova", Name: "Project Manager", Description: "Plans the work and delegates it to the team. Breaks work down, hands it off with message_bot, and summarizes what came back.", SymbolName: "list.bullet.clipboard.fill", Accent: "indigo", RunnerID: "dev-workbench", Provider: "chatgpt", CreatedAt: minutesAgo(60 * 24 * 21)},
		{ID: "bot-patch", Name: "Developer", Description: "Implements changes in small diffs, explains the tradeoff in one line, and never invents APIs.", SymbolName: "chevron.left.forwardslash.chevron.right", Accent: "blue", RunnerID: "dev-studio", Provider: "deepseek", CreatedAt: minutesAgo(60 * 24 * 18)},
		{ID: "bot-scout", Name: "Researcher", Description: "Gathers context, reads the sources before answering, cites them, and says when it is unsure.", SymbolName: "magnifyingglass", Accent: "teal", RunnerID: "dev-studio", Provider: "deepseek", CreatedAt: minutesAgo(60 * 24 * 12)},
		{ID: "bot-quill", Name: "Writer", Description: "Writes docs, copy, and release notes in plain language: short sentences, no filler, and no exclamation marks.", SymbolName: "pencil.and.scribble", Accent: "pink", RunnerID: "dev-workbench", Provider: "anthropic", Permissions: mockWriterAccess(), CreatedAt: minutesAgo(60 * 24 * 9)},
		{ID: "bot-ember", Name: "DevOps", Description: "Handles deploys and incident triage, watches the relay, and always states the blast radius first.", SymbolName: "server.rack", Accent: "orange", RunnerID: "dev-closet", Provider: "deepseek", CreatedAt: minutesAgo(60 * 24 * 4)},
	}
}

// mockBudgets are the demo's limits: Project Manager's turns and Researcher's each have some,
// Researcher's newest turn stopped at its token limit, and Review requests used up its spending.
func mockBudgets() []BudgetState {
	now := float64(time.Now().Unix())
	usd := func(v float64) *float64 { return &v }
	n := func(v uint64) *uint64 { return &v }
	return []BudgetState{
		{Kind: "chat", ID: "chat-nova", RunnerID: "dev-workbench", ChatID: "chat-nova", Limits: BudgetLimits{MaxUSD: usd(2), MaxTokens: n(200_000)}, State: "ready", UpdatedAt: now - 60*60*24},
		{Kind: "chat", ID: "chat-scout", RunnerID: "dev-studio", ChatID: "chat-scout", Limits: BudgetLimits{MaxTokens: n(100_000), MaxRuntimeSecs: n(900)}, State: "ready", UpdatedAt: now - 60*60*24},
		{Kind: "job", ID: "job-demo-research", RunnerID: "dev-studio", ChatID: "chat-scout", Limits: BudgetLimits{MaxTokens: n(100_000), MaxRuntimeSecs: n(900)},
			Usage: BudgetUsage{Tokens: 100_412, APICostUSD: 0.21, RuntimeSecs: 384, Retries: 1, ConnectorCalls: 9}, State: "budget_exhausted", Reached: "tokens", UpdatedAt: now - 60*28},
		{Kind: "routine", ID: "rt-reviews", RunnerID: "dev-workbench", ChatID: "chat-nova", Limits: BudgetLimits{MaxUSD: usd(5), MaxRuntimeSecs: n(3600)},
			Usage: BudgetUsage{Tokens: 1_840_000, SubscriptionEstimateUSD: 5.02, RuntimeSecs: 2_760, ConnectorCalls: 64}, State: "budget_exhausted", Reached: "usd", UpdatedAt: now - 60*5},
	}
}

func mockChat(id string, kind ChatKind, botIDs []string, messages []*Message, extra func(*Chat)) *Chat {
	chat := &Chat{ID: id, Kind: kind, BotIDs: botIDs, Messages: messages, CreatedAt: time.Now()}
	extra(chat)
	return chat
}

func mockChats() []*Chat {
	return []*Chat{
		mockChat("chat-relay", ChatGroup, []string{"bot-nova", "bot-patch", "bot-scout"}, launchRoomThread(), func(c *Chat) {
			c.CustomTitle = "Launch room"
			c.GroupDescription = "Ship the relay launch: the TLS rollout, the release notes, and the go/no-go call on Friday."
			c.IsPinned = true
			c.CreatedAt = minutesAgo(400)
		}),
		mockChat("chat-nova", ChatDM, []string{"bot-nova"}, managerThread(), func(c *Chat) {
			c.CreatedAt = minutesAgo(60 * 30)
			c.Usage = &ChatUsage{ContextTokens: 18_400, ContextWindow: 400_000, InputTokens: 212_000, OutputTokens: 31_000, CacheReadTokens: 160_000,
				CostUSD: 0.86, Turns: 14, Model: "gpt-5.5", SubscriptionEstimateUSD: 0.86, PricingKinds: []string{"subscription_estimate"}}
		}),
		mockChat("chat-patch", ChatDM, []string{"bot-patch"}, developerThread(), func(c *Chat) { c.UnreadCount = 2; c.CreatedAt = minutesAgo(60 * 26) }),
		mockChat("chat-launch", ChatGroup, []string{"bot-quill", "bot-nova"}, launchThread(), func(c *Chat) { c.CustomTitle = "Launch copy"; c.CreatedAt = minutesAgo(60 * 52) }),
		mockChat("chat-scout", ChatDM, []string{"bot-scout"}, researcherThread(), func(c *Chat) {
			c.UnreadCount = 1
			c.CreatedAt = minutesAgo(60 * 24 * 12)
			c.Usage = &ChatUsage{ContextTokens: 61_000, ContextWindow: 128_000, InputTokens: 402_000, OutputTokens: 22_000, CacheReadTokens: 290_000,
				CostUSD: 0.34, Turns: 9, Model: "deepseek-chat", APICostUSD: 0.34, PricingKinds: []string{"api"}}
		}),
		mockChat("chat-quill", ChatDM, []string{"bot-quill"}, writerThread(), func(c *Chat) { c.CreatedAt = minutesAgo(60 * 24 * 9) }),
		mockChat("chat-ember", ChatDM, []string{"bot-ember"}, devopsThread(), func(c *Chat) { c.CreatedAt = minutesAgo(60 * 72) }),
	}
}

func launchRoomThread() []*Message {
	checklist := mockMessage(BotAuthor("bot-nova"), textBody("The launch checklist is ready:\n\n- **Onboarding** — reviewed on Linux, Windows, and iPhone\n- **Website** — guide updated, links checked\n- **Launch copy** — Writer's draft is ready\n\nOnly your final review is left."), minutesAgo(7))
	checklist.ID = "msg-mock-checklist"
	reply := mockMessage(You, textBody("Great. Keep the announcement as a draft until I've reviewed it."), minutesAgo(5))
	reply.ReplyTo = &ReplyQuote{
		MessageID: "msg-mock-checklist",
		Author:    BotAuthor("bot-nova"),
		Text:      "The launch checklist is ready: Onboarding — reviewed on Linux, Windows, and iPhone Website — guide updated, links checked Launch copy — Writer's draft is ready Only your final review is left.",
	}
	return []*Message{
		mockMessage(You, textBody("Launch review: @Researcher check onboarding, @Developer check the site. @Project Manager pull it together."), minutesAgo(14)),
		mockMessage(BotAuthor("bot-scout"), textBody("Walked through setup on Linux, Windows, and iPhone. Pairing is clear. One gap: the guide needs to explain that your computer runs the bots while you chat from your phone."), minutesAgo(12)),
		mockMessage(BotAuthor("bot-patch"), textBody("Updated the getting-started guide and checked every download link. The site builds cleanly. The changes are ready to review."), minutesAgo(9)),
		checklist,
		reply,
		mockMessage(BotAuthor("bot-nova"), textBody("Saved in `launch/announcement.md`. I'll include the checklist in your morning brief."), minutesAgo(4)),
	}
}

func managerThread() []*Message {
	return []*Message{
		mockMessage(You, textBody("Give me a short launch brief every weekday at 9. Focus on blockers and decisions."), minutesAgo(192)),
		mockMessage(BotAuthor("bot-nova"), textBody("Your **Morning brief** runs weekdays at 9:00 AM on Workbench. I'll read our chats and the launch checklist, then post what changed and what needs you."), minutesAgo(191)),
		mockMessage(System, Body{Kind: BodyNotice, Text: "Routine · Morning brief"}, minutesAgo(190)),
		mockMessage(BotAuthor("bot-nova"), textBody("**Today's focus: the launch.**\n\n- Researcher is reviewing the setup guide.\n- Developer is checking the website and download links.\n- Writer has a first draft of the announcement.\n\nI'll bring their updates together in Launch room."), minutesAgo(190)),
		mockMessage(System, Body{Kind: BodyNotice, Text: "Waiting for your review · $ git tag v1.4.0 && git push origin v1.4.0"}, minutesAgo(189)),
		mockMessage(System, Body{Kind: BodyNotice, Text: "Waiting for your review · GitHub: add_issue_comment · owner: lorca-app, repo: relay, issue_number: 214"}, minutesAgo(188)),
		mockMessage(You, textBody("Ask Writer to keep the announcement short and lead with what people can do."), minutesAgo(36)),
		mockMessage(BotAuthor("bot-nova"), Body{Kind: BodyTool, Tool: &ToolInvocation{Name: "message_bot", Summary: "Messaged Writer", Detail: "Draft a short launch announcement that leads with what people can do.", TargetBotID: "bot-quill"}}, minutesAgo(35)),
		mockMessage(BotAuthor("bot-nova"), textBody("Writer has the brief. I'll bring the draft back here when it's ready."), minutesAgo(34)),
		// Writer's handoff report, which wakes Project Manager in this chat.
		mockMessage(BotAuthor("bot-quill"), Body{Kind: BodyHandoff, Handoff: Handoff{From: "bot-quill", To: "bot-nova", Reason: "Draft saved to `launch/announcement.md`. It leads with what people can do and stays under 60 words."}}, minutesAgo(24)),
		mockMessage(BotAuthor("bot-nova"), textBody("Writer's draft is in `launch/announcement.md`: three short sentences that open with building a team of bots. I added it to the launch checklist for your review."), minutesAgo(23)),
		mockMessage(System, Body{Kind: BodyNotice, Text: "Waiting for your review · Draft: Launch announcement"}, minutesAgo(22)),
	}
}

// mockReviews is what the demo's bots left for review, as the CLI sends items: a command and a
// GitHub call held while Project Manager's routine ran, and a draft it wants edited.
func mockReviews() []*ReviewItem {
	item := func(id string, minutes float64, payload, account, resource, rationale string) *ReviewItem {
		target, _ := json.Marshal(ReviewTarget{Account: account, Resource: resource})
		raw := `{"id":"` + id + `","runner_id":"dev-workbench","bot_id":"bot-nova","origin":{"chat_id":"chat-nova"},"target":` + string(target) +
			`,"rationale":` + quoted(rationale) + `,"payload":` + payload + `,"version":1,"revision":1,"preconditions":{"workdir":"~/Projects/relay","files":[]},"state":"pending"}`
		var review ReviewItem
		if err := json.Unmarshal([]byte(raw), &review); err != nil {
			panic(err)
		}
		review.CreatedAt = float64(minutesAgo(minutes).Unix())
		return &review
	}
	return []*ReviewItem{
		item("review-tag", 189, `{"kind":"shell","arguments":{"command":"git tag v1.4.0 && git push origin v1.4.0","description":"Tag the release"}}`,
			"Workbench", "~/Projects/relay", "Pushes a release tag to the shared repository, which starts the release build."),
		item("review-comment", 188, `{"kind":"plugin","plugin_id":"github","server_name":"github","tool":"add_issue_comment","arguments":{"owner":"lorca-app","repo":"relay","issue_number":214,"body":"Release notes are ready: the TLS rollout, the new pairing flow, and the relay quotas."}}`,
			"GitHub", "add_issue_comment · owner: lorca-app, repo: relay, issue_number: 214", "Posts a public comment on a pull request."),
		item("review-draft", 33, `{"kind":"draft","text":`+quoted("Lorca 1.4 is out. Pair your phone in one step, keep chats in sync across every Device, and run bots on the computers you already own.\n\nUpdate from the app, or download it from lorca.app.")+`}`,
			"Launch room", "Launch announcement", "Writer's draft, shortened to lead with what people can do. Edit it before it goes to the team."),
	}
}

func quoted(text string) string {
	data, _ := json.Marshal(text)
	return string(data)
}

func developerThread() []*Message {
	return []*Message{
		mockMessage(You, textBody("Check the getting-started page and make sure every download link works."), minutesAgo(55)),
		mockMessage(BotAuthor("bot-patch"), textBody("The Windows and Linux downloads and the CLI install links work. I also checked the docs links in both languages."), minutesAgo(28)),
		mockMessage(BotAuthor("bot-patch"), textBody("The website build passes. I've left the changes ready for review."), minutesAgo(27)),
	}
}

func researcherThread() []*Message {
	return []*Message{
		mockMessage(You, textBody("Read the setup guide as a new user. What would you want explained sooner?"), minutesAgo(80)),
		mockMessage(BotAuthor("bot-scout"), textBody("I'd explain the Device roles right after pairing: your computer runs the bots, and your phone lets you chat with them. I added that note to `research/onboarding.md`."), minutesAgo(45)),
		mockMessage(You, textBody("Read the setup guides of five similar apps and compare what each explains first."), minutesAgo(34)),
		mockMessage(System, Body{Kind: BodyNotice, Text: "Stopped at the token limit. Raise it in Limits to resume."}, minutesAgo(28)),
	}
}

func writerThread() []*Message {
	return []*Message{
		mockMessage(BotAuthor("bot-nova"), Body{Kind: BodyHandoff, Handoff: Handoff{From: "bot-nova", To: "bot-quill", Reason: "Draft a short launch announcement that leads with what people can do."}}, minutesAgo(35)),
		mockMessage(BotAuthor("bot-quill"), textBody("Create a team of bots for your everyday work. Give each one a role, bring them into a group chat, and pick up the conversation from your phone. Lorca runs the bots on your computers and encrypts your chats before they sync.\n\nDraft saved to `launch/announcement.md`."), minutesAgo(24)),
		mockMessage(You, textBody("File an issue for the pairing section of the docs."), minutesAgo(12)),
		// The Writer's Access lets it read GitHub and draft reviews, not open issues.
		mockMessage(BotAuthor("bot-quill"), Body{Kind: BodyPermission, Request: &PermissionRequest{PluginID: "github", PluginName: "GitHub", Tool: "access", Summary: "GitHub · create_issue", Decision: DecisionPending}}, minutesAgo(11)),
		mockMessage(BotAuthor("bot-quill"), textBody("I can't open issues on GitHub: my Access only lets me read it and draft reviews. I left a request above if you want to allow it."), minutesAgo(11)),
	}
}

func launchThread() []*Message {
	return []*Message{
		mockMessage(You, textBody("@Writer write a short welcome for the setup guide. @Project Manager check that it covers the first steps."), minutesAgo(110)),
		mockMessage(BotAuthor("bot-quill"), textBody("Meet your first bot. Give it a name and a job, connect your model provider, and send a message. Add more bots when you need a team, or pair your phone to take the conversation with you."), minutesAgo(108)),
		mockMessage(BotAuthor("bot-nova"), textBody("That covers the first session. The pairing guide follows it with a computer-and-phone walkthrough."), minutesAgo(106)),
	}
}

func devopsThread() []*Message {
	return []*Message{
		mockMessage(You, textBody("Check the relay health and disk usage when you're back online."), minutesAgo(60*4)),
		mockMessage(System, Body{Kind: BodyNotice, Text: "Closet PC is offline. DevOps's turn is queued on the relay and will run when that Runner reconnects."}, minutesAgo(60*3+4)),
	}
}

// MockBackupPhrase is the demo's backup phrase, for onboarding without a CLI.
var MockBackupPhrase = []string{"k4mq", "7rth", "2bnz", "wq5f", "j3xd", "pv82", "ct6m", "9hsa", "e7lw", "4knr", "zb3u", "m5yq"}

// mockPlaybooks are the demo's skills: two of the Developer's and a draft it proposed, and one for
// the Launch room.
func mockPlaybooks() []PlaybookRecord {
	record := func(id string, scope PlaybookScope, status string, content PlaybookContent, hoursAgo ...float64) PlaybookRecord {
		r := PlaybookRecord{ID: id, Scope: scope, Status: status, Revision: uint64(len(hoursAgo)), Hash: id + "-hash", Content: &content}
		for i, hours := range hoursAgo {
			step := PlaybookRevision{ID: fmt.Sprintf("%s-%d", id, i+1), Revision: uint64(i + 1), Status: "saved", Content: &content, DeviceID: "dev-studio",
				CreatedAt: float64(minutesAgo(hours * 60).Unix())}
			if i == len(hoursAgo)-1 {
				step.Status = status
			}
			if i%2 == 1 {
				step.DeviceID = "dev-workbench"
			}
			step.Provenance.Kind = "edit"
			if i == 0 {
				step.Provenance.Kind = "manual"
				if status == "draft" {
					step.Provenance.Kind = "workflow"
				}
			}
			r.Revisions = append(r.Revisions, step)
		}
		return r
	}
	return []PlaybookRecord{
		record("playbook-release-notes", BotScope("bot-patch"), "saved", PlaybookContent{
			Name: "release-notes", Description: "Turn the merged pull requests since the last tag into release notes.",
			Instructions: "1. List the pull requests merged since the last release tag.\n2. Group them under Added, Changed, and Fixed.\n3. Write one plain line per change, in the user's words where the PR has them.\n4. Leave out internal refactors and dependency bumps.\n5. Show the draft before posting it anywhere.",
			Examples:     "Fixed: The composer keeps your draft when you switch chats.",
			References:   []PlaybookFile{{Path: "references/style.md", Text: "Short lines. No ticket numbers. Present tense."}},
			Scripts:      []PlaybookFile{{Path: "scripts/merged-since-tag.sh", Text: "git log --merges --oneline \"$(git describe --tags --abbrev=0)\"..HEAD"}},
		}, 80, 26, 3),
		record("playbook-flaky-tests", BotScope("bot-patch"), "draft", PlaybookContent{
			Name: "flaky-tests", Description: "Rerun a failing test in isolation before calling it a real failure.",
			Instructions: "When a test fails in CI, rerun it alone three times. If it passes every time, report it as flaky with the failing seed; otherwise look for the bug.",
		}, 1),
		record("playbook-ship-checklist", BotScope("bot-patch"), "saved", PlaybookContent{
			Name: "ship-checklist", Description: "Check the build, the changelog, and the version before a release.",
			Instructions: "Run the full test suite, confirm the changelog has a section for the new version, and bump the version in one commit.",
		}, 200),
		record("playbook-launch-post", GroupScope("chat-relay"), "saved", PlaybookContent{
			Name: "launch-post", Description: "Write the launch announcement from the checklist and the release notes.",
			Instructions: "Open with what the user can do now. Keep it under 150 words. Link the release notes last.",
		}, 50, 5),
	}
}

// mockDraftedSkill is what the demo's drafting model writes for a capture.
func mockDraftedSkill(kind string) PlaybookContent {
	if kind == "corrections" {
		return PlaybookContent{Name: "plain-replies", Description: "How the user wants replies written.",
			Instructions: "Answer in two or three sentences. Use metric units. Don't add a summary at the end.",
			References:   []PlaybookFile{}, Scripts: []PlaybookFile{}}
	}
	return PlaybookContent{Name: "relay-deploy", Description: "Deploy the relay to Railway and check it came up.",
		Instructions: "1. Build the relay image from the Dockerfile.\n2. Deploy it to the staging service.\n3. Check /health answers within a minute.\n4. Tell the user the version that is live.",
		Examples:     "\"Relay 0.4.2 is live on staging; /health answered in 3 s.\"",
		References:   []PlaybookFile{}, Scripts: []PlaybookFile{}}
}
