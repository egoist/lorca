package main

import (
	"runtime"
	"strings"
	"unicode"

	"github.com/egoist/lorca/desktop/model"
	"golang.org/x/text/runes"
	"golang.org/x/text/transform"
	"golang.org/x/text/unicode/norm"
)

// One searchable setting, after the macOS app's SettingsSearch. The panes label their rows from
// these, so the search index and the pages share one spelling.

type settingsEntry struct {
	pane model.SettingsPane
	// title is what the results list shows.
	title string
	// row is the row's label on the pane, or a section's title. The pane finds the row by it.
	row string
	// keywords are words that find the setting besides its title.
	keywords []string
}

func entry(pane model.SettingsPane, title, row string, keywords ...string) settingsEntry {
	if row == "" {
		row = title
	}
	return settingsEntry{pane: pane, title: title, row: row, keywords: keywords}
}

// The entries, each built when a view is, in the language in force.

// sendOnReturnEntry: the key is Return on a Mac keyboard and Enter on the others.
func sendOnReturnEntry() settingsEntry {
	title := L("Enter sends the message")
	if runtime.GOOS == "darwin" {
		title = L("Return sends the message")
	}
	return entry(model.PaneGeneral, title, "", L("enter send newline keyboard chats"))
}

func timestampsEntry() settingsEntry {
	return entry(model.PaneGeneral, L("Show timestamps in transcripts"), "", L("time date messages chats"))
}

func appearanceEntry() settingsEntry {
	return entry(model.PaneGeneral, L("Appearance"), "", L("theme dark mode light mode system"))
}

func appLanguageEntry() settingsEntry {
	return entry(model.PaneGeneral, L("App Language"), "", L("language locale english chinese translation"))
}

func versionEntry() settingsEntry {
	return entry(model.PaneGeneral, L("Check for Updates"), L("Version"), L("update upgrade release"))
}

func automaticChecksEntry() settingsEntry {
	return entry(model.PaneGeneral, L("Check for updates automatically"), "", L("update upgrade background"))
}

func automaticDownloadsEntry() settingsEntry {
	return entry(model.PaneGeneral, L("Download and install updates automatically"), "", L("update upgrade background"))
}

func autoReviewSwitchEntry() settingsEntry {
	return entry(model.PaneAutoReview, L("Check actions before they run"), "", L("auto-review approve approval permission plugin ask"))
}

func autoReviewRulesEntry() settingsEntry {
	return entry(model.PaneAutoReview, L("Auto-review Rules"), "", L("rule allow automatically ask first always allow"))
}

func relayURLEntry() settingsEntry {
	return entry(model.PaneAdvanced, L("Relay URL"), "", L("server self-host sync pairing connection"))
}

func cliPortEntry() settingsEntry {
	return entry(model.PaneAdvanced, L("CLI port"), "", L("localhost 127.0.0.1 serve connection"))
}

func onboardingEntry() settingsEntry {
	return entry(model.PaneAdvanced, L("Onboarding"), "", L("show onboarding again setup welcome restore"))
}

func deleteAccountEntry() settingsEntry {
	return entry(model.PaneAdvanced, L("Delete Account"), L("Account"), L("erase remove wipe relay data identity"))
}

func machineKeyEntry() settingsEntry {
	return entry(model.PaneDevice, L("Machine key"), "", L("device os role runner last seen relay"))
}

func pairingEntry() settingsEntry {
	return entry(model.PaneDevice, L("Pairing"), "", L("unpair remove device paired"))
}

func botEntry(bot *model.Bot) settingsEntry {
	return entry(model.PaneBots, bot.Name, "", bot.Description, model.ProviderName(bot.Provider, store.Providers), L("bot runner"))
}

func pluginEntry(plugin model.InstalledPlugin) settingsEntry {
	return entry(model.PanePlugins, plugin.Name, "", plugin.Description, L("plugin mcp marketplace"))
}

// mcpServersEntry is the section a Runner's mcp.json fills; each server in it goes by its row.
func mcpServersEntry(device *model.Device) settingsEntry {
	return entry(model.PanePlugins, L("MCP Servers"), L("MCP Servers on %@", device.Name), L("mcp model context protocol server mcp.json custom command url json claude cursor"))
}

func mcpServerEntry(plugin model.InstalledPlugin) settingsEntry {
	return entry(model.PanePlugins, plugin.Name, "", plugin.Description, L("mcp server mcp.json"))
}

// providerEntry: a custom provider goes by the name the user gave it, as its row on the pane does.
func providerEntry(kind model.ProviderKind) settingsEntry {
	return entry(model.PaneProviders, model.ProviderName(kind, store.Providers), "", model.ProviderSubtitle(kind), L("credential connect disconnect sign in model"))
}

var foldAccents = transform.Chain(norm.NFD, runes.Remove(runes.In(unicode.Mn)), norm.NFC)

func fold(text string) string {
	folded, _, err := transform.String(foldAccents, text)
	if err != nil {
		folded = text
	}
	return strings.ToLower(folded)
}

// standardContains is whether `text` holds `query` the way a search field matches: ignoring case
// and accents.
func standardContains(text, query string) bool {
	return strings.Contains(fold(text), fold(query))
}

func (s settingsEntry) matches(query string) bool {
	if standardContains(s.title, query) {
		return true
	}
	for _, keyword := range s.keywords {
		if standardContains(keyword, query) {
			return true
		}
	}
	return false
}

func entriesIn(pane model.SettingsPane, device *model.Device) []settingsEntry {
	switch pane {
	case model.PaneGeneral:
		out := []settingsEntry{sendOnReturnEntry(), timestampsEntry(), appearanceEntry(), appLanguageEntry()}
		if updatesEnabled() {
			out = append(out, versionEntry(), automaticChecksEntry(), automaticDownloadsEntry())
		}
		return out
	case model.PaneAutoReview:
		return []settingsEntry{autoReviewSwitchEntry(), autoReviewRulesEntry()}
	case model.PaneAdvanced:
		out := []settingsEntry{relayURLEntry(), cliPortEntry(), onboardingEntry()}
		if store.HasIdentity != nil && *store.HasIdentity {
			out = append(out, deleteAccountEntry())
		}
		return out
	case model.PaneBots:
		var out []settingsEntry
		if device != nil {
			for _, bot := range store.BotsOn(device.ID) {
				out = append(out, botEntry(bot))
			}
		}
		return out
	case model.PaneProviders:
		var out []settingsEntry
		for _, credential := range store.Providers {
			out = append(out, providerEntry(credential.Kind))
		}
		return out
	case model.PanePlugins:
		var out, servers []settingsEntry
		if device != nil {
			for _, plugin := range device.Plugins {
				if !plugin.IsMcpServer() {
					out = append(out, pluginEntry(plugin))
				}
			}
			if device.IsRunner() {
				servers = append(servers, mcpServersEntry(device))
				for _, plugin := range device.Plugins {
					if plugin.IsMcpServer() {
						servers = append(servers, mcpServerEntry(plugin))
					}
				}
			}
		}
		return append(out, servers...)
	case model.PaneDevice:
		if device != nil {
			return []settingsEntry{machineKeyEntry(), pairingEntry()}
		}
		return []settingsEntry{machineKeyEntry()}
	}
	return nil
}

type paneMatch struct {
	pane    model.SettingsPane
	entries []settingsEntry
}

// panesMatching is each pane that matches by name or holds a matching setting, and those settings
// under it. The Device panes answer for the picked Device.
func panesMatching(query string, device *model.Device) []paneMatch {
	var out []paneMatch
	for _, pane := range model.SettingsPanes {
		var entries []settingsEntry
		for _, setting := range entriesIn(pane, device) {
			if setting.matches(query) {
				entries = append(entries, setting)
			}
		}
		if len(entries) == 0 && !standardContains(pane.Title(), query) {
			continue
		}
		out = append(out, paneMatch{pane, entries})
	}
	return out
}

// revealedSetting is a setting a search picked: its pane scrolls to its row and flashes it.
type revealedSetting struct {
	entry settingsEntry
	// shown is its pane having taken it up, so it scrolls and flashes once.
	shown bool
}
