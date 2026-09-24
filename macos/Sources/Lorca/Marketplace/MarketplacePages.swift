import AppKit

/// How many rows a home section shows before View all.
private let previewCount = 4

/// The marketplace's first page: its title with what the Runner has installed, the search, and
/// then the featured plugins and bots and each category, or what the search found.
final class MarketplaceHomePage: MarketplacePage {
    private let search = NSSearchField()
    private lazy var installedButton = HoverButton(
        title: "", trailingSymbol: "chevron.right", target: self, action: #selector(openInstalled))
    private let body = Build.stack([], spacing: 24)
    /// The results' filter: 0 everything, 1 plugins, 2 bots.
    private var kind = 0

    override func loadView() {
        super.loadView()
        let title = Build.label(L("Marketplace"), font: .systemFont(ofSize: 20, weight: .semibold))
        installedButton.translatesAutoresizingMaskIntoConstraints = false

        let header = NSView()
        header.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(title)
        header.addSubview(installedButton)
        NSLayoutConstraint.activate([
            header.heightAnchor.constraint(equalToConstant: 30),
            title.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 12),
            title.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            installedButton.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -4),
            installedButton.centerYAnchor.constraint(equalTo: header.centerYAnchor),
            title.trailingAnchor.constraint(lessThanOrEqualTo: installedButton.leadingAnchor, constant: -12),
        ])

        search.placeholderString = L("Search plugins and bots")
        search.controlSize = .large
        search.sendsSearchStringImmediately = true
        search.target = self
        search.action = #selector(searchChanged)
        search.translatesAutoresizingMaskIntoConstraints = false

        body.alignment = .leading
        for view in [header, search, body] as [NSView] {
            content.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: content.widthAnchor).isActive = true
        }
        content.setCustomSpacing(14, after: header)
        content.setCustomSpacing(26, after: search)
    }

    override func reload() {
        let installed = market.installedPlugins
        installedButton.label = L("%d installed", installed.count)
        installedButton.toolTip = market.runner.map { L("Plugins on %@", $0.name) }
        installedButton.isHidden = market.runner == nil

        let query = search.stringValue.trimmingCharacters(in: .whitespaces)
        let catalog = market.catalog
        var sections: [NSView]
        if catalog.items.isEmpty, market.loading != .loaded {
            sections = [loadingState()]
        } else if query.isEmpty {
            sections = homeSections(catalog)
        } else {
            sections = results(for: query, in: catalog)
        }
        for view in body.arrangedSubviews {
            body.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        for view in sections {
            body.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: body.widthAnchor).isActive = true
        }
    }

    private func loadingState() -> NSView {
        guard market.loading == .failed else { return statusLine(L("Loading the marketplace…")) }
        let line = statusLine(L("The marketplace isn't available right now."))
        let retry = ActionButton(title: L("Try Again")) { [weak self] in self?.market.load() }
        return Build.stack([line, retry], spacing: 10)
    }

    /// Featured Plugins, Featured Bots, then a section per category, plugins before bots. A
    /// featured section's View all lists every plugin or every bot, the featured ones first.
    private func homeSections(_ catalog: Marketplace) -> [NSView] {
        var sections: [NSView] = []
        let featuredPlugins = catalog.plugins.filter(\.isFeatured).map(MarketplaceItem.plugin)
        if !featuredPlugins.isEmpty {
            sections.append(section(L("Featured Plugins"), featuredPlugins, listTitle: L("Plugins")) { catalog in
                (catalog.plugins.filter(\.isFeatured) + catalog.plugins.filter { !$0.isFeatured }).map(MarketplaceItem.plugin)
            })
        }
        let featuredBots = catalog.bots.filter(\.isFeatured).map(MarketplaceItem.bot)
        if !featuredBots.isEmpty {
            sections.append(section(L("Featured Bots"), featuredBots, listTitle: L("Bots")) { catalog in
                (catalog.bots.filter(\.isFeatured) + catalog.bots.filter { !$0.isFeatured }).map(MarketplaceItem.bot)
            })
        }
        for key in MarketplaceCategory.keys(in: catalog.items) {
            let items = catalog.items.filter { $0.category == key }
            sections.append(section(MarketplaceCategory.title(key), items) { $0.items.filter { $0.category == key } })
        }
        return sections.isEmpty ? [statusLine(L("Nothing in the marketplace yet."))] : sections
    }

    /// A section showing the first rows of `items`, with View all when `all` has more: its page
    /// is titled `listTitle`, or the section's own title.
    private func section(
        _ title: String, _ items: [MarketplaceItem], listTitle: String? = nil, all: @escaping (Marketplace) -> [MarketplaceItem]
    ) -> NSView {
        let rows = items.prefix(previewCount).map(market.row(for:))
        let viewAll: (() -> Void)? =
            all(market.catalog).count > rows.count
            ? { [weak self] in self?.market.openList(title: listTitle ?? title, items: all) } : nil
        return MarketplaceSection(title: title, rows: rows, onViewAll: viewAll)
    }

    /// What matches every word of the query, with All, Plugins, and Bots when both kinds do.
    private func results(for query: String, in catalog: Marketplace) -> [NSView] {
        let words = query.split(whereSeparator: \.isWhitespace).map(String.init)
        let found = catalog.items.filter { $0.matches(words) }
        guard !found.isEmpty else { return [statusLine(L("No results match “%@”", query))] }
        let plugins = found.filter { if case .plugin = $0 { true } else { false } }
        let bots = found.filter { if case .bot = $0 { true } else { false } }
        var filter: NSView?
        var shown = found
        if !plugins.isEmpty, !bots.isEmpty {
            let control = NSSegmentedControl(
                labels: [L("All"), L("Plugins"), L("Bots")], trackingMode: .selectOne, target: self, action: #selector(kindChanged(_:)))
            control.segmentStyle = .automatic
            control.selectedSegment = kind
            control.setAccessibilityLabel(L("Filter results"))
            filter = control
            shown = kind == 1 ? plugins : kind == 2 ? bots : found
        }
        return [MarketplaceSection(title: L("Results"), rows: shown.map(market.row(for:)), accessory: filter)]
    }

    @objc private func searchChanged() {
        reload()
    }

    @objc private func kindChanged(_ sender: NSSegmentedControl) {
        kind = sender.selectedSegment
        reload()
    }

    @objc private func openInstalled() {
        market.openInstalled()
    }
}

/// Everything in one of the home page's sections: the featured plugins or bots, or a category.
final class MarketplaceListPage: MarketplacePage {
    private let heading: String
    private let items: (Marketplace) -> [MarketplaceItem]
    private var kind = 0

    init(market: MarketplaceViewController, heading: String, items: @escaping (Marketplace) -> [MarketplaceItem]) {
        self.heading = heading
        self.items = items
        super.init(market: market)
    }

    override func reload() {
        let all = items(market.catalog)
        let plugins = all.filter { if case .plugin = $0 { true } else { false } }
        let bots = all.filter { if case .bot = $0 { true } else { false } }
        var filter: NSView?
        var shown = all
        if !plugins.isEmpty, !bots.isEmpty {
            let control = NSSegmentedControl(
                labels: [L("All"), L("Plugins"), L("Bots")], trackingMode: .selectOne, target: self, action: #selector(kindChanged(_:)))
            control.selectedSegment = kind
            control.setAccessibilityLabel(L("Filter results"))
            filter = control
            shown = kind == 1 ? plugins : kind == 2 ? bots : all
        }
        let title = pageTitle(heading, accessory: filter)
        let grid = shown.isEmpty ? statusLine(L("Nothing here yet.")) : MarketplaceGrid(rows: shown.map(market.row(for:)))
        setContent([title, grid], spacing: 12)
    }

    @objc private func kindChanged(_ sender: NSSegmentedControl) {
        kind = sender.selectedSegment
        reload()
    }
}

/// The plugins the picked Runner has, from the home page's installed button. Each opens its
/// own sheet: its sign-in, its setup, and Remove.
final class MarketplaceInstalledPage: MarketplacePage {
    override func reload() {
        guard let runner = market.runner else {
            setContent([pageTitle(L("Installed")), statusLine(L("Pair a Runner first."))], spacing: 12)
            return
        }
        let section = SectionView(title: L("Installed"))
        section.style = .heading
        let plugins = market.installedPlugins
        section.setRows(
            plugins.isEmpty
                ? [NoteRow(text: L("Nothing installed yet. Find plugins in the marketplace."))]
                : plugins.map { plugin in
                    let row = StatusRow()
                    row.configure(plugin: plugin)
                    row.identifier = NSUserInterfaceItemIdentifier(plugin.id)
                    row.toolTip = L("Open %@", plugin.name)
                    row.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(open(_:))))
                    return row
                })
        let note = statusLine(L("Every bot on %@ can use these. Sign-ins and keys stay on %@.", runner.name, runner.name))
        setContent([pageTitle(L("Plugins on %@", runner.name)), note, section], spacing: 12)
    }

    @objc private func open(_ sender: NSClickGestureRecognizer) {
        guard let id = sender.view?.identifier?.rawValue else { return }
        market.manage(id)
    }
}

extension MarketplacePage {
    /// A page's own title, large, with an optional control at its trailing end.
    func pageTitle(_ text: String, accessory: NSView? = nil) -> NSView {
        let title = Build.label(text, font: .systemFont(ofSize: 22, weight: .semibold))
        let row = NSView()
        row.translatesAutoresizingMaskIntoConstraints = false
        row.addSubview(title)
        var constraints = [
            title.leadingAnchor.constraint(equalTo: row.leadingAnchor, constant: 12),
            title.topAnchor.constraint(equalTo: row.topAnchor),
            title.bottomAnchor.constraint(equalTo: row.bottomAnchor),
        ]
        if let accessory {
            accessory.translatesAutoresizingMaskIntoConstraints = false
            row.addSubview(accessory)
            constraints += [
                accessory.trailingAnchor.constraint(equalTo: row.trailingAnchor, constant: -12),
                accessory.centerYAnchor.constraint(equalTo: title.centerYAnchor),
                title.trailingAnchor.constraint(lessThanOrEqualTo: accessory.leadingAnchor, constant: -12),
            ]
        } else {
            constraints.append(title.trailingAnchor.constraint(lessThanOrEqualTo: row.trailingAnchor, constant: -12))
        }
        NSLayoutConstraint.activate(constraints)
        return row
    }

    /// A line of secondary text on the rows' leading edge, for loading, empty, and notes.
    func statusLine(_ text: String) -> NSView {
        let label = Build.label(text, font: .systemFont(ofSize: 13), color: .secondaryLabelColor, lines: 0)
        let row = NSView()
        row.translatesAutoresizingMaskIntoConstraints = false
        row.addSubview(label)
        label.pin(to: row, insets: NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12))
        return row
    }
}
