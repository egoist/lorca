package main

import (
	"strconv"
	"strings"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The marketplace's pages, after the macOS app's MarketplacePages and MarketplaceDetailPages: the
// home page, a full list, the plugins the Runner has, a plugin in full, and a bot in full.

// MARK: - Home

// marketHomeSection is one of the home page's sections: its first rows, and View all when there is
// more.
type marketHomeSection struct {
	title   string
	items   []marketItem
	viewAll func()
}

func (mk *marketplace) homeSection(title string, items []marketItem, all func(*model.Marketplace) []marketItem, listTitle string) marketHomeSection {
	shown := min(len(items), marketPreviewCount)
	section := marketHomeSection{title: title, items: items[:shown]}
	if len(all(&mk.catalog)) > shown {
		heading := firstNonEmpty(listTitle, title)
		section.viewAll = func() { mk.show(&marketPage{kind: marketListPage, title: heading, items: all}) }
	}
	return section
}

// homeSections are Featured Plugins, Featured Bots, then a section per category, plugins before
// bots. A featured section's View all lists every plugin or every bot, the featured ones first. A
// category leads with what the featured sections do not already show, and one with both kinds
// keeps half its rows for bots, so its plugins never hide them.
func (mk *marketplace) homeSections() []marketHomeSection {
	catalog := &mk.catalog
	featuredPlugin := func(p *model.MarketplacePlugin) bool { return p.IsFeatured }
	otherPlugin := func(p *model.MarketplacePlugin) bool { return !p.IsFeatured }
	featuredBot := func(b *model.BotTemplate) bool { return b.IsFeatured }
	otherBot := func(b *model.BotTemplate) bool { return !b.IsFeatured }

	var sections []marketHomeSection
	featuredPlugins := marketPluginItems(catalog.Plugins, featuredPlugin)
	if len(featuredPlugins) > 0 {
		sections = append(sections, mk.homeSection(L("Featured Plugins"), featuredPlugins, func(all *model.Marketplace) []marketItem {
			return append(marketPluginItems(all.Plugins, featuredPlugin), marketPluginItems(all.Plugins, otherPlugin)...)
		}, L("Plugins")))
	}
	featuredBots := marketBotItems(catalog.Bots, featuredBot)
	if len(featuredBots) > 0 {
		sections = append(sections, mk.homeSection(L("Featured Bots"), featuredBots, func(all *model.Marketplace) []marketItem {
			return append(marketBotItems(all.Bots, featuredBot), marketBotItems(all.Bots, otherBot)...)
		}, L("Bots")))
	}
	shown := map[string]bool{}
	for _, item := range featuredPlugins[:min(len(featuredPlugins), marketPreviewCount)] {
		shown[item.key()] = true
	}
	for _, item := range featuredBots[:min(len(featuredBots), marketPreviewCount)] {
		shown[item.key()] = true
	}
	// fresh puts what the featured sections do not show first.
	fresh := func(items []marketItem) []marketItem {
		var first, last []marketItem
		for _, item := range items {
			if shown[item.key()] {
				last = append(last, item)
			} else {
				first = append(first, item)
			}
		}
		return append(first, last...)
	}
	for _, key := range marketCategoryKeys(marketAllItems(catalog)) {
		plugins := fresh(marketPluginItems(catalog.Plugins, func(p *model.MarketplacePlugin) bool { return p.Category == key }))
		bots := fresh(marketBotItems(catalog.Bots, func(b *model.BotTemplate) bool { return b.Category == key }))
		botRows := min(len(bots), max(marketPreviewCount-len(plugins), marketPreviewCount/2))
		preview := append(plugins[:min(len(plugins), marketPreviewCount-botRows)], bots[:botRows]...)
		sections = append(sections, mk.homeSection(marketCategoryTitle(key), preview, func(all *model.Marketplace) []marketItem {
			var out []marketItem
			for _, item := range marketAllItems(all) {
				if item.category() == key {
					out = append(out, item)
				}
			}
			return out
		}, ""))
	}
	return sections
}

// homePage is the first page: its title with what the Runner has installed, the search, and then
// the featured plugins and bots and each category, or what the search found.
func (mk *marketplace) homePage(c *ui.Context, page *marketPage) {
	on := mk.runner()
	ui.Row(c).Height(30).Gap(12).Padding(0, 4, 0, 12).Margin(0, 0, -10, 0).Justify(ui.SpaceBetween).Children(func() {
		ui.Text(c, L("Marketplace")).FontSize(20).FontWeight(600).SingleLine().Shrink(1)
		if on != nil {
			installed := hoverButton(c, hoverButtonOptions{
				Title:    L("%d installed", len(mk.installedPlugins())),
				Trailing: "chevron.right",
				Tooltip:  L("Plugins on %@", on.Name),
			})
			if installed.Clicked() {
				mk.show(&marketPage{kind: marketInstalledPage})
			}
		}
	})
	search := marketSearchField(c, &page.query, L("Search plugins and bots"))
	if page.focusSearch {
		search.Focus()
		page.focusSearch = false
	}

	ui.Column(c).Gap(24).Margin(2, 0, 0, 0).Children(func() {
		if len(mk.catalog.Plugins)+len(mk.catalog.Bots)+len(mk.catalog.Packs) == 0 && mk.loading != marketLoaded {
			if mk.loading != marketFailed {
				marketStatusLine(c, L("Loading the marketplace…"))
				return
			}
			ui.Column(c).Gap(10).Children(func() {
				marketStatusLine(c, L("The marketplace isn't available right now."))
				ui.Row(c).Padding(0, 12).Children(func() {
					if pushButton(c, L("Try Again"), pushOptions{}).Clicked() {
						mk.load()
					}
				})
			})
			return
		}
		query := strings.TrimSpace(page.query)
		packs := workflowMatchingPacks(mk.catalog.Packs, query)
		if len(packs) > 0 {
			mk.workflowSection(c, packs)
		}
		if query == "" {
			sections := mk.homeSections()
			if len(sections) == 0 && len(packs) == 0 {
				marketStatusLine(c, L("Nothing in the marketplace yet."))
			}
			for _, each := range sections {
				mk.section(c, each.title, each.items, nil, each.viewAll)
			}
			return
		}
		words := strings.Fields(query)
		var results []marketItem
		for _, item := range marketAllItems(&mk.catalog) {
			if marketMatches(item, words) {
				results = append(results, item)
			}
		}
		if len(results) == 0 && len(packs) == 0 {
			marketStatusLine(c, L("No results match “%@”", query))
			return
		}
		if len(results) == 0 {
			return
		}
		shown, mixed := marketFiltered(results, page.filter)
		var control func()
		if mixed {
			control = func() { marketKindFilter(c, &page.filter) }
		}
		mk.section(c, L("Results"), shown, control, nil)
	})
}

// marketSearchField is the home page's large capsule search field. Escape leaves its text alone and
// closes the sheet, as it does from anywhere in it.
func marketSearchField(c *ui.Context, value *string, placeholder string) ui.Element {
	p := colors(c)
	var input ui.Element
	box := ui.Row(c).Height(34).Gap(6).Padding(0, 8, 0, 10).Radius(17).Background(p.SearchBG).TextColor(p.Label2).Cursor(ui.CursorText)
	box.Children(func() {
		symbol(c, "magnifyingglass", 13, 2)
		input = ui.TextInputBase(c, value).Grow(1).Shrink(1).MinWidth(0).FontSize(13).TextColor(p.Label).Placeholder(placeholder).Label(placeholder).FocusRing(false)
		if *value != "" {
			clear := ui.ButtonBase(c).Size(16, 16).Radius(8).Center().Background(p.Label3).TextColor(p.Content).Label(L("Clear")).FocusRing(false)
			clear.Children(func() { symbol(c, "xmark", 9, 3) })
			if clear.Clicked() {
				*value = ""
				input.Focus()
			}
		}
	})
	if input.Focused() {
		box.Shadow(0, 0, 0, 1, p.Accent).Border(1, p.Accent)
	}
	if box.Clicked() {
		input.Focus()
	}
	return input
}

// MARK: - Lists

// listPage is everything in one of the home page's sections: the featured plugins or bots, or a
// category.
func (mk *marketplace) listPage(c *ui.Context, page *marketPage) {
	ui.Column(c).Gap(12).Children(func() {
		shown, mixed := marketFiltered(page.items(&mk.catalog), page.filter)
		var control func()
		if mixed {
			control = func() { marketKindFilter(c, &page.filter) }
		}
		marketPageTitle(c, page.title, control)
		if len(shown) == 0 {
			marketStatusLine(c, L("Nothing here yet."))
			return
		}
		mk.grid(c, shown)
	})
}

// installedPage is the plugins the picked Runner has. Each opens its own sheet: its sign-in, its
// setup, and Remove.
func (mk *marketplace) installedPage(c *ui.Context) {
	ui.Column(c).Gap(12).Children(func() {
		on := mk.runner()
		if on == nil {
			marketPageTitle(c, L("Installed"), nil)
			marketStatusLine(c, L("Pair a Runner first."))
			return
		}
		marketPageTitle(c, L("Plugins on %@", on.Name), nil)
		marketStatusLine(c, L("Every bot on %@ can use these. Sign-ins and keys stay on %@.", on.Name, on.Name))
		plugins := mk.installedPlugins()
		section(c, L("Installed"), sectionHeading, nil, func(k *card) {
			if len(plugins) == 0 {
				noteRow(c, k, L("Nothing installed yet. Find plugins in the marketplace."), nil)
				return
			}
			for _, plugin := range plugins {
				ui.Box(c.Key(plugin.ID)).Children(func() {
					if _, result := pluginRow(c, k, plugin, true, L("Open %@", plugin.Name)); result.Clicked {
						mk.manage(plugin.ID)
					}
				})
			}
		})
	})
}

// MARK: - A plugin

// marketCard is a card in System Settings' look, its count beside the title when it has one.
func marketCard(c *ui.Context, title string, count int, rows func(k *card)) {
	var accessory func()
	if count >= 0 {
		accessory = func() { ui.Text(c, strconv.Itoa(count)).FontSize(12).TextColor(colors(c).Label2) }
	}
	section(c, title, sectionHeading, accessory, rows)
}

// marketWebsiteButton is a push button for a link, with the arrow that says it leaves the app.
func marketWebsiteButton(c *ui.Context, link string) ui.Element {
	p := colors(c)
	b := ui.ButtonBase(c).Height(24).Padding(0, 12).Radius(6).Gap(5).Justify(ui.Center).FontSize(13).TextColor(p.Label).Label(L("Website")).Tooltip(link)
	fill := p.ButtonBG
	switch {
	case b.Pressed():
		fill = fill.Mix(p.Label, 0.08)
	case b.Hovered():
		fill = fill.Mix(p.Label, 0.04)
	}
	b.Background(fill)
	b.Children(func() {
		ui.Text(c, L("Website")).SingleLine()
		symbol(c, "arrow.up.right", 11, 2.4)
	})
	return b
}

// pluginPage is a plugin in full: what it does, its servers and skills, the setup it asks for, and
// who makes it, with Add, or its state and Manage once the Runner has it.
func (mk *marketplace) pluginPage(c *ui.Context, page *marketPage) {
	p := colors(c)
	plugin := mk.plugin(page.id)
	if plugin == nil {
		if mk.loading == marketLoaded {
			marketStatusLine(c, L("This plugin is no longer in the marketplace."))
		} else {
			marketStatusLine(c, L("Loading…"))
		}
		return
	}
	on := mk.runner()
	installed, isInstalled := mk.installedPlugin(plugin.ID)
	var byline []string
	if plugin.Author != "" {
		byline = append(byline, L("by %@", plugin.Author))
	}
	if plugin.Category != "" {
		byline = append(byline, marketCategoryTitle(plugin.Category))
	}
	site := hostOf(plugin.Homepage)

	ui.Column(c).Gap(22).Children(func() {
		ui.Column(c).Gap(14).Children(func() {
			ui.Row(c).Gap(14).Padding(0, 12).Children(func() {
				marketPluginIcon(c, plugin.ID, marketPluginSymbol(plugin), 56)
				ui.Column(c).Gap(3).Grow(1).Shrink(1).MinWidth(0).Children(func() {
					ui.Text(c, plugin.Name).FontSize(20).FontWeight(600)
					if len(byline) > 0 {
						ui.Text(c, strings.Join(byline, " · ")).FontSize(12.5).TextColor(p.Label2)
					}
				})
				ui.Row(c).Gap(8).Children(func() {
					if site != "" && marketWebsiteButton(c, plugin.Homepage).Clicked() {
						openExternal(plugin.Homepage)
					}
					switch {
					case mk.installing[plugin.ID]:
						spinner(c, 16).Label(L("Adding %@", plugin.Name))
					case isInstalled && plugin.NamedAccounts:
						if pushButton(c, L("Add Account…"), pushOptions{}).Clicked() {
							mk.addAccount(plugin)
						}
					case isInstalled:
						if pushButton(c, L("Manage…"), pushOptions{}).Clicked() {
							mk.manage(installed.ID)
						}
					default:
						tip := L("Pair a Runner first.")
						if on != nil {
							tip = L("Install %@ on %@, for every bot there", plugin.Name, on.Name)
						}
						if pushButton(c, L("Add"), pushOptions{Kind: buttonPrimary, Disabled: on == nil, Tooltip: tip}).Clicked() {
							mk.install(plugin)
						}
					}
				})
			})
			ui.Text(c, plugin.Description).Padding(0, 12).FontSize(13).LineHeight(1.45).TextColor(p.Label2).Selectable()
		})

		if on != nil && isInstalled && plugin.NamedAccounts {
			marketCard(c, L("Accounts on %@", on.Name), -1, func(k *card) {
				for _, account := range mk.installedAccounts(plugin.ID) {
					text, tone := account.ShortStatus()
					color := p.tone(tone)
					_, row := statusRow(c.Key(account.ID), k, statusRowOptions{Symbol: marketPluginSymbol(plugin), PluginID: plugin.ID,
						Title: firstNonEmpty(account.AccountName, account.Name), State: text, StateColor: &color, Clickable: true, Tooltip: L("Open %@", account.Name)})
					if row.Clicked {
						mk.manage(account.ID)
					}
				}
			})
		} else if on != nil && isInstalled {
			marketCard(c, L("On %@", on.Name), -1, func(k *card) {
				action := ""
				switch installed.State {
				case model.PluginNeedsAuth, model.PluginInsufficientAccess:
					action = L("Connect")
				case model.PluginNeedsSetup:
					action = L("Set Up")
				}
				_, row := statusRow(c, k, statusRowOptions{Symbol: on.Symbol(), Title: installed.Detail, Subtitle: L("Every bot on %@ can use it.", on.Name), ActionTitle: action})
				if row.Action {
					mk.manage(installed.ID)
				}
			})
		}
		if len(plugin.Servers) > 0 {
			marketCard(c, L("Servers"), len(plugin.Servers), func(k *card) {
				for _, server := range plugin.Servers {
					symbolName, subtitle := "terminal", L("Runs on the Runner: %@", server.Address)
					if server.IsRemote {
						parts := []string{firstNonEmpty(hostOf(server.Address), server.Address)}
						if server.SignsIn {
							parts = append(parts, L("Signs in with your account"))
						}
						symbolName, subtitle = "network", strings.Join(parts, " · ")
					}
					statusRow(c, k, statusRowOptions{Symbol: symbolName, Title: server.Name, Subtitle: subtitle})
				}
			})
		}
		if len(plugin.Skills) > 0 {
			marketCard(c, L("Skills"), len(plugin.Skills), func(k *card) {
				for _, skill := range plugin.Skills {
					statusRow(c, k, statusRowOptions{Symbol: "cube", Title: skill.Name, Subtitle: skill.Description})
				}
			})
		}
		if len(plugin.Variables) > 0 {
			marketCard(c, Lc("Setup", "plugin variables"), -1, func(k *card) {
				for _, variable := range plugin.Variables {
					symbolName, state := "slider.horizontal.3", L("Optional")
					if variable.Secret {
						symbolName = "key"
					}
					if variable.Required {
						state = L("Required")
					}
					statusRow(c, k, statusRowOptions{Symbol: symbolName, Title: variable.Name, Subtitle: variable.Description, State: state})
				}
			})
		}
		if plugin.Author != "" || plugin.Category != "" || site != "" {
			marketCard(c, L("Information"), -1, func(k *card) {
				if plugin.Author != "" {
					keyValueRow(c, k, L("Developer"), plugin.Author, false, nil)
				}
				if plugin.Category != "" {
					keyValueRow(c, k, L("Category"), marketCategoryTitle(plugin.Category), false, nil)
				}
				if site != "" {
					keyValueRow(c, k, L("Website"), site, false, nil)
				}
			})
		}
	})
}

// MARK: - A bot

// marketPart is one part of a bot the list beside its panel picks.
type marketPart int

const (
	marketInstructions marketPart = iota
	marketMemories
	marketRoutines
	marketPlugins
)

func (part marketPart) title() string {
	switch part {
	case marketMemories:
		return L("Memories")
	case marketRoutines:
		return L("Routines")
	case marketPlugins:
		return L("Plugins")
	}
	return L("Instructions")
}

func (part marketPart) subtitle() string {
	switch part {
	case marketMemories:
		return L("Facts it already knows")
	case marketRoutines:
		return L("Jobs that run on their own")
	case marketPlugins:
		return L("What it works with")
	}
	return L("How this bot should work")
}

// marketParagraph is a paragraph of a bot's panel, in its lines as written.
func marketParagraph(c *ui.Context, text string, tint ui.Color) ui.Element {
	return ui.Text(c, text).FontSize(13).LineHeight(1.45).TextColor(tint)
}

// botPage is a bot in full: who it is with Add Bot, then its instructions, what it already knows,
// its routines, and its plugins, one at a time from the list beside them.
func (mk *marketplace) botPage(c *ui.Context, page *marketPage) {
	p := colors(c)
	template := mk.template(page.id)
	if template == nil {
		if mk.loading == marketLoaded {
			marketStatusLine(c, L("This bot is no longer in the marketplace."))
		} else {
			marketStatusLine(c, L("Loading…"))
		}
		return
	}
	on := mk.runner()
	parts := []marketPart{marketInstructions}
	if len(template.Memory) > 0 {
		parts = append(parts, marketMemories)
	}
	if len(template.Routines) > 0 {
		parts = append(parts, marketRoutines)
	}
	if len(template.Plugins) > 0 {
		parts = append(parts, marketPlugins)
	}
	selected := marketInstructions
	for _, part := range parts {
		if part == page.part {
			selected = part
		}
	}

	ui.Column(c).Gap(20).Children(func() {
		ui.Column(c).Gap(12).Padding(0, 12).Children(func() {
			ui.Row(c).Children(func() {
				avatar(c, avatarContent{Kind: avatarBot, SymbolName: template.SymbolName, Accent: template.Accent}, 64, false)
			})
			ui.Row(c).Gap(12).Margin(2, 0, 0, 0).Children(func() {
				ui.Column(c).Gap(3).Grow(1).Shrink(1).MinWidth(0).Children(func() {
					ui.Text(c, template.Name).FontSize(22).FontWeight(600)
					if template.Author != "" {
						ui.Text(c, L("By %@", template.Author)).FontSize(12.5).TextColor(p.Label2)
					}
				})
				if pushButton(c, L("Add Bot"), pushOptions{Kind: buttonPrimary, Disabled: on == nil}).Clicked() {
					mk.add(template)
				}
			})
			ui.Text(c, template.Summary).FontSize(14).LineHeight(1.4)
			note := L("Pair a Runner first: a bot runs on a computer.")
			if on != nil {
				note = L("Adds %@ to %@. Its routines start paused, and it asks before it installs a plugin.", template.Name, on.Name)
			}
			ui.Text(c, note).Margin(-4, 0, 0, 0).FontSize(12).LineHeight(1.4).TextColor(p.Label2)
		})
		ui.Box(c).Height(1).Background(p.Separator)
		// The panel is at least as tall as the list beside it.
		ui.Row(c).Gap(16).AlignItems(ui.Stretch).Children(func() {
			ui.Column(c).Width(196).Gap(2).AlignSelf(ui.Start).Children(func() {
				for _, part := range parts {
					ui.Box(c.Key(int(part))).Children(func() {
						on := part == selected
						item := ui.ButtonBase(c).Column().Gap(2).Padding(10, 10, 10, 14).Radius(10).Justify(ui.Start).AlignItems(ui.Start).
							Label(part.title()).Role(ui.RoleToggleButton).Checked(on)
						if on {
							item.Background(p.BotBubble)
						}
						if item.Clicked() {
							page.part = part
						}
						item.Children(func() {
							title := ui.Text(c, part.title()).FontSize(13)
							if on {
								title.FontWeight(600)
							}
							ui.Text(c, part.subtitle()).FontSize(12).TextColor(p.Label2)
						})
					})
				}
			})
			panel := ui.Column(c).Gap(16).Grow(1).Shrink(1).MinWidth(0).Padding(18, 20, 20, 20).Radius(12).Background(p.BotBubble)
			if selected == marketPlugins {
				panel.Gap(4)
			}
			if selected != marketPlugins {
				// The part's words are one selection: a drag runs across its facts or prompts.
				panel.Selectable()
			}
			panel.Children(func() { mk.botPanel(c, template, selected) })
		})
	})
}

// botPanel is what the panel beside the list shows for the part picked there.
func (mk *marketplace) botPanel(c *ui.Context, template *model.BotTemplate, part marketPart) {
	p := colors(c)
	switch part {
	case marketInstructions:
		marketParagraph(c, template.Description, p.Label)
	case marketMemories:
		for _, fact := range template.Memory {
			marketParagraph(c, fact, p.Label)
		}
	case marketRoutines:
		for _, routine := range template.Routines {
			ui.Column(c).Gap(3).Children(func() {
				ui.Text(c, routine.Name).FontSize(13).FontWeight(600).Unselectable()
				ui.Text(c, model.Schedule(routine.ScheduleText)).FontSize(12).TextColor(p.Label2).Unselectable()
				marketParagraph(c, routine.Prompt, p.Label2)
			})
		}
		marketParagraph(c, L("They start paused. %@ asks whether to turn them on.", template.Name), p.Label3).Unselectable()
	case marketPlugins:
		for _, id := range template.Plugins {
			plugin := mk.plugin(id)
			if plugin == nil {
				continue
			}
			state, tint := L("Not installed"), p.Label2
			if installed, ok := mk.installedPlugin(plugin.ID); ok {
				if installed.State == model.PluginReady {
					state, tint = L("Installed"), p.Green
				} else {
					state, tint = installed.Detail, p.tone(installed.State.Tone())
				}
			}
			media := func() { marketPluginIcon(c, plugin.ID, marketPluginSymbol(plugin), 36) }
			accessory := func() { ui.Text(c, state).FontSize(12).TextColor(tint).SingleLine() }
			if marketRow(c, "plugin:"+plugin.ID, marketRowOptions{Title: plugin.Name, Subtitle: plugin.Description, OnCard: true}, media, accessory) {
				mk.show(&marketPage{kind: marketPluginPage, id: plugin.ID})
			}
		}
		if on := mk.runner(); on != nil {
			marketParagraph(c, L("%@ asks before it installs one on %@.", template.Name, on.Name), p.Label3)
		}
	}
}
