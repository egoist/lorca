package main

import (
	"slices"
	"sort"
	"strings"
	"time"

	"github.com/egoist/lorca/desktop/model"
	"github.com/egoist/mygo/ui"
)

// The marketplace, after the macOS app's MarketplaceViewController and its pages: featured plugins
// and bots, everything else by category, one search over both, and a page for each plugin and bot.
// A sheet with a way back: the home page leads to a plugin, a bot, a full list, or the plugins the
// Runner has. Plugins install on the Runner picked in the top bar, for every bot there; a bot is
// added to that Runner, and the sheet closes on the new bot's chat, where the bot sets itself up.

// marketPreviewCount is how many rows a home section shows before View all.
const marketPreviewCount = 4

// marketItem is one entry of a grid: a plugin or a bot.
type marketItem struct {
	plugin *model.MarketplacePlugin
	bot    *model.BotTemplate
}

func (i marketItem) category() string {
	if i.plugin != nil {
		return i.plugin.Category
	}
	return i.bot.Category
}

// key tells the item apart in a grid, a plugin and a bot sharing an id too.
func (i marketItem) key() string {
	if i.plugin != nil {
		return "plugin:" + i.plugin.ID
	}
	return "bot:" + i.bot.ID
}

func marketPluginItems(plugins []model.MarketplacePlugin, keep func(*model.MarketplacePlugin) bool) []marketItem {
	var out []marketItem
	for i := range plugins {
		if keep == nil || keep(&plugins[i]) {
			out = append(out, marketItem{plugin: &plugins[i]})
		}
	}
	return out
}

func marketBotItems(bots []model.BotTemplate, keep func(*model.BotTemplate) bool) []marketItem {
	var out []marketItem
	for i := range bots {
		if keep == nil || keep(&bots[i]) {
			out = append(out, marketItem{bot: &bots[i]})
		}
	}
	return out
}

// marketAllItems is every plugin, then every bot.
func marketAllItems(catalog *model.Marketplace) []marketItem {
	return append(marketPluginItems(catalog.Plugins, nil), marketBotItems(catalog.Bots, nil)...)
}

// marketMatches is every word of the query appearing in its name, description, category, or maker.
func marketMatches(item marketItem, words []string) bool {
	var fields []string
	if p := item.plugin; p != nil {
		fields = append([]string{p.Name, p.Description, p.Author, marketCategoryTitle(p.Category)}, p.Tags...)
	} else {
		b := item.bot
		fields = []string{b.Name, b.Summary, b.Description, b.Author, marketCategoryTitle(b.Category)}
	}
	text := strings.ToLower(strings.Join(fields, " "))
	for _, word := range words {
		if !strings.Contains(text, strings.ToLower(word)) {
			return false
		}
	}
	return true
}

// marketCategoryOrder is the order of the sections the home page lists after the featured ones.
var marketCategoryOrder = []string{"credentials", "productivity", "communication", "design", "code", "data", "sales", "finance", "research", "support"}

func marketCategoryTitle(key string) string {
	switch key {
	case "credentials":
		return L("Login and Credential Management")
	case "productivity":
		return L("Productivity")
	case "communication":
		return L("Communication")
	case "design":
		return L("Design")
	case "code":
		return L("Code")
	case "data":
		return L("Data")
	case "sales":
		return L("Sales")
	case "finance":
		return L("Finance")
	case "research":
		return L("Research")
	case "support":
		return L("Support")
	case "":
		return L("More")
	}
	words := strings.Split(strings.ReplaceAll(key, "-", " "), " ")
	for i, word := range words {
		if word != "" {
			words[i] = strings.ToUpper(word[:1]) + strings.ToLower(word[1:])
		}
	}
	return strings.Join(words, " ")
}

// marketCategoryKeys are the categories in use: known ones in their order, then the rest, then the
// uncategorized.
func marketCategoryKeys(items []marketItem) []string {
	used := map[string]bool{}
	for _, item := range items {
		used[item.category()] = true
	}
	var keys, other []string
	for _, key := range marketCategoryOrder {
		if used[key] {
			keys = append(keys, key)
		}
	}
	for key := range used {
		if key != "" && !slices.Contains(marketCategoryOrder, key) {
			other = append(other, key)
		}
	}
	sort.Strings(other)
	keys = append(keys, other...)
	if used[""] {
		keys = append(keys, "")
	}
	return keys
}

// marketFiltered is a grid's items under the kind filter (0 everything, 1 plugins, 2 bots), and
// whether both kinds are there, which is when the filter shows.
func marketFiltered(items []marketItem, kind int) (shown []marketItem, mixed bool) {
	var plugins, bots []marketItem
	for _, item := range items {
		if item.plugin != nil {
			plugins = append(plugins, item)
		} else {
			bots = append(bots, item)
		}
	}
	mixed = len(plugins) > 0 && len(bots) > 0
	switch {
	case !mixed:
		return items, false
	case kind == 1:
		return plugins, true
	case kind == 2:
		return bots, true
	}
	return items, true
}

// marketPluginSymbol is a marketplace plugin's symbol, a puzzle piece when it names none.
func marketPluginSymbol(plugin *model.MarketplacePlugin) string {
	return firstNonEmpty(plugin.Icon, "puzzlepiece.extension")
}

// MARK: - The sheet

type marketLoading int

const (
	marketLoadingNow marketLoading = iota
	marketLoaded
	marketFailed
)

type marketPageKind int

const (
	marketHomePage marketPageKind = iota
	marketPluginPage
	marketBotPage
	marketListPage
	marketInstalledPage
)

// marketPage is one page of the sheet and what it keeps while pages above it come and go: its
// search, its filter, the part of a bot it shows, and where it is scrolled to.
type marketPage struct {
	kind   marketPageKind
	serial int
	// id is the plugin's or the bot's.
	id string
	// title and items are a list's: everything in one of the home page's sections.
	title string
	items func(*model.Marketplace) []marketItem

	query  string
	filter int
	part   marketPart
	scroll ui.ScrollState
	// focusSearch puts the keyboard in the home page's search as the sheet opens.
	focusSearch bool
}

// marketNotice is the line at the foot of the sheet for what an install did, gone after a few
// seconds.
type marketNotice struct {
	text    string
	isError bool
	// at is when it showed, set by the frame that first draws it.
	at time.Time
}

type marketplace struct {
	m       *mainWindow
	sheet   *sheet
	catalog model.Marketplace
	loading marketLoading
	// installing are the plugins being installed, by id.
	installing map[string]bool
	// installed is what an install answered, by Runner and plugin, until the Runner's own list has
	// it: a Runner elsewhere reports its plugins through the relay a moment later.
	installed map[string]map[string]model.InstalledPlugin
	pages     []*marketPage
	serials   int
	// runnerID is where plugins install and bots are added.
	runnerID string
	notice   marketNotice
	// width and height are the sheet's, sized to the window as it opens.
	width, height float32
}

// marketplaceSheet is the marketplace up over the main window, which opens one at a time.
var marketplaceSheet *sheet

// presentMarketplace opens the marketplace as a sheet sized to the window. From a bot's inspector
// it opens on that bot's Runner, from Settings on the picked Device; a bot added there lands in its
// chat.
func (m *mainWindow) presentMarketplace(runnerID string) {
	if marketplaceSheet != nil && marketplaceSheet.window == &m.appWindow {
		return
	}
	mk := &marketplace{
		m:          m,
		installing: map[string]bool{},
		installed:  map[string]map[string]model.InstalledPlugin{},
	}
	runners := store.Runners()
	picked := slices.IndexFunc(runners, func(device *model.Device) bool { return device.ID == runnerID })
	if picked < 0 {
		picked = max(0, slices.IndexFunc(runners, func(device *model.Device) bool { return device.IsThisDevice }))
	}
	if picked < len(runners) {
		mk.runnerID = runners[picked].ID
	}
	mk.show(&marketPage{kind: marketHomePage, focusSearch: true})
	mk.sheet = m.present(mk.view, func() { marketplaceSheet = nil })
	marketplaceSheet = mk.sheet
	mk.load()
}

func (mk *marketplace) load() {
	mk.loading = marketLoadingNow
	store.Marketplace(func(catalog model.Marketplace, err error) {
		if err != nil {
			mk.loading = marketFailed
			return
		}
		mk.catalog, mk.loading = catalog, marketLoaded
	})
}

// runner is where plugins install and bots are added; the first Runner when the picked one leaves.
func (mk *marketplace) runner() *model.Device {
	runners := store.Runners()
	for _, device := range runners {
		if device.ID == mk.runnerID {
			return device
		}
	}
	if len(runners) > 0 {
		mk.runnerID = runners[0].ID
		return runners[0]
	}
	mk.runnerID = ""
	return nil
}

func (mk *marketplace) plugin(id string) *model.MarketplacePlugin {
	for i := range mk.catalog.Plugins {
		if mk.catalog.Plugins[i].ID == id {
			return &mk.catalog.Plugins[i]
		}
	}
	return nil
}

func (mk *marketplace) template(id string) *model.BotTemplate {
	for i := range mk.catalog.Bots {
		if mk.catalog.Bots[i].ID == id {
			return &mk.catalog.Bots[i]
		}
	}
	return nil
}

// installedPlugin is the plugin as the picked Runner has it; false when it is not installed there.
// Of a service's named accounts, one that is ready stands for them all; each says how it stands on
// the service's page.
func (mk *marketplace) installedPlugin(id string) (model.InstalledPlugin, bool) {
	accounts := mk.installedAccounts(id)
	for _, account := range accounts {
		if account.State == model.PluginReady {
			return account, true
		}
	}
	if len(accounts) > 0 {
		return accounts[0], true
	}
	return model.InstalledPlugin{}, false
}

// installedAccounts is each install of a marketplace plugin on the picked Runner: one, or a
// service's accounts, in the Runner's order.
func (mk *marketplace) installedAccounts(id string) []model.InstalledPlugin {
	on := mk.runner()
	if on == nil {
		return nil
	}
	var accounts, replied []model.InstalledPlugin
	// A server of the Runner's mcp.json that shares the id is not this plugin.
	for _, plugin := range on.Plugins {
		if plugin.MarketplaceID() == id && !plugin.IsMcpServer() {
			accounts = append(accounts, plugin)
		}
	}
	for _, plugin := range mk.installed[on.ID] {
		if plugin.MarketplaceID() == id && !slices.ContainsFunc(accounts, func(each model.InstalledPlugin) bool { return each.ID == plugin.ID }) {
			replied = append(replied, plugin)
		}
	}
	sort.Slice(replied, func(a, b int) bool { return replied[a].Name < replied[b].Name })
	return append(accounts, replied...)
}

// installedPlugins is everything the picked Runner has, the marketplace's and the rest, in its own
// order.
func (mk *marketplace) installedPlugins() []model.InstalledPlugin {
	on := mk.runner()
	if on == nil {
		return nil
	}
	var extra []model.InstalledPlugin
	for _, plugin := range mk.installed[on.ID] {
		if !slices.ContainsFunc(on.Plugins, func(each model.InstalledPlugin) bool { return each.ID == plugin.ID }) {
			extra = append(extra, plugin)
		}
	}
	sort.Slice(extra, func(a, b int) bool { return extra[a].Name < extra[b].Name })
	return append(slices.Clone(on.Plugins), extra...)
}

// show puts a page in front, with the way back to the one before it.
func (mk *marketplace) show(page *marketPage) {
	mk.serials++
	page.serial = mk.serials
	mk.pages = append(mk.pages, page)
}

func (mk *marketplace) goBack() {
	if len(mk.pages) > 1 {
		mk.pages = mk.pages[:len(mk.pages)-1]
	}
}

func (mk *marketplace) showNotice(text string, isError bool) {
	mk.notice = marketNotice{text: text, isError: isError}
}

func marketNextStep(plugin model.InstalledPlugin, on *model.Device) string {
	switch plugin.State {
	case model.PluginReady:
		return L("Added %@. Every bot on %@ can use it.", plugin.Name, on.Name)
	case model.PluginNeedsAuth, model.PluginInsufficientAccess:
		return L("Added %@. It needs a sign-in: click Connect.", plugin.Name)
	case model.PluginNeedsSetup:
		return L("Added %@. It needs setup: click Set Up.", plugin.Name)
	}
	return plugin.Name + ": " + plugin.Detail
}

// install installs a plugin on the picked Runner, for every bot there.
func (mk *marketplace) install(plugin *model.MarketplacePlugin) {
	on := mk.runner()
	if on == nil || mk.installing[plugin.ID] {
		return
	}
	if plugin.NamedAccounts {
		mk.addAccount(plugin)
		return
	}
	id, name := plugin.ID, plugin.Name
	mk.installing[id] = true
	store.InstallPlugin(id, on.ID, func(status model.InstalledPlugin, err error) {
		delete(mk.installing, id)
		if err != nil {
			mk.showNotice(L("Couldn't install %@: %@", name, model.ErrorText(err)), true)
			return
		}
		mk.remember(on.ID, status)
		mk.showNotice(marketNextStep(status, on), false)
	})
}

// addAccount adds another account of a service with named accounts, such as a work Gmail beside a
// personal one, and opens it for its setup and sign-in.
func (mk *marketplace) addAccount(plugin *model.MarketplacePlugin) {
	on := mk.runner()
	if on == nil || mk.installing[plugin.ID] {
		return
	}
	id, name, runnerID := plugin.ID, plugin.Name, on.ID
	value := ""
	mk.m.showAlert(alertOptions{
		Message:     L("New %@ Account", name),
		Informative: L("A name such as Work or Personal tells your bots which account to use."),
		Buttons:     []alertButton{{Title: L("Add")}, {Title: L("Cancel")}},
		Accessory: func(c *ui.Context) {
			textField(c, &value, fieldOptions{Placeholder: L("Work"), AutoFocus: true})
		},
	}, func(answer int) {
		if answer != 0 {
			return
		}
		mk.installing[id] = true
		// A blank name becomes the next free "Account 1" on the Runner.
		store.InstallPluginAccount(id, runnerID, strings.TrimSpace(value), func(status model.InstalledPlugin, err error) {
			delete(mk.installing, id)
			if err != nil {
				mk.showNotice(L("Couldn't install %@: %@", name, model.ErrorText(err)), true)
				return
			}
			mk.remember(runnerID, status)
			mk.manage(status.ID)
		})
	})
}

// remember keeps what an install answered until the Runner's roster lists it.
func (mk *marketplace) remember(runnerID string, status model.InstalledPlugin) {
	if mk.installed[runnerID] == nil {
		mk.installed[runnerID] = map[string]model.InstalledPlugin{}
	}
	mk.installed[runnerID][status.ID] = status
}

// manage opens an installed plugin's own sheet on the picked Runner, by the id the Runner gave it (a
// named account's own): its sign-in, its setup, and Remove.
func (mk *marketplace) manage(pluginID string) {
	if on := mk.runner(); on != nil {
		mk.m.presentPlugin(pluginID, on)
	}
}

// add adds the bot on the picked Runner and opens its chat, where it greets the user.
func (mk *marketplace) add(template *model.BotTemplate) {
	on := mk.runner()
	if on == nil {
		return
	}
	chatID := store.AddBotFromTemplate(*template, on.ID)
	mk.sheet.dismiss()
	mk.m.open(chatID)
}

func (mk *marketplace) view(c *ui.Context, s *sheet) {
	p := colors(c)
	if mk.width == 0 {
		w, h := c.Size()
		mk.width, mk.height = min(800, max(640, w-60)), min(700, max(460, h-60))
	}
	page := mk.pages[len(mk.pages)-1]
	panel := ui.Box(c).Width(mk.width).Height(mk.height).MaxWidthPercent(100).MaxHeightPercent(100).Radius(22).Clip().
		Background(p.Sheet).Shadow(0, 10, 40, 0, p.Shadow).Border(0.5, p.ShadowEdge).Label(L("Marketplace")).
		Transition(ui.ElementTransition{Enter: &ui.Motion{Y: -10}, Duration: 160 * time.Millisecond})
	panel.Children(func() {
		ui.Box(c).Absolute().Top(44).Left(0).Right(0).Bottom(0).Children(func() {
			content := ui.Scroll(c.Key(page.serial)).Fill().Padding(2, 32, 32, 32).Gap(24).TrackScroll(&page.scroll).
				Transition(ui.ElementTransition{Enter: &ui.Motion{}, Duration: 180 * time.Millisecond})
			content.Children(func() { mk.pageView(c, page) })
		})
		mk.bar(c, s)
		mk.noticeView(c)
	})
	// Escape closes the sheet from any page, wherever the keyboard is, the search field too.
	if panel.OverlayShortcut(0, ui.KeyEscape) {
		s.dismiss()
	}
}

// bar is the top of the sheet: Back, the Runner picker, and Close.
func (mk *marketplace) bar(c *ui.Context, s *sheet) {
	ui.Row(c).Absolute().Top(10).Left(10).Right(10).Height(28).Gap(8).Children(func() {
		if len(mk.pages) >= 2 && hoverButton(c, hoverButtonOptions{Symbol: "chevron.left", Tooltip: L("Back")}).Clicked() {
			mk.goBack()
		}
		ui.Spacer(c)
		// One Runner is no choice; the pages name it where it matters.
		if runners := store.Runners(); len(runners) >= 2 {
			on := mk.runner()
			options := make([]popUpOption, 0, len(runners))
			for _, device := range runners {
				label := device.Name
				if device.IsThisDevice {
					label = L("%@ (this computer)", device.Name)
				}
				options = append(options, popUpOption{Value: device.ID, Label: label})
			}
			tip := L("Plugins install here, and bots are added here.")
			if picked, changed, _ := popUpButton(c, popUp{Options: options, Value: on.ID, Style: popUpSettings, Tooltip: tip, Label: tip}); changed {
				mk.runnerID = picked
			}
		}
		if hoverButton(c, hoverButtonOptions{Symbol: "xmark", Size: 14, Tooltip: L("Close")}).Clicked() {
			s.dismiss()
		}
	})
}

func (mk *marketplace) noticeView(c *ui.Context) {
	n := &mk.notice
	if n.text == "" {
		return
	}
	if n.at.IsZero() {
		n.at = c.Now()
	}
	shown := 4 * time.Second
	if n.isError {
		shown = 7 * time.Second
	}
	elapsed := c.Now().Sub(n.at)
	if elapsed >= shown {
		// Its last frame fades out as it goes.
		*n = marketNotice{}
		return
	}
	c.After(shown - elapsed)
	p := colors(c)
	tint := p.Label
	if n.isError {
		tint = p.Red
	}
	box := ui.Box(c.Key("notice")).Attach(ui.AnchorBottom, ui.AnchorBottom).Bottom(18).MaxWidth(mk.width-96).Padding(9, 14).Radius(10).
		Border(1, p.Separator).Background(p.Window).Shadow(0, 2, 12, 0, ui.RGBA(0, 0, 0, 0.18)).Role(ui.RoleStatus).
		Transition(ui.ElementTransition{Exit: &ui.Motion{}, Duration: 250 * time.Millisecond})
	box.Children(func() {
		ui.Text(c, n.text).FontSize(12.5).LineHeight(1.35).MaxLines(2).TextColor(tint)
	})
}

func (mk *marketplace) pageView(c *ui.Context, page *marketPage) {
	switch page.kind {
	case marketHomePage:
		mk.homePage(c, page)
	case marketPluginPage:
		mk.pluginPage(c, page)
	case marketBotPage:
		mk.botPage(c, page)
	case marketListPage:
		mk.listPage(c, page)
	case marketInstalledPage:
		mk.installedPage(c)
	}
}

// MARK: - Pieces

// marketPluginIcon is a plugin's icon: its real mark on a white tile, or its symbol on a light tile
// with a hairline.
func marketPluginIcon(c *ui.Context, pluginID, symbolName string, size float32) ui.Element {
	if pluginMark(pluginID) != nil {
		return pluginTile(c, pluginID, symbolName, size)
	}
	p := colors(c)
	tile := ui.Row(c).Size(size, size).Radius(size*0.25).Center().Background(p.Content).Border(1, p.Separator).TextColor(p.Label)
	tile.Children(func() { symbol(c, symbolName, float32(int(size*0.46+0.5)), 1.8) })
	return tile
}

type marketRowOptions struct {
	Title string
	// Byline names the maker after the title, smaller.
	Byline   string
	Subtitle string
	// OnCard is a row on a bot's panel, which fills darker under the pointer.
	OnCard bool
}

// marketRow is one entry in a grid: the icon or avatar, the name with its maker, a line about it,
// and what can be done with it on the Runner. It reports a click anywhere but the accessory's
// button, which opens it.
func marketRow(c *ui.Context, key string, o marketRowOptions, media, accessory func()) bool {
	p := colors(c)
	opened := false
	ui.Box(c.Key(key)).MinWidth(0).Children(func() {
		label := o.Title
		for _, part := range []string{o.Byline, o.Subtitle} {
			if part != "" {
				label += ", " + part
			}
		}
		row := ui.ButtonBase(c).Height(64).Gap(12).Padding(0, 12).Radius(10).Justify(ui.Start).MinWidth(0).Shrink(1).Label(label).Tooltip(o.Subtitle)
		if row.Hovered() {
			if o.OnCard {
				row.Background(p.CodeHover)
			} else {
				row.Background(p.BotBubble)
			}
		}
		opened = row.Clicked()
		row.Children(func() {
			ui.Row(c).Children(media)
			ui.Column(c).Gap(2).Grow(1).Shrink(1).MinWidth(0).Children(func() {
				spans := []ui.Span{{Text: o.Title}}
				if o.Byline != "" {
					spans = append(spans, ui.Span{Text: "  " + o.Byline, Size: 12, Weight: 400, Color: p.Label2})
				}
				ui.RichText(c, spans...).FontSize(13).FontWeight(600).SingleLine()
				ui.Text(c, o.Subtitle).FontSize(12.5).TextColor(p.Label2).SingleLine()
			})
			if accessory != nil {
				ui.Row(c).Children(accessory)
			}
		})
	})
	return opened
}

// pluginAccessory is what a plugin's row offers on the picked Runner: Add, Added, Connect, Set Up,
// or its state.
func (mk *marketplace) pluginAccessory(c *ui.Context, plugin *model.MarketplacePlugin) {
	if mk.installing[plugin.ID] {
		spinner(c, 14).Label(L("Adding %@", plugin.Name))
		return
	}
	if current, ok := mk.installedPlugin(plugin.ID); ok {
		mk.installedAccessory(c, current)
		return
	}
	on := mk.runner()
	tip := L("Pair a Runner first.")
	if on != nil {
		tip = L("Install %@ on %@, for every bot there", plugin.Name, on.Name)
	}
	if pushButton(c, L("Add"), pushOptions{Disabled: on == nil, Tooltip: tip}).Clicked() {
		mk.install(plugin)
	}
}

func (mk *marketplace) installedAccessory(c *ui.Context, current model.InstalledPlugin) {
	p := colors(c)
	switch current.State {
	case model.PluginReady:
		ui.Row(c).Gap(5).Children(func() {
			symbol(c, "checkmark", 12, 2.6).TextColor(p.Green)
			ui.Text(c, L("Added")).FontSize(12.5).TextColor(p.Label2).SingleLine()
		})
	case model.PluginNeedsAuth, model.PluginInsufficientAccess:
		if pushButton(c, L("Connect"), pushOptions{}).Clicked() {
			mk.manage(current.ID)
		}
	case model.PluginNeedsSetup:
		if pushButton(c, L("Set Up"), pushOptions{}).Clicked() {
			mk.manage(current.ID)
		}
	default:
		// What went wrong, in full, is the tooltip and in the plugin's sheet.
		text, tone := current.ShortStatus()
		ui.Text(c, text).FontSize(12.5).TextColor(p.tone(tone)).SingleLine().Tooltip(current.Detail)
	}
}

func (mk *marketplace) itemRow(c *ui.Context, item marketItem) {
	if plugin := item.plugin; plugin != nil {
		media := func() { marketPluginIcon(c, plugin.ID, marketPluginSymbol(plugin), 40) }
		if marketRow(c, item.key(), marketRowOptions{Title: plugin.Name, Subtitle: plugin.Description}, media, func() { mk.pluginAccessory(c, plugin) }) {
			mk.show(&marketPage{kind: marketPluginPage, id: plugin.ID})
		}
		return
	}
	template := item.bot
	byline := ""
	if template.Author != "" {
		byline = L("by %@", template.Author)
	}
	media := func() {
		avatar(c, avatarContent{Kind: avatarBot, SymbolName: template.SymbolName, Accent: template.Accent}, 40, false)
	}
	accessory := func() {
		on := mk.runner()
		tip := L("Pair a Runner first.")
		if on != nil {
			tip = L("Add %@ to %@", template.Name, on.Name)
		}
		if pushButton(c, L("Add"), pushOptions{Disabled: on == nil, Tooltip: tip}).Clicked() {
			mk.add(template)
		}
	}
	if marketRow(c, item.key(), marketRowOptions{Title: template.Name, Byline: byline, Subtitle: template.Summary}, media, accessory) {
		mk.show(&marketPage{kind: marketBotPage, id: template.ID})
	}
}

// grid lays the rows two to a line: equal columns 8 apart, a line 2 below the one before.
func (mk *marketplace) grid(c *ui.Context, items []marketItem) {
	ui.Grid(c).Columns(2).GapX(8).GapY(2).Children(func() {
		for _, item := range items {
			mk.itemRow(c, item)
		}
	})
}

// section is a home page section: its title with View all when there is more, or a control, over
// its grid.
func (mk *marketplace) section(c *ui.Context, title string, items []marketItem, control func(), viewAll func()) {
	ui.Column(c.Key(title)).Gap(6).Children(func() {
		header := ui.Row(c).Height(28).Gap(8).Padding(0, 4, 0, 12).Justify(ui.SpaceBetween)
		// View all hangs 4 in from the edge; a control, as the kind filter, 12.
		if viewAll == nil && control != nil {
			header.Padding(0, 12)
		}
		header.Children(func() {
			ui.Text(c, title).FontSize(14).FontWeight(600).SingleLine().Shrink(1)
			switch {
			case viewAll != nil:
				if hoverButton(c, hoverButtonOptions{Title: L("View all")}).Clicked() {
					viewAll()
				}
			case control != nil:
				control()
			}
		})
		mk.grid(c, items)
	})
}

// marketPageTitle is a page's own title, large, with an optional control at its trailing end.
func marketPageTitle(c *ui.Context, text string, control func()) {
	ui.Row(c).Gap(12).Padding(0, 12).Justify(ui.SpaceBetween).Children(func() {
		ui.Text(c, text).FontSize(22).FontWeight(600).Shrink(1)
		if control != nil {
			control()
		}
	})
}

// marketStatusLine is a line of secondary text on the rows' leading edge, for loading, empty, and
// notes.
func marketStatusLine(c *ui.Context, text string) ui.Element {
	return ui.Text(c, text).Padding(0, 12).FontSize(13).LineHeight(1.4).TextColor(colors(c).Label2)
}

// marketKindFilter is All, Plugins, and Bots, when both kinds are there.
func marketKindFilter(c *ui.Context, kind *int) {
	segmented(c, kind, L("Filter results"), L("All"), L("Plugins"), L("Bots"))
}
