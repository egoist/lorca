// Package model is the app's model, after the macOS app's Models.swift and AppStore: everything
// comes from the CLI over 127.0.0.1, read into these types by wire.go, and kept by the Store.
// The views read it on the main thread, where the Store changes it.
package model

import (
	"crypto/rand"
	"encoding/hex"
	"fmt"
	"math"
	"net/url"
	"slices"
	"strings"
	"time"
	"unicode"
)

// MARK: - Providers

// ProviderKind names a provider: a built-in one ("deepseek", "anthropic", …) or one the user
// added, `custom:` and a slug of the name it was added with.
type ProviderKind = string

// ProviderKinds are the built-in kinds, in the order Settings lists them. The account's custom
// providers follow them, in the order they were added (`Store.ProviderKinds`).
var ProviderKinds = []ProviderKind{"deepseek", "anthropic", "opencode", "opencode-go", "chatgpt", "grok"}

const customPrefix = "custom:"

// IsCustomKind is whether the kind is a provider the user added.
func IsCustomKind(kind string) bool { return strings.HasPrefix(kind, customPrefix) }

// IsProviderKind is a built-in kind or a custom one. Any other kind is one this build does not know.
func IsProviderKind(value string) bool {
	return slices.Contains(ProviderKinds, value) || IsCustomKind(value)
}

// ProviderName is the name people know the provider by: "DeepSeek", "OpenCode Zen". A custom
// provider's is the one the user gave it, from the account's providers, and its slug once it is
// deleted.
func ProviderName(kind ProviderKind, providers []ProviderCredential) string {
	if IsCustomKind(kind) {
		for _, provider := range providers {
			if provider.Kind == kind && provider.Name != "" {
				return provider.Name
			}
		}
		return strings.TrimPrefix(kind, customPrefix)
	}
	switch kind {
	case "deepseek":
		return "DeepSeek"
	case "anthropic":
		return "Anthropic"
	case "opencode":
		return "OpenCode Zen"
	case "opencode-go":
		return "OpenCode Go"
	case "chatgpt":
		return "ChatGPT"
	case "grok":
		return "Grok"
	}
	return kind
}

// UsesAPIKey is whether the provider connects with a pasted API key; ChatGPT and Grok sign in
// through the browser instead, and a custom provider is set up in its own sheet.
func UsesAPIKey(kind ProviderKind) bool {
	switch kind {
	case "deepseek", "anthropic", "opencode", "opencode-go":
		return true
	}
	return false
}

func ProviderSymbol(kind ProviderKind) string {
	if IsCustomKind(kind) {
		return "server.rack"
	}
	if UsesAPIKey(kind) {
		return "key.fill"
	}
	return "person.badge.key.fill"
}

func ProviderSubtitle(kind ProviderKind) string {
	if IsCustomKind(kind) {
		return L("Custom")
	}
	if UsesAPIKey(kind) {
		return L("API key")
	}
	return L("Subscription")
}

// SignInRequirement is what the sign-in needs, for the subscription providers.
func SignInRequirement(kind ProviderKind) string {
	switch kind {
	case "chatgpt":
		return L("It needs a ChatGPT subscription.")
	case "grok":
		return L("It needs a SuperGrok or X Premium+ subscription.")
	}
	return ""
}

// DefaultBaseURL is the API root the CLI calls unless the credential names another.
func DefaultBaseURL(kind ProviderKind) string {
	switch kind {
	case "deepseek":
		return "https://api.deepseek.com"
	case "anthropic":
		return "https://api.anthropic.com"
	case "opencode":
		return "https://opencode.ai/zen"
	case "opencode-go":
		return "https://opencode.ai/zen/go"
	}
	return ""
}

// KeyPlaceholder is the key field's placeholder, naming where the key comes from.
func KeyPlaceholder(kind ProviderKind) string {
	switch kind {
	case "deepseek":
		return L("sk-… from platform.deepseek.com")
	case "anthropic":
		return L("sk-ant-… from console.anthropic.com")
	case "opencode", "opencode-go":
		return L("API key from opencode.ai/auth")
	}
	return ""
}

// ConnectMethod is the CLI method that connects a built-in provider: underscores even where the
// stored kind has a hyphen.
func ConnectMethod(kind ProviderKind) string {
	return "providers.connect_" + strings.ReplaceAll(kind, "-", "_")
}

func ThinkingLabel(level string) string {
	switch level {
	case "off":
		return L("Off")
	case "minimal":
		return L("Minimal")
	case "low":
		return L("Low")
	case "medium":
		return L("Medium")
	case "high":
		return L("High")
	case "xhigh":
		return L("Extra high")
	case "max":
		return L("Max")
	}
	if level == "" {
		return level
	}
	return strings.ToUpper(level[:1]) + level[1:]
}

// ProviderModel is a model the CLI's catalog offers: its provider, id, and name, and the thinking
// levels it takes, lowest first.
type ProviderModel struct {
	Provider string
	ID       string
	Label    string
	Levels   []string
	// Decides is a decision model, which answers typed questions instead of chatting: Auto-review
	// can run it, and no bot can.
	Decides bool
}

// ProviderModels are the models a bot of a provider can run, in the catalog's order; the first is
// the default the CLI uses. Decision models are Auto-review's alone.
func ProviderModels(models []ProviderModel, kind ProviderKind) []ProviderModel {
	var out []ProviderModel
	for _, model := range ReviewModels(models, kind) {
		if !model.Decides {
			out = append(out, model)
		}
	}
	return out
}

// ReviewModels are every model of a provider Auto-review can run: the ones bots can, and
// decision models.
func ReviewModels(models []ProviderModel, kind ProviderKind) []ProviderModel {
	var out []ProviderModel
	for _, model := range models {
		if model.Provider == kind {
			out = append(out, model)
		}
	}
	return out
}

var allThinkingLevels = []string{"off", "minimal", "low", "medium", "high", "xhigh", "max"}

// Choice is one item of a picker: its id and what it says.
type Choice struct {
	ID    string
	Label string
}

// ThinkingLevels are the levels `model` takes, lowest first: the provider's default model's when
// it is empty, and every level the provider's models take for a model the catalog does not have.
// None on a bot means the model's default.
func ThinkingLevels(models []ProviderModel, kind ProviderKind, model string) []Choice {
	offered := ProviderModels(models, kind)
	want := model
	if want == "" && len(offered) > 0 {
		want = offered[0].ID
	}
	var ids []string
	found := false
	for _, each := range offered {
		if each.ID == want {
			ids, found = each.Levels, true
			break
		}
	}
	if !found {
		for _, id := range allThinkingLevels {
			for _, each := range offered {
				if slices.Contains(each.Levels, id) {
					ids = append(ids, id)
					break
				}
			}
		}
	}
	out := make([]Choice, 0, len(ids))
	for _, id := range ids {
		out = append(out, Choice{ID: id, Label: ThinkingLabel(id)})
	}
	return out
}

// WithCustomModels is the catalog with each custom provider's saved models after it, so the
// pickers offer them as they do the catalog's.
func WithCustomModels(models []ProviderModel, providers []ProviderCredential) []ProviderModel {
	out := slices.Clone(models)
	for _, provider := range providers {
		if !IsCustomKind(provider.Kind) {
			continue
		}
		for _, model := range provider.Models {
			label := model.Name
			if label == "" {
				label = model.ID
			}
			out = append(out, ProviderModel{Provider: provider.Kind, ID: model.ID, Label: label, Levels: model.Levels, Decides: provider.Decides()})
		}
	}
	return out
}

// ProviderCredential is one of the account's provider credentials, as statuses: never a key.
type ProviderCredential struct {
	Kind        ProviderKind
	IsConnected bool
	Detail      string
	// BaseURL is a custom API root, when the account credential has one.
	BaseURL string
	// A custom provider's name, the protocol its server speaks, and the models it offers.
	// Built-in providers have none.
	Name   string
	API    CustomAPI
	Models []CustomModel
	// ReviewModel is the model Auto-review runs on it unless the user picks another: the
	// catalog's small one for a built-in provider, a custom provider's first. Empty for none.
	ReviewModel string
}

// Decides is a custom provider of decision models, which Auto-review can run and no bot can.
func (p ProviderCredential) Decides() bool { return IsCustomKind(p.Kind) && p.API.Decides() }

// CustomAPI is the wire protocol a custom provider's server speaks.
type CustomAPI string

const (
	APIChatCompletions CustomAPI = "chat-completions"
	APIResponses       CustomAPI = "responses"
	APIMessages        CustomAPI = "messages"
	APISystemOne       CustomAPI = "system-one"
	APIDecisions       CustomAPI = "decisions"
)

// CustomAPIs are every protocol, in the order the custom provider sheet offers them.
var CustomAPIs = []CustomAPI{APIChatCompletions, APIResponses, APIMessages, APISystemOne, APIDecisions}

func IsCustomAPI(value string) bool { return slices.Contains(CustomAPIs, CustomAPI(value)) }

// Title is the product name, the same in every language.
func (api CustomAPI) Title() string {
	switch api {
	case APIChatCompletions:
		return "OpenAI Chat Completions"
	case APIResponses:
		return "OpenAI Responses"
	case APIMessages:
		return "Anthropic Messages"
	case APISystemOne:
		return "System One"
	case APIDecisions:
		return "OpenAI Decisions"
	}
	return string(api)
}

// Decides is a decision API, whose models answer typed questions instead of chatting: Auto-review
// can run them, and no bot can.
func (api CustomAPI) Decides() bool { return api == APISystemOne || api == APIDecisions }

// Path is what the CLI adds to the base URL for a model call.
func (api CustomAPI) Path() string {
	switch api {
	case APIResponses:
		return "/responses"
	case APIMessages:
		return "/messages"
	case APISystemOne:
		return "/systemone"
	case APIDecisions:
		return "/decisions"
	}
	return "/chat/completions"
}

func CustomBaseURLPlaceholder(api CustomAPI) string {
	return "https://api.example.com/v1"
}

// CustomEndpoint is the URL the CLI calls for a base URL as typed: a pasted endpoint is cut back
// to its root first, as the CLI does, then the API's path goes on, Messages' with the /v1 a root
// without one lacks. A decision API's URL is its
// endpoint, since vendors serve one at different paths: one that ends in a decision path stays as
// it is.
func CustomEndpoint(api CustomAPI, baseURL string) string {
	root := strings.TrimRight(strings.TrimSpace(baseURL), "/")
	if api.Decides() {
		if strings.HasSuffix(root, "/systemone") || strings.HasSuffix(root, "/decisions") {
			return root
		}
		return root + api.Path()
	}
	root = strings.TrimSuffix(root, api.Path())
	// Messages adds the /v1 a root without one lacks (Moonshot's …/anthropic).
	if api == APIMessages && !strings.HasSuffix(root, "/v1") {
		root += "/v1"
	}
	return root + api.Path()
}

// CustomEndpointNote is the note under the base URL field, naming the URL the CLI calls.
func CustomEndpointNote(api CustomAPI, baseURL string) string {
	if strings.TrimSpace(baseURL) == "" {
		return L("Lorca adds %@ to it.", api.Path())
	}
	return L("Requests go to %@.", CustomEndpoint(api, baseURL))
}

// CustomHost is the base URL's host: "api.example.com". Empty until the base URL has one.
func CustomHost(baseURL string) string {
	u, err := url.Parse(strings.TrimSpace(baseURL))
	if err != nil || u.Scheme == "" {
		return ""
	}
	return u.Hostname()
}

// IsUsableBaseURL is a base URL the sheet can ask a server at for its models: http or https,
// with a host.
func IsUsableBaseURL(baseURL string) bool {
	trimmed := strings.TrimSpace(baseURL)
	return (strings.HasPrefix(trimmed, "http://") || strings.HasPrefix(trimmed, "https://")) && CustomHost(trimmed) != ""
}

// CustomPreset is a server people often add, with what the sheet fills in for it.
type CustomPreset struct {
	// Name is a product name, the same in every language.
	Name    string
	API     CustomAPI
	BaseURL string
	// Local is a model server on the user's own network, which takes no key.
	Local bool
	// KeyPlaceholder names where the key comes from. Called while a view is built.
	KeyPlaceholder func() string
}

// CustomPresets are the chat servers Add Provider… offers, in its menu's order: hosted APIs, then
// servers on the user's network. Onboarding offers them too.
var CustomPresets = []CustomPreset{
	{"OpenAI", APIResponses, "https://api.openai.com/v1", false, func() string { return L("sk-… from platform.openai.com") }},
	{"OpenRouter", APIChatCompletions, "https://openrouter.ai/api/v1", false, func() string { return L("sk-or-… from openrouter.ai/keys") }},
	{"Gemini", APIChatCompletions, "https://generativelanguage.googleapis.com/v1beta/openai", false, func() string { return L("Key from aistudio.google.com") }},
	{"Groq", APIChatCompletions, "https://api.groq.com/openai/v1", false, func() string { return L("gsk_… from console.groq.com") }},
	{"Together AI", APIChatCompletions, "https://api.together.xyz/v1", false, func() string { return L("Key from api.together.ai") }},
	{"Ollama", APIChatCompletions, "http://localhost:11434/v1", true, func() string { return L("Optional for a server on your network") }},
	{"LM Studio", APIChatCompletions, "http://localhost:1234/v1", true, func() string { return L("Optional for a server on your network") }},
}

// DecisionPresets are decision APIs, whose models Auto-review can run: Add Provider… offers them
// after the chat servers, and onboarding, which picks what the first bot runs on, does not.
var DecisionPresets = []CustomPreset{
	{"OpenRouter Decisions", APISystemOne, "https://openrouter.ai/api/alpha/decisions", false, func() string { return L("sk-or-… from openrouter.ai/keys") }},
	{"OpenAI Decisions", APIDecisions, "https://api.openai.com/v1/decisions", false, func() string { return L("sk-… from platform.openai.com") }},
	{"TypeSafe", APISystemOne, "https://api.typesafe.ai/v1/systemone", false, func() string { return L("Key from typesafe.ai") }},
}

// MatchingPreset is the preset for a base URL as typed, by its host and port and, among a
// server's presets, its API ("" when none is known), so openrouter.ai with System One is
// OpenRouter Decisions and with Chat Completions OpenRouter.
func MatchingPreset(baseURL string, api CustomAPI) *CustomPreset {
	u, err := url.Parse(strings.TrimSpace(baseURL))
	if err != nil || u.Hostname() == "" {
		return nil
	}
	var server []*CustomPreset
	for _, presets := range [][]CustomPreset{CustomPresets, DecisionPresets} {
		for i := range presets {
			known, _ := url.Parse(presets[i].BaseURL)
			if known.Hostname() == u.Hostname() && known.Port() == u.Port() {
				server = append(server, &presets[i])
			}
		}
	}
	for _, preset := range server {
		if preset.API == api {
			return preset
		}
	}
	if len(server) > 0 {
		return server[0]
	}
	return nil
}

// SuggestedProviderName is what a provider left unnamed is saved as: the known server's name,
// else the base URL's host.
func SuggestedProviderName(baseURL string, api CustomAPI) string {
	if preset := MatchingPreset(baseURL, api); preset != nil {
		return preset.Name
	}
	return CustomHost(baseURL)
}

// PresetProvider is the custom provider the account has under a preset's name, in any case,
// which the preset's menu item opens instead of adding another.
func PresetProvider(preset CustomPreset, providers []ProviderCredential) *ProviderCredential {
	for i := range providers {
		if IsCustomKind(providers[i].Kind) && strings.EqualFold(providers[i].Name, preset.Name) {
			return &providers[i]
		}
	}
	return nil
}

// CustomModel is a model a custom provider offers, with what its server's model list says of it.
type CustomModel struct {
	ID            string
	Name          string
	ContextWindow int
	MaxOutput     int
	// Images is whether it takes images; nil when the server did not say.
	Images *bool
	// Levels are the thinking levels the CLI says it takes, lowest first: in a provider's status only.
	Levels []string
}

// DisplayName is the name a model goes by in the list: what its server calls it, else its id.
func (m CustomModel) DisplayName() string {
	if m.Name != "" {
		return m.Name
	}
	return m.ID
}

// ModelChecklist is the custom provider sheet's model list: every model it knows in list order,
// the ones bots can pick, the ids the user typed in, and the default.
type ModelChecklist struct {
	Models    []CustomModel
	Selected  map[string]bool
	Added     map[string]bool
	DefaultID string
}

// SavedChecklist is a provider's saved models, all picked, the first the default.
func SavedChecklist(models []CustomModel) ModelChecklist {
	checklist := ModelChecklist{Models: slices.Clone(models), Selected: map[string]bool{}, Added: map[string]bool{}}
	for _, model := range models {
		checklist.Selected[model.ID] = true
	}
	if len(models) > 0 {
		checklist.DefaultID = models[0].ID
	}
	return checklist
}

func cloneSet(set map[string]bool) map[string]bool {
	out := make(map[string]bool, len(set))
	for key, value := range set {
		if value {
			out[key] = true
		}
	}
	return out
}

// TakeListing takes a new listing: the models to keep (picked or added by hand) stay where they
// are with the listing's facts, the rest of the old listing goes, and the new one follows. With
// nothing picked yet, a list of eight models or fewer starts picked, its first the default.
func (c ModelChecklist) TakeListing(listed []CustomModel) ModelChecklist {
	facts := map[string]CustomModel{}
	for _, model := range listed {
		if _, ok := facts[model.ID]; !ok {
			facts[model.ID] = model
		}
	}
	var models []CustomModel
	seen := map[string]bool{}
	for _, model := range c.Models {
		if c.Selected[model.ID] || c.Added[model.ID] {
			if fact, ok := facts[model.ID]; ok {
				model = fact
			}
			models = append(models, model)
			seen[model.ID] = true
		}
	}
	for _, model := range listed {
		if seen[model.ID] {
			continue
		}
		seen[model.ID] = true
		models = append(models, model)
	}
	out := ModelChecklist{Models: models, Selected: cloneSet(c.Selected), Added: cloneSet(c.Added), DefaultID: c.DefaultID}
	if len(out.Selected) > 0 || len(listed) == 0 || len(listed) > 8 {
		return out
	}
	out.Selected = map[string]bool{}
	for _, model := range listed {
		out.Selected[model.ID] = true
	}
	out.DefaultID = listed[0].ID
	return out
}

// Toggle picks or unpicks a model. The first one picked becomes the default; unpicking the default
// makes the first picked one in the list the default.
func (c ModelChecklist) Toggle(id string) ModelChecklist {
	out := ModelChecklist{Models: c.Models, Selected: cloneSet(c.Selected), Added: c.Added, DefaultID: c.DefaultID}
	if out.Selected[id] {
		delete(out.Selected, id)
		if out.DefaultID == id {
			out.DefaultID = ""
			for _, model := range out.Models {
				if out.Selected[model.ID] {
					out.DefaultID = model.ID
					break
				}
			}
		}
	} else {
		out.Selected[id] = true
		if out.DefaultID == "" {
			out.DefaultID = id
		}
	}
	return out
}

// Add puts a model the server does not list at the top of the list, picked. It stays when another
// listing replaces the server's.
func (c ModelChecklist) Add(id string) ModelChecklist {
	models := c.Models
	if !slices.ContainsFunc(models, func(m CustomModel) bool { return m.ID == id }) {
		models = append([]CustomModel{{ID: id}}, models...)
	}
	out := ModelChecklist{Models: models, Selected: cloneSet(c.Selected), Added: cloneSet(c.Added), DefaultID: c.DefaultID}
	out.Selected[id] = true
	out.Added[id] = true
	if out.DefaultID == "" {
		out.DefaultID = id
	}
	return out
}

// OrderedIDs are the picked ids as the CLI keeps them: the default first, then the others in list
// order.
func (c ModelChecklist) OrderedIDs() []string {
	var picked []string
	for _, model := range c.Models {
		if c.Selected[model.ID] {
			picked = append(picked, model.ID)
		}
	}
	if c.DefaultID != "" && slices.Contains(picked, c.DefaultID) {
		out := []string{c.DefaultID}
		for _, id := range picked {
			if id != c.DefaultID {
				out = append(out, id)
			}
		}
		return out
	}
	return picked
}

// FilterModels are the models whose name or id holds the search text, in any case.
func FilterModels(models []CustomModel, query string) []CustomModel {
	text := strings.ToLower(strings.TrimSpace(query))
	if text == "" {
		return models
	}
	var out []CustomModel
	for _, model := range models {
		if strings.Contains(strings.ToLower(model.ID), text) || strings.Contains(strings.ToLower(model.Name), text) {
			out = append(out, model)
		}
	}
	return out
}

// AddCandidate is the id an Add row offers for the search text: one no model has yet.
func AddCandidate(query string, models []CustomModel) string {
	id := strings.TrimSpace(query)
	if id == "" || slices.ContainsFunc(models, func(m CustomModel) bool { return m.ID == id }) {
		return ""
	}
	return id
}

// MARK: - Device

// DeviceOS is a Device's system. `unknown` is a machine the relay lists that never said what it
// is: never a Runner.
type DeviceOS string

const (
	OSMacOS   DeviceOS = "macos"
	OSLinux   DeviceOS = "linux"
	OSWindows DeviceOS = "windows"
	OSIOS     DeviceOS = "ios"
	OSIPadOS  DeviceOS = "ipados"
	OSAndroid DeviceOS = "android"
	OSUnknown DeviceOS = "unknown"
)

func (os DeviceOS) DisplayName() string {
	switch os {
	case OSMacOS:
		return "macOS"
	case OSLinux:
		return "Linux"
	case OSWindows:
		return "Windows"
	case OSIOS:
		return "iOS"
	case OSIPadOS:
		return "iPadOS"
	case OSAndroid:
		return "Android"
	}
	return ""
}

// IsDesktop is whether the system runs the agent loop. Phones and tablets never do.
func (os DeviceOS) IsDesktop() bool { return os == OSMacOS || os == OSLinux || os == OSWindows }

type DeviceStatus string

const (
	StatusOnline  DeviceStatus = "online"
	StatusOffline DeviceStatus = "offline"
	StatusPairing DeviceStatus = "pairing"
)

func (s DeviceStatus) Label() string {
	switch s {
	case StatusOnline:
		return L("Online")
	case StatusPairing:
		return Lc("Pairing", "device state")
	}
	return L("Offline")
}

// Device is a paired machine or phone. Its OS decides whether it is a Runner: only desktop
// systems run the CLI and get bots assigned.
type Device struct {
	ID           string
	Name         string
	Model        string
	OS           DeviceOS
	OSVersion    string
	IsThisDevice bool
	Status       DeviceStatus
	LastSeen     time.Time
	MachineKey   string
	// Plugins installed on this Runner, as it advertises them. Secrets stay on the Runner.
	Plugins []InstalledPlugin
	// Version is the `lorca` this Device runs; empty when its CLI has not said.
	Version string
	// Update is how a CLI that replaces itself keeps current; nil where an app updates the CLI it
	// carries, and on phones.
	Update *DeviceUpdate
}

// DeviceUpdate is a self-updating CLI's updates, as its `machine` blob tells every Device.
type DeviceUpdate struct {
	// Auto installs a newer release by itself and restarts into it once no bot is at work there.
	Auto bool
	// Latest is the newest release, when it is newer than the Device's version.
	Latest string
	// State is "installing", "restarting", "installed", or "".
	State string
	// Error is why the last check or install failed, in the CLI's words.
	Error string
}

// IsRunner is derived from the OS alone: a desktop Device is a Runner and can be assigned bots.
func (d *Device) IsRunner() bool { return d.OS.IsDesktop() }

func (d *Device) RoleLabel() string {
	if d.IsRunner() {
		return L("Runner")
	}
	return L("Device")
}

// UnknownDeviceNote is what the Device panes say about a machine that never said what it is.
func UnknownDeviceNote() string {
	return L("This machine is paired to your account but has not sent its name or system. If you don't recognize it, unpair it.")
}

// Symbol is the symbol for a Device: its kind of machine.
func (d *Device) Symbol() string {
	switch d.OS {
	case OSMacOS:
		switch {
		case strings.Contains(d.Model, "MacBook"):
			return "laptopcomputer"
		case strings.Contains(d.Model, "Studio"):
			return "macstudio"
		case strings.Contains(d.Model, "mini"):
			return "macmini"
		}
		return "desktopcomputer"
	case OSLinux:
		return "server.rack"
	case OSWindows:
		return "pc"
	case OSIOS:
		return "iphone"
	case OSIPadOS:
		return "ipad"
	case OSAndroid:
		return "smartphone"
	}
	return "questionmark.circle"
}

// MARK: - Bot

type Accent string

// Accents in the order the Look sheet offers them.
var Accents = []Accent{"indigo", "blue", "teal", "green", "orange", "pink", "purple", "red"}

func IsAccent(value string) bool { return slices.Contains(Accents, Accent(value)) }

type Bot struct {
	ID string
	// Name and Description: what the bot does and how it should work.
	Name        string
	Description string
	SymbolName  string
	Accent      Accent
	RunnerID    string
	Provider    ProviderKind
	// Model is empty for the provider's default model.
	Model string
	// Thinking is empty for the provider's default.
	Thinking string
	// Avatar is a custom profile image, kept as a `file` blob like a message attachment. Shown in
	// place of the symbol and accent once this computer has the bytes.
	Avatar      *Attachment
	Permissions *BotPermissions
	CreatedAt   time.Time
}

// MARK: - Auto-review

// AutoReviewRule is one rule: what a bot wants to do, in the user's words, and whether that runs
// on its own ("allow") or asks first ("ask").
type AutoReviewRule struct {
	ID       string
	Text     string
	Behavior string
	// Tool is the exact plugin tool (`github/create_issue`) Always allow saved this rule for.
	Tool string
}

func BehaviorTitle(behavior string) string {
	if behavior == "allow" {
		return L("Allow automatically")
	}
	return L("Ask first")
}

// AutoReview is the check on effectful plugin actions and shell commands, shared by every Device
// through the roster: on, a model asks only when needed; off, each one asks. The model is the
// review model of the picked provider, else of the bot's provider.
type AutoReview struct {
	IsEnabled bool
	Rules     []AutoReviewRule
	// Provider is the provider that reviews; empty for the bot's own.
	Provider ProviderKind
	// Models are the review models the user picked, by provider; a provider without one reviews
	// with its default (ProviderCredential.ReviewModel).
	Models map[ProviderKind]string
}

// MARK: - Plugins

type PluginState string

const (
	PluginReady              PluginState = "ready"
	PluginNeedsSetup         PluginState = "needs_setup"
	PluginNeedsAuth          PluginState = "needs_auth"
	PluginInsufficientAccess PluginState = "insufficient_access"
	PluginConnecting         PluginState = "connecting"
	PluginError              PluginState = "error"
	PluginUnknown            PluginState = "unknown"
)

// InstalledPlugin is a plugin as its Runner advertises it: installed, and in what state.
type InstalledPlugin struct {
	ID          string
	Name        string
	Description string
	Version     string
	Icon        string
	State       PluginState
	Detail      string
	// Source is `mcp.json` for one of the Runner's own MCP servers, which the server sheet edits.
	Source string
	// ServiceID and AccountName are a named account's marketplace service (gmail) and the user's
	// name for it (Work). Its ID is the account's own.
	ServiceID   string
	AccountName string
}

func (p InstalledPlugin) MarketplaceID() string {
	if p.ServiceID != "" {
		return p.ServiceID
	}
	return p.ID
}

func (p InstalledPlugin) Symbol() string {
	if p.Icon != "" {
		return p.Icon
	}
	return "puzzlepiece.extension"
}

// IsMcpServer is a plugin that is one of the Runner's mcp.json servers.
func (p InstalledPlugin) IsMcpServer() bool { return p.Source == "mcp.json" }

// Tone is the role of a color a state takes: the views map it to the theme's colors.
type Tone int

const (
	ToneSecondary Tone = iota
	ToneTertiary
	ToneGreen
	ToneAccent
	ToneRed
	ToneOrange
)

// ShortStatus is the state in a word or two, for a row in a list; what it needs, in full, is in its
// sheet. It is colored only when the user has something to do.
func (p InstalledPlugin) ShortStatus() (string, Tone) {
	switch p.State {
	case PluginReady:
		return L("Connected"), ToneSecondary
	case PluginConnecting:
		return L("Connecting…"), ToneSecondary
	case PluginNeedsAuth:
		return L("Needs a sign-in"), ToneOrange
	case PluginInsufficientAccess:
		return L("Needs more access"), ToneOrange
	case PluginNeedsSetup:
		return L("Needs setup"), ToneOrange
	case PluginError:
		return L("Can’t connect"), ToneRed
	}
	return p.Detail, ToneSecondary
}

func (s PluginState) Tone() Tone {
	switch s {
	case PluginReady:
		return ToneGreen
	case PluginConnecting:
		return ToneAccent
	case PluginError:
		return ToneRed
	}
	return ToneOrange
}

type MarketplaceServer struct {
	Name string
	// Address is the URL of a remote server, or the command a local one runs on the Runner.
	Address  string
	IsRemote bool
	SignsIn  bool
}

type NamedText struct {
	Name        string
	Description string
}

type PluginVariable struct {
	Name        string
	Description string
	Secret      bool
	Required    bool
}

// MarketplacePlugin is a marketplace plugin, with the Runners that already have it.
type MarketplacePlugin struct {
	ID            string
	Name          string
	Description   string
	Icon          string
	Homepage      string
	Author        string
	Category      string
	IsFeatured    bool
	Tags          []string
	Servers       []MarketplaceServer
	Skills        []NamedText
	Variables     []PluginVariable
	InstalledOn   []string
	NamedAccounts bool
}

// SignsIn is whether at least one server signs in with OAuth on the Runner.
func (p MarketplacePlugin) SignsIn() bool {
	return slices.ContainsFunc(p.Servers, func(s MarketplaceServer) bool { return s.SignsIn })
}

type TemplateRoutine struct {
	Name         string
	ScheduleText string
	Prompt       string
}

// BotTemplate is a bot to add from the marketplace.
type BotTemplate struct {
	ID string
	// Name and Summary, one line for the marketplace's rows.
	Name    string
	Summary string
	// Description is what the bot does and how it should work: the new bot's description.
	Description string
	SymbolName  string
	Accent      Accent
	Category    string
	IsFeatured  bool
	Author      string
	Plugins     []string
	Routines    []TemplateRoutine
	Memory      []string
}

// Marketplace is what the marketplace offers, in the index's order.
type Marketplace struct {
	Plugins []MarketplacePlugin
	Bots    []BotTemplate
}

type PluginDetailVariable struct {
	Name        string
	Description string
	Secret      bool
	Required    bool
	IsSet       bool
	Value       string
}

type PluginDetailServer struct {
	Name     string
	Kind     string
	URL      string
	OAuth    bool
	SignedIn bool
	// Code and Link, while a device-flow sign-in waits: the code to enter, and the page to enter it on.
	Code string
	Link string
}

// PluginDetail is one installed plugin in full, as its Runner reports it: never a secret's value.
type PluginDetail struct {
	Status    InstalledPlugin
	Homepage  string
	Variables []PluginDetailVariable
	Servers   []PluginDetailServer
	Skills    []NamedText
}

// MARK: - Permission

type PermissionDecision string

const (
	DecisionPending   PermissionDecision = "pending"
	DecisionAllowed   PermissionDecision = "allowed"
	DecisionAlways    PermissionDecision = "always"
	DecisionDenied    PermissionDecision = "denied"
	DecisionExpired   PermissionDecision = "expired"
	DecisionDismissed PermissionDecision = "dismissed"
	DecisionConnected PermissionDecision = "connected"
	DecisionFailed    PermissionDecision = "failed"
)

// PermissionRequest is a bot asking before a plugin or shell action runs, or before a plugin is
// installed.
type PermissionRequest struct {
	PluginID   string
	PluginName string
	Tool       string
	Summary    string
	Decision   PermissionDecision
	// Link and Code: a sign-in mid-flow, where to go and the code to enter there.
	Link string
	Code string
	// Reason is why Auto-review paused the action, when it did.
	Reason string
	// Rule is the rule Always allow adds, which Auto-review proposed for a shell command.
	Rule    string
	HasRule bool
	// Command is a shell card's whole command, where Summary is its first line.
	Command string
}

// FullCommand is the command as the card and its sheet show it, without the summary's `$ ` prompt.
func (r *PermissionRequest) FullCommand() string {
	if r.Command != "" {
		return r.Command
	}
	return strings.TrimPrefix(r.Summary, "$ ")
}

func (r *PermissionRequest) IsPending() bool { return r.Decision == DecisionPending }
func (r *PermissionRequest) IsInstall() bool { return r.Tool == "install" }

// IsAccess is the bot's Access refusing a call: the card opens its Access sheet or is dismissed.
func (r *PermissionRequest) IsAccess() bool { return r.Tool == "access" }

// ShownSummary is the line under the title: an access request names a plugin's tool as it is,
// or what the bot wanted to do on its Runner in the CLI's English, which reads here in the app's
// language.
func (r *PermissionRequest) ShownSummary() string {
	if r.IsAccess() {
		switch r.Summary {
		case "Shell commands":
			return L("Shell commands")
		case "Changing files":
			return L("Changing files")
		case "Reading files":
			return L("Reading files")
		}
	}
	return r.Summary
}

// ShownReason is why the card asks: what Auto-review said, or for an access request, where it is
// turned on.
func (r *PermissionRequest) ShownReason() string {
	if r.IsAccess() {
		return L("Not allowed in this bot's Access settings.")
	}
	return r.Reason
}

// IsShell is a shell command on the bot's Runner.
func (r *PermissionRequest) IsShell() bool { return r.PluginID == "computer" && !r.IsAccess() }

// IsConnect is a sign-in card: Sign in starts the OAuth flow on the Runner.
func (r *PermissionRequest) IsConnect() bool { return r.Tool == "connect" }

// VerbPhrase is "wants to use GitHub" / "wants to install GitHub" / "needs a sign-in to GitHub" /
// "wants to run a command on Workbench".
func (r *PermissionRequest) VerbPhrase() string {
	switch {
	case r.IsAccess():
		return L("needs more access")
	case r.IsConnect():
		return L("needs a sign-in to %@", r.PluginName)
	case r.IsShell():
		return L("wants to run a command on %@", r.PluginName)
	case r.IsInstall():
		return L("wants to install %@", r.PluginName)
	}
	return L("wants to use %@", r.PluginName)
}

func (r *PermissionRequest) DecisionText() string {
	switch r.Decision {
	case DecisionPending:
		return L("Waiting for you")
	case DecisionAllowed:
		if r.IsConnect() {
			return L("Signing in")
		}
		return L("Allowed once")
	case DecisionAlways:
		return L("Always allowed")
	case DecisionDenied:
		if r.IsConnect() {
			return L("Not now")
		}
		return L("Denied")
	case DecisionExpired:
		return L("No answer in time")
	case DecisionDismissed:
		return L("Dismissed")
	case DecisionConnected:
		return L("Signed in")
	case DecisionFailed:
		return L("Sign-in failed")
	}
	return ""
}

// Answer is one button a question offers: its title and the decision it sends.
type Answer struct {
	Title    string
	Decision string
}

// Choices are the buttons a pending card offers. A shell command offers Always allow only with a
// rule to add.
func (r *PermissionRequest) Choices() []Answer {
	switch {
	case r.IsAccess():
		return []Answer{{L("Edit Access…"), "access"}, {L("Dismiss"), "deny"}}
	case r.IsConnect():
		return []Answer{{L("Sign in"), "allow"}, {L("Not now"), "deny"}}
	case r.IsInstall():
		return []Answer{{L("Allow"), "allow"}, {L("Deny"), "deny"}}
	case r.IsShell() && !r.HasRule:
		return []Answer{{L("Allow once"), "allow"}, {L("Deny"), "deny"}}
	}
	return []Answer{{L("Allow once"), "allow"}, {L("Always allow"), "always"}, {L("Deny"), "deny"}}
}

// MARK: - Memory

// BotMemory is a bot's memory as its Runner reports it: the curated index with its load budget,
// and the other files by name. Here is false when the bot runs elsewhere and only Runner is known.
type BotMemory struct {
	BotID     string
	Here      bool
	Runner    string
	Path      string
	Text      string
	Hash      string
	Lines     int
	Bytes     int
	Truncated bool
	MaxLines  int
	MaxBytes  int
	Topics    []string
	Logs      []string
}

// BudgetSummary is "12 lines · 1.2 KB of 24 KB", or "Empty".
func (m BotMemory) BudgetSummary() string {
	if m.Lines <= 0 {
		return L("Empty")
	}
	if m.Lines == 1 {
		return L("%d line · %@ of %@", m.Lines, Kilobytes(m.Bytes), Kilobytes(m.MaxBytes))
	}
	return L("%d lines · %@ of %@", m.Lines, Kilobytes(m.Bytes), Kilobytes(m.MaxBytes))
}

// FilesSummary is "2 topics · 5 days of logs".
func (m BotMemory) FilesSummary() string {
	var parts []string
	if n := len(m.Topics); n > 0 {
		if n == 1 {
			parts = append(parts, L("%d topic", 1))
		} else {
			parts = append(parts, L("%d topics", n))
		}
	}
	if n := len(m.Logs); n > 0 {
		if n == 1 {
			parts = append(parts, L("%d day of logs", 1))
		} else {
			parts = append(parts, L("%d days of logs", n))
		}
	}
	if len(parts) == 0 {
		return L("No other notes yet")
	}
	return strings.Join(parts, " · ")
}

// MARK: - Routine

// Routine is a recurring task a bot runs on a schedule in its direct chat, as the roster carries
// it. The schedule's words, the next run, and the running state come from the CLI.
type Routine struct {
	ID    string
	BotID string
	Name  string
	// Prompt is the task, written to the bot, handed to it on every run.
	Prompt string
	// Schedule is `every 30m`, `every 2h`, `every 1d`, or five cron fields in Timezone.
	Schedule string
	// Timezone is the IANA timezone a cron schedule reads in.
	Timezone string
	// MissedRunPolicy is what happens after due times its Runner missed: "coalesce" runs once
	// when it is back, "skip" waits for the next one.
	MissedRunPolicy string
	// State is how it stands, from the CLI: "on", "running", "paused", "blocked", "failed", or
	// "waiting_for_runner".
	State  string
	Health RoutineHealth
	// ScheduleText is the schedule in words: "Weekdays at 9:00 AM".
	ScheduleText string
	IsEnabled    bool
	// PausedReason is why Lorca paused it, when it did: "away", or "authentication" after three
	// failed sign-ins in a row.
	PausedReason string
	LastRunAt    time.Time
	// LastOutcome is how the last run ended: "sent", "pass", or "error".
	LastOutcome string
	NextRunAt   time.Time
	IsRunning   bool
	// Check is the script the Runner runs at each due time before the bot does.
	Check     string
	HasCheck  bool
	CreatedAt time.Time
}

// Detail is the line under the name in the inspector: the schedule, then what is going on.
func (r Routine) Detail() string {
	if r.IsRunning {
		return L("%@ · Running…", r.ScheduleText)
	}
	if problem := r.Problem(); problem != ProblemNone {
		return problem.Text() + " · " + r.ScheduleText
	}
	if !r.IsEnabled {
		if r.PausedReason == "away" {
			return L("%@ · Paused while you were away", r.ScheduleText)
		}
		return L("%@ · Paused", r.ScheduleText)
	}
	if !r.NextRunAt.IsZero() {
		next := Upcoming(r.NextRunAt)
		if !r.HasCheck {
			return L("%@ · Next %@", r.ScheduleText, next)
		}
		return L("%@ · Next check %@", r.ScheduleText, next)
	}
	return r.ScheduleText
}

// LastRunSummary is "Today 9:00 AM · replied", "Never", "Yesterday 6:00 PM · nothing to report".
func (r Routine) LastRunSummary() string {
	if r.LastRunAt.IsZero() {
		return L("Never")
	}
	when := DaySeparator(r.LastRunAt)
	switch r.LastOutcome {
	case "sent":
		return L("%@ · replied", when)
	case "pass":
		return L("%@ · nothing to report", when)
	case "error":
		return L("%@ · failed", when)
	}
	return when
}

// MARK: - Message

type CommandState string

const (
	CommandChecking  CommandState = "checking"
	CommandAsking    CommandState = "asking"
	CommandRunning   CommandState = "running"
	CommandWaiting   CommandState = "waiting"
	CommandExited    CommandState = "exited"
	CommandFailed    CommandState = "failed"
	CommandStopped   CommandState = "stopped"
	CommandDenied    CommandState = "denied"
	CommandExpired   CommandState = "expired"
	CommandDismissed CommandState = "dismissed"
)

// CommandRun is where a shell command stands: Auto-review checking it, the question it asks, the
// command running in its terminal, what the command asks, and that it ended.
type CommandRun struct {
	// SessionID is the terminal session running it, once one does. None before it starts, and on
	// a Windows Runner, where a command runs on pipes and takes no answers.
	SessionID string
	// Command is the command, its first 8,000 characters.
	Command string
	State   CommandState
	// Prompt is the line it asks with: "[sudo] password for ana:".
	Prompt string
	// Output is its last lines, as the bottom of a terminal shows them. Never what was typed.
	Output string
	// Device is the Runner it runs on, for the question: "Workbench".
	Device string
	// Reason is why Auto-review asked.
	Reason string
	// Rule is the rule Always allow adds.
	Rule    string
	HasRule bool
	// HandedOver is the bot leaving the command to the user.
	HandedOver bool
	// Background is a command that runs in the background. Stop in the chat leaves it running.
	Background bool
}

func (r *CommandRun) IsLive() bool { return r.State == CommandWaiting || r.State == CommandRunning }

// TakesInput is a command running in a session here or on its Runner: it takes answers and a Stop.
func (r *CommandRun) TakesInput() bool { return r.IsLive() && r.SessionID != "" }

// HasEnded is a command that ran and ended: by itself, with a nonzero code, or stopped.
func (r *CommandRun) HasEnded() bool {
	return r.State == CommandExited || r.State == CommandFailed || r.State == CommandStopped
}

// FirstLine is the command's first line with anything on it.
func FirstLine(command string) string {
	for _, line := range strings.Split(command, "\n") {
		if trimmed := strings.TrimSpace(line); trimmed != "" {
			return trimmed
		}
	}
	return command
}

// AsksYesOrNo is whether what the user types should show as they type it: a yes-or-no question.
// Anything else may be a secret.
func (r *CommandRun) AsksYesOrNo() bool {
	prompt := strings.ToLower(r.Prompt)
	if prompt == "" {
		return false
	}
	return strings.Contains(prompt, "[y/n]") || strings.Contains(prompt, "(y/n)") || strings.Contains(prompt, "(yes/no")
}

// Choices are the buttons the question offers. Always allow only with a rule to add.
func (r *CommandRun) Choices() []Answer {
	if !r.HasRule {
		return []Answer{{L("Allow once"), "allow"}, {L("Deny"), "deny"}}
	}
	return []Answer{{L("Allow once"), "allow"}, {L("Always allow"), "always"}, {L("Deny"), "deny"}}
}

type ToolInvocation struct {
	Name      string
	Summary   string
	Detail    string
	IsRunning bool
	// Description is what the call does, in the bot's words: a shell command's "Install dependencies".
	Description string
	// TargetBotID is the bot a message_bot call goes to.
	TargetBotID string
	// ScriptCommand is a codemode script's latest command, by its description.
	ScriptCommand string
	// Run is a shell command's card, which the transcript shows only while the command needs the
	// user (IsShown). Every `bash` row has one.
	Run *CommandRun
}

// IsSentMessage is a finished message_bot call: the one tool the transcript shows, as
// "Messaged ◉ Name".
func (t *ToolInvocation) IsSentMessage() bool {
	return t.Name == "message_bot" && !t.IsRunning && strings.HasPrefix(t.Summary, "Messaged ")
}

// IsShown is whether the transcript shows the row: a sent message's marker, or a command's card
// while the command needs the user.
func (t *ToolInvocation) IsShown() bool {
	run := t.Run
	if run == nil {
		return t.IsSentMessage()
	}
	return run.State == CommandAsking || (run.IsLive() && run.HandedOver)
}

// Attachment is a file sent with a message. The bytes live under `~/.lorca/files/<id>` once this
// Device has them; Width and Height size an image's thumbnail before the file arrives.
type Attachment struct {
	ID     string
	Name   string
	Mime   string
	Size   int64
	Width  int
	Height int
}

func (a Attachment) IsImage() bool { return strings.HasPrefix(a.Mime, "image/") }

// AttachmentSummary is "Photo", "3 photos", "report.pdf", "2 files": the preview of a message
// with no text.
func AttachmentSummary(attachments []Attachment) string {
	if len(attachments) == 0 {
		return ""
	}
	if len(attachments) == 1 {
		if attachments[0].IsImage() {
			return L("Photo")
		}
		return attachments[0].Name
	}
	for _, attachment := range attachments {
		if !attachment.IsImage() {
			return L("%d files", len(attachments))
		}
	}
	return L("%d photos", len(attachments))
}

func SizeText(bytes int64) string {
	switch {
	case bytes < 1024:
		return fmt.Sprintf("%d B", bytes)
	case bytes < 1024*1024:
		return fmt.Sprintf("%d KB", int64(math.Round(float64(bytes)/1024)))
	}
	return fmt.Sprintf("%.1f MB", float64(bytes)/1024/1024)
}

type AuthorKind int

const (
	AuthorYou AuthorKind = iota
	AuthorBot
	AuthorSystem
)

type Author struct {
	Kind  AuthorKind
	BotID string
}

var (
	You    = Author{Kind: AuthorYou}
	System = Author{Kind: AuthorSystem}
)

func BotAuthor(id string) Author { return Author{Kind: AuthorBot, BotID: id} }

// Same is whether two authors are one: the user, the system, or the same bot.
func (a Author) Same(b Author) bool {
	return a.Kind == b.Kind && (a.Kind != AuthorBot || a.BotID == b.BotID)
}

type BodyKind int

const (
	BodyText BodyKind = iota
	BodyTool
	BodyHandoff
	BodyNotice
	BodyPermission
)

type Handoff struct {
	From   string
	To     string
	Reason string
}

// Body is what a message says: text, a tool call, a handoff, a notice, or a question.
type Body struct {
	Kind BodyKind
	// Text of a text body or a notice.
	Text    string
	Tool    *ToolInvocation
	Handoff Handoff
	Request *PermissionRequest
}

type StateKind int

const (
	StateComplete StateKind = iota
	StateThinking
	StateStreaming
	StateFailed
)

type MessageState struct {
	Kind  StateKind
	Error string
}

type Message struct {
	ID        string
	Author    Author
	Body      Body
	State     MessageState
	CreatedAt time.Time
	// Attachments are the files sent with a text body; other bodies carry none.
	Attachments []Attachment
	// ReplyTo is the message the user answers with this one, quoted.
	ReplyTo *ReplyQuote
	// Queued is a message of the user's the bot's turn holds for its next step; Send now has it
	// read now.
	Queued bool
	// Output identifies an immutable published deliverable/evidence version.
	Output *Output
	// Notification is how a coordinator's brief, an urgent report, or a quiet check alerts.
	Notification NotificationTag
}

// ReplyQuote is a message quoted by the user's reply: who wrote it and how it opens, as the CLI
// keeps it with the reply.
type ReplyQuote struct {
	MessageID string
	Author    Author
	Text      string
}

// CanBeQuoted is a finished text message, the user's or a bot's, which a reply can answer.
func (m *Message) CanBeQuoted() bool {
	return m.Body.Kind == BodyText && m.State.Kind == StateComplete && m.Author.Kind != AuthorSystem
}

// QuoteOf is the quote of `m` the CLI makes, for a reply it has not confirmed yet: its words
// without the Markdown, on one line, or the names of its files.
func QuoteOf(m *Message) *ReplyQuote {
	if !m.CanBeQuoted() {
		return nil
	}
	line := strings.Join(strings.FieldsFunc(PlainText(MarkdownBlocks(m.Body.Text)), unicode.IsSpace), " ")
	if line == "" {
		names := make([]string, 0, len(m.Attachments))
		for _, attachment := range m.Attachments {
			names = append(names, attachment.Name)
		}
		line = strings.Join(names, ", ")
	}
	if runes := []rune(line); len(runes) > 280 {
		line = strings.TrimRightFunc(string(runes[:280]), unicode.IsSpace) + "…"
	}
	return &ReplyQuote{MessageID: m.ID, Author: m.Author, Text: line}
}

func randomHex(bytes int) string {
	raw := make([]byte, bytes)
	_, _ = rand.Read(raw)
	return hex.EncodeToString(raw)
}

func uuid() string {
	raw := make([]byte, 16)
	_, _ = rand.Read(raw)
	raw[6] = raw[6]&0x0f | 0x40
	raw[8] = raw[8]&0x3f | 0x80
	h := hex.EncodeToString(raw)
	return h[0:8] + "-" + h[8:12] + "-" + h[12:16] + "-" + h[16:20] + "-" + h[20:]
}

func NewMessageID() string { return "msg-" + uuid() }

// ShortID is a prefix and the first characters of a new UUID: "chat-1f2e3d4c".
func ShortID(prefix string) string { return prefix + "-" + uuid()[:8] }

// NewAttachmentID is "att-" and twelve hex digits.
func NewAttachmentID() string { return "att-" + randomHex(6) }

// CommandRunOf is a `bash` row's command.
func CommandRunOf(m *Message) *CommandRun {
	if m.Body.Kind == BodyTool && m.Body.Tool != nil {
		return m.Body.Tool.Run
	}
	return nil
}

// RunsInForeground is a command running in its terminal that the bot's call still waits on: Run
// in Background sends it there, and the call returns.
func RunsInForeground(m *Message) bool {
	if m.Body.Kind != BodyTool || m.Body.Tool == nil {
		return false
	}
	run := m.Body.Tool.Run
	return m.Body.Tool.IsRunning && run != nil && run.TakesInput() && !run.Background
}

// MessageText is a message's words, whatever its body.
func MessageText(m *Message) string {
	switch m.Body.Kind {
	case BodyText, BodyNotice:
		return m.Body.Text
	case BodyTool:
		return m.Body.Tool.Summary
	case BodyHandoff:
		return m.Body.Handoff.Reason
	case BodyPermission:
		return m.Body.Request.Summary
	}
	return ""
}

// MARK: - Chat

const MaxGroupBots = 6

type ChatKind string

const (
	ChatDM    ChatKind = "dm"
	ChatGroup ChatKind = "group"
)

// Chat is a DM, a fixed one-to-one thread with a single bot, or a group of one to six bots that
// can gain or lose members after it is created.
type Chat struct {
	ID          string
	Kind        ChatKind
	CustomTitle string
	BotIDs      []string
	Messages    []*Message
	UnreadCount int
	IsPinned    bool
	CreatedAt   time.Time
	// Usage is what the turns run in this chat used, from the Runner that ran them.
	Usage *ChatUsage
	// HasMore is the CLI holding messages older than the ones here; the transcript asks for them
	// by page.
	HasMore bool
	// OwnerBotID is the group member holding the work, as the CLI last said.
	OwnerBotID string
	// GroupDescription is what a group is for, which every member reads in its system prompt.
	GroupDescription string
}

func (c *Chat) IsGroup() bool { return c.Kind == ChatGroup }
func (c *Chat) IsDM() bool    { return c.Kind == ChatDM }

// CanAddBot is whether another bot may join. Only groups grow, and never past the cap.
func (c *Chat) CanAddBot() bool { return c.IsGroup() && len(c.BotIDs) < MaxGroupBots }

// CanRemoveBot is whether a bot may leave. Groups keep at least one bot; DMs never change.
func (c *Chat) CanRemoveBot() bool { return c.IsGroup() && len(c.BotIDs) > 1 }

// Owner is a group's owner: the one set, else the first member, as the CLI picks.
func (c *Chat) Owner() string {
	if !c.IsGroup() {
		return ""
	}
	if c.OwnerBotID != "" && slices.Contains(c.BotIDs, c.OwnerBotID) {
		return c.OwnerBotID
	}
	if len(c.BotIDs) > 0 {
		return c.BotIDs[0]
	}
	return ""
}

func (c *Chat) LastActivity() time.Time {
	if n := len(c.Messages); n > 0 {
		return c.Messages[n-1].CreatedAt
	}
	return c.CreatedAt
}

// Message finds one of the chat's loaded messages.
func (c *Chat) Message(id string) *Message {
	for _, message := range c.Messages {
		if message.ID == id {
			return message
		}
	}
	return nil
}

// MARK: - Usage

// ChatUsage is the tokens and money the turns in a chat used. ContextTokens and ContextWindow are
// the last turn's; the rest accumulate.
type ChatUsage struct {
	ContextTokens           int
	ContextWindow           int
	InputTokens             int
	OutputTokens            int
	CacheReadTokens         int
	CostUSD                 float64
	Turns                   int
	Model                   string
	APICostUSD              float64
	SubscriptionEstimateUSD float64
	UnknownPriceCalls       uint64
	PricingKinds            []string
}

// ContextSummary is "128k of 1M · 13%", or "128k" when the window is unknown.
func (u ChatUsage) ContextSummary() string {
	if u.ContextWindow <= 0 {
		return Tokens(u.ContextTokens)
	}
	percent := int(math.Round(float64(u.ContextTokens) / float64(u.ContextWindow) * 100))
	return L("%@ of %@ · %d%%", Tokens(u.ContextTokens), Tokens(u.ContextWindow), percent)
}

// SpendSummary is "$0.42 · 18 turns". A subscription's turns cost what the plan costs, so their
// price at API rates reads as an estimate ("$0.42 est."); a model without a known price reads
// Price unknown, never $0.00.
func (u ChatUsage) SpendSummary() string {
	count := L("%d turns", u.Turns)
	if u.Turns == 1 {
		count = L("1 turn")
	}
	var parts []string
	if slices.Contains(u.PricingKinds, "api") {
		parts = append(parts, Dollars(u.APICostUSD))
	}
	if slices.Contains(u.PricingKinds, "subscription_estimate") {
		parts = append(parts, L("%@ est.", Dollars(u.SubscriptionEstimateUSD)))
	}
	switch {
	case len(parts) == 0:
		parts = append(parts, L("Price unknown"))
	case u.UnknownPriceCalls > 0:
		parts = append(parts, Lc("unknown", "price"))
	}
	return strings.Join(parts, " + ") + " · " + count
}

// SpendNote is what the Spent row's estimate or unknown price means, for its tooltip.
func (u ChatUsage) SpendNote() string {
	if slices.Contains(u.PricingKinds, "subscription_estimate") {
		return L("An estimate of what these turns would cost at API prices. Your subscription covers them.")
	}
	if u.UnknownPriceCalls > 0 || len(u.PricingKinds) == 0 {
		return L("This model has no known price.")
	}
	return ""
}

// MARK: - Settings

type SettingsPane string

const (
	PaneGeneral    SettingsPane = "general"
	PaneProviders  SettingsPane = "providers"
	PaneAutoReview SettingsPane = "auto-review"
	PanePlugins    SettingsPane = "plugins"
	PaneBots       SettingsPane = "bots"
	PaneDevice     SettingsPane = "device"
	PaneAdvanced   SettingsPane = "advanced"
)

var SettingsPanes = []SettingsPane{PaneGeneral, PaneProviders, PaneAutoReview, PanePlugins, PaneBots, PaneDevice, PaneAdvanced}

// IsDeviceScoped is a pane that shows one Device, picked at the top of the page: bots and plugins
// live on a Runner. The others hold this computer's settings and the account's.
func (p SettingsPane) IsDeviceScoped() bool {
	return p == PaneBots || p == PanePlugins || p == PaneDevice
}

func (p SettingsPane) Title() string {
	switch p {
	case PaneGeneral:
		return L("General")
	case PaneAutoReview:
		return L("Auto-review")
	case PaneAdvanced:
		return L("Advanced")
	case PaneBots:
		return L("Bots")
	case PaneProviders:
		return L("Providers")
	case PanePlugins:
		return L("Plugins")
	case PaneDevice:
		return L("Devices")
	}
	return string(p)
}

func (p SettingsPane) Symbol() string {
	switch p {
	case PaneGeneral:
		return "gearshape"
	case PaneAutoReview:
		return "checkmark.shield"
	case PaneAdvanced:
		return "slider.horizontal.3"
	case PaneBots:
		return "person.2"
	case PaneProviders:
		return "key"
	case PanePlugins:
		return "puzzlepiece.extension"
	case PaneDevice:
		return "desktopcomputer"
	}
	return "gearshape"
}

// Selection is what the main window shows: a chat, or a settings pane.
type Selection struct {
	ChatID string
	Pane   SettingsPane
}

func (s Selection) IsChat() bool     { return s.ChatID != "" }
func (s Selection) IsSettings() bool { return s.Pane != "" }
func (s Selection) IsNone() bool     { return s.ChatID == "" && s.Pane == "" }

// Encode is "chat:<id>", "settings:<pane>", or "".
func (s Selection) Encode() string {
	switch {
	case s.ChatID != "":
		return "chat:" + s.ChatID
	case s.Pane != "":
		return "settings:" + string(s.Pane)
	}
	return ""
}

func DecodeSelection(raw string) Selection {
	kind, value, ok := strings.Cut(raw, ":")
	if !ok || value == "" {
		return Selection{}
	}
	switch kind {
	case "chat":
		return Selection{ChatID: value}
	case "settings":
		if slices.Contains(SettingsPanes, SettingsPane(value)) {
			return Selection{Pane: SettingsPane(value)}
		}
	}
	return Selection{}
}
