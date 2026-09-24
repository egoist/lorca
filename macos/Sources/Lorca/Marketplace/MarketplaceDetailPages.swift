import AppKit

/// A plugin in full, as Grok Bot's plugin page: what it does, its servers and skills, the setup
/// it asks for, and who makes it, with Add, or its state and Manage once the Runner has it.
final class MarketplacePluginPage: MarketplacePage {
    private let pluginID: MarketplacePlugin.ID

    init(market: MarketplaceViewController, pluginID: MarketplacePlugin.ID) {
        self.pluginID = pluginID
        super.init(market: market)
    }

    override func reload() {
        guard let plugin = market.plugin(pluginID) else {
            setContent([statusLine(market.loading == .loaded ? L("This plugin is no longer in the marketplace.") : L("Loading…"))])
            return
        }
        var views: [NSView] = [header(for: plugin)]
        if let runner = market.runner, let installed = market.installedPlugin(plugin.id) {
            let section = card(L("On %@", runner.name))
            let row = StatusRow()
            let action: String? =
                switch installed.state {
                case .needsAuth: L("Connect")
                case .needsSetup: L("Set Up")
                default: nil
                }
            row.configure(
                symbol: runner.symbolName, title: installed.detail, subtitle: L("Every bot on %@ can use it.", runner.name),
                state: nil, actionTitle: action)
            row.onAction = { [weak self] in self?.market.manage(plugin.id) }
            section.setRows([row])
            views.append(section)
        }
        if !plugin.servers.isEmpty {
            let section = card(L("Servers"), count: plugin.servers.count)
            section.setRows(plugin.servers.map { server in
                let row = StatusRow()
                let place =
                    server.isRemote
                    ? [URL(string: server.address)?.host ?? server.address, server.signsIn ? L("Signs in with your account") : nil]
                        .compactMap { $0 }.joined(separator: " · ")
                    : L("Runs on the Runner: %@", server.address)
                row.configure(symbol: server.isRemote ? "network" : "terminal", title: server.name, subtitle: place, state: nil)
                return row
            })
            views.append(section)
        }
        if !plugin.skills.isEmpty {
            let section = card(L("Skills"), count: plugin.skills.count)
            section.setRows(plugin.skills.map { skill in
                let row = StatusRow()
                row.configure(symbol: "cube", title: skill.name, subtitle: skill.description, state: nil)
                return row
            })
            views.append(section)
        }
        if !plugin.variables.isEmpty {
            let section = card(L("Setup", context: "plugin variables"))
            section.setRows(plugin.variables.map { variable in
                let row = StatusRow()
                row.configure(
                    symbol: variable.secret ? "key" : "slider.horizontal.3", title: variable.name, subtitle: variable.description,
                    state: variable.required ? L("Required") : L("Optional"))
                return row
            })
            views.append(section)
        }
        let information = card(L("Information"))
        var rows: [NSView] = []
        if !plugin.author.isEmpty { rows.append(KeyValueRow(key: L("Developer"), value: plugin.author)) }
        if !plugin.category.isEmpty { rows.append(KeyValueRow(key: L("Category"), value: MarketplaceCategory.title(plugin.category))) }
        if let homepage = plugin.homepage, let url = URL(string: homepage) {
            rows.append(KeyValueRow(key: L("Website"), value: url.host ?? homepage))
        }
        if !rows.isEmpty {
            information.setRows(rows)
            views.append(information)
        }
        setContent(views, spacing: 22)
    }

    private func header(for plugin: MarketplacePlugin) -> NSView {
        let icon = PluginIconView(pluginID: plugin.id, symbol: plugin.symbolName, size: 56)
        let name = Build.label(plugin.name, font: .systemFont(ofSize: 20, weight: .semibold))
        var bylineParts: [String] = []
        if !plugin.author.isEmpty { bylineParts.append(L("by %@", plugin.author)) }
        if !plugin.category.isEmpty { bylineParts.append(MarketplaceCategory.title(plugin.category)) }
        let byline = Build.label(bylineParts.joined(separator: " · "), font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor)
        let titles = Build.stack([name, byline], spacing: 3)

        var actions: [NSView] = []
        if let homepage = plugin.homepage, let url = URL(string: homepage) {
            let website = ActionButton(title: L("Website")) { NSWorkspace.shared.open(url) }
            website.image = NSImage(systemSymbolName: "arrow.up.right", accessibilityDescription: nil)
            website.imagePosition = .imageTrailing
            website.toolTip = homepage
            actions.append(website)
        }
        if market.installing.contains(plugin.id) {
            let spinner = NSProgressIndicator()
            spinner.style = .spinning
            spinner.controlSize = .small
            spinner.startAnimation(nil)
            actions.append(spinner)
        } else if market.installedPlugin(plugin.id) != nil {
            actions.append(ActionButton(title: L("Manage…")) { [weak self] in self?.market.manage(plugin.id) })
        } else {
            let add = ActionButton(title: L("Add"), prominent: true) { [weak self] in self?.market.install(plugin) }
            add.isEnabled = market.runner != nil
            add.toolTip = market.runner.map { L("Install %@ on %@, for every bot there", plugin.name, $0.name) } ?? L("Pair a Runner first.")
            actions.append(add)
        }
        let buttons = Build.stack(actions, orientation: .horizontal, spacing: 8)

        let top = NSView()
        top.translatesAutoresizingMaskIntoConstraints = false
        for view in [icon, titles, buttons] as [NSView] { top.addSubview(view) }
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: top.leadingAnchor, constant: 12),
            icon.topAnchor.constraint(equalTo: top.topAnchor),
            icon.bottomAnchor.constraint(equalTo: top.bottomAnchor),
            titles.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 14),
            titles.centerYAnchor.constraint(equalTo: icon.centerYAnchor),
            buttons.trailingAnchor.constraint(equalTo: top.trailingAnchor, constant: -12),
            buttons.centerYAnchor.constraint(equalTo: icon.centerYAnchor),
            titles.trailingAnchor.constraint(lessThanOrEqualTo: buttons.leadingAnchor, constant: -12),
        ])
        let description = Build.label(plugin.description, font: .systemFont(ofSize: 13), color: .secondaryLabelColor, lines: 0)
        description.isSelectable = true
        let text = NSView()
        text.translatesAutoresizingMaskIntoConstraints = false
        text.addSubview(description)
        description.pin(to: text, insets: NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12))
        let stack = Build.stack([top, text], spacing: 14)
        top.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        text.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        return stack
    }

    /// A card in System Settings' look, its count beside the title.
    private func card(_ title: String, count: Int? = nil) -> SectionView {
        let section = SectionView(title: title)
        section.style = .heading
        if let count {
            section.setHeaderAccessory(Build.label(String(count), font: .systemFont(ofSize: 12), color: .secondaryLabelColor))
        }
        return section
    }
}

/// A bot in full, as Grok Bot's bot page: who it is with Add Bot, then its instructions, what it
/// already knows, its routines, and its plugins, one at a time from the list beside them.
final class MarketplaceBotPage: MarketplacePage {
    enum Part {
        case instructions, memories, routines, plugins

        var title: String {
            switch self {
            case .instructions: L("Instructions")
            case .memories: L("Memories")
            case .routines: L("Routines")
            case .plugins: L("Plugins")
            }
        }

        var subtitle: String {
            switch self {
            case .instructions: L("How this bot should work")
            case .memories: L("Facts it already knows")
            case .routines: L("Jobs that run on their own")
            case .plugins: L("What it works with")
            }
        }
    }

    private let templateID: BotTemplate.ID
    private var selected = Part.instructions

    init(market: MarketplaceViewController, templateID: BotTemplate.ID) {
        self.templateID = templateID
        super.init(market: market)
    }

    override func reload() {
        guard let template = market.catalog.bots.first(where: { $0.id == templateID }) else {
            setContent([statusLine(market.loading == .loaded ? L("This bot is no longer in the marketplace.") : L("Loading…"))])
            return
        }
        var parts: [Part] = [.instructions]
        if !template.memory.isEmpty { parts.append(.memories) }
        if !template.routines.isEmpty { parts.append(.routines) }
        if !template.plugins.isEmpty { parts.append(.plugins) }
        if !parts.contains(selected) { selected = .instructions }

        let divider = HairlineView()
        let rail = Build.stack(
            parts.map { part in
                RailItem(title: part.title, subtitle: part.subtitle, isSelected: part == selected) { [weak self] in
                    self?.selected = part
                    self?.reload()
                }
            }, spacing: 2)
        let panel = BackgroundView()
        panel.fillColor = Theme.botBubble
        panel.cornerRadius = 12
        let panelContent = self.panel(for: selected, of: template)
        panel.addSubview(panelContent)
        panelContent.pin(to: panel, insets: NSEdgeInsets(top: 18, left: 20, bottom: 20, right: 20))

        let body = NSView()
        body.translatesAutoresizingMaskIntoConstraints = false
        body.addSubview(rail)
        body.addSubview(panel)
        NSLayoutConstraint.activate([
            rail.leadingAnchor.constraint(equalTo: body.leadingAnchor),
            rail.topAnchor.constraint(equalTo: body.topAnchor),
            rail.widthAnchor.constraint(equalToConstant: 196),
            rail.bottomAnchor.constraint(lessThanOrEqualTo: body.bottomAnchor),
            panel.leadingAnchor.constraint(equalTo: rail.trailingAnchor, constant: 16),
            panel.trailingAnchor.constraint(equalTo: body.trailingAnchor),
            panel.topAnchor.constraint(equalTo: body.topAnchor),
            panel.bottomAnchor.constraint(lessThanOrEqualTo: body.bottomAnchor),
            panel.heightAnchor.constraint(greaterThanOrEqualTo: rail.heightAnchor),
            body.heightAnchor.constraint(greaterThanOrEqualTo: panel.heightAnchor),
        ])
        for item in rail.arrangedSubviews {
            item.widthAnchor.constraint(equalTo: rail.widthAnchor).isActive = true
        }
        setContent([header(for: template), divider, body], spacing: 20)
    }

    private func header(for template: BotTemplate) -> NSView {
        let avatar = AvatarView(diameter: 64)
        avatar.content = .bot(symbolName: template.symbolName, accent: template.accent)
        avatar.translatesAutoresizingMaskIntoConstraints = false
        let name = Build.label(template.name, font: .systemFont(ofSize: 22, weight: .semibold))
        let byline = Build.label(
            template.author.isEmpty ? "" : L("By %@", template.author), font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor)
        byline.isHidden = template.author.isEmpty
        let add = ActionButton(title: L("Add Bot"), prominent: true) { [weak self] in self?.market.add(template) }
        add.isEnabled = market.runner != nil
        let summary = Build.label(template.summary, font: .systemFont(ofSize: 14), lines: 0)
        let note = Build.label(
            market.runner.map { L("Adds %@ to %@. Its routines start paused, and it asks before it installs a plugin.", template.name, $0.name) }
                ?? L("Pair a Runner first: a bot runs on a computer."),
            font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)

        let top = NSView()
        top.translatesAutoresizingMaskIntoConstraints = false
        let titles = Build.stack([name, byline], spacing: 3)
        for view in [avatar, titles, add] as [NSView] { top.addSubview(view) }
        NSLayoutConstraint.activate([
            avatar.leadingAnchor.constraint(equalTo: top.leadingAnchor, constant: 12),
            avatar.topAnchor.constraint(equalTo: top.topAnchor),
            titles.leadingAnchor.constraint(equalTo: top.leadingAnchor, constant: 12),
            titles.topAnchor.constraint(equalTo: avatar.bottomAnchor, constant: 14),
            titles.bottomAnchor.constraint(equalTo: top.bottomAnchor),
            add.trailingAnchor.constraint(equalTo: top.trailingAnchor, constant: -12),
            add.centerYAnchor.constraint(equalTo: name.centerYAnchor),
            titles.trailingAnchor.constraint(lessThanOrEqualTo: add.leadingAnchor, constant: -12),
        ])
        let text = Build.stack([summary, note], spacing: 8)
        let textRow = NSView()
        textRow.translatesAutoresizingMaskIntoConstraints = false
        textRow.addSubview(text)
        text.pin(to: textRow, insets: NSEdgeInsets(top: 0, left: 12, bottom: 0, right: 12))
        summary.widthAnchor.constraint(equalTo: text.widthAnchor).isActive = true
        note.widthAnchor.constraint(equalTo: text.widthAnchor).isActive = true
        let stack = Build.stack([top, textRow], spacing: 12)
        top.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        textRow.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        return stack
    }

    /// What the panel beside the list shows for the part picked there.
    private func panel(for part: Part, of template: BotTemplate) -> NSView {
        func paragraph(_ text: String, color: NSColor = .secondaryLabelColor) -> NSTextField {
            let label = Build.label(text, font: .systemFont(ofSize: 13), color: color, lines: 0)
            label.isSelectable = true
            return label
        }
        var views: [NSView] = []
        switch part {
        case .instructions:
            views = [paragraph(template.description, color: .labelColor)]
        case .memories:
            views = template.memory.map { paragraph($0, color: .labelColor) }
        case .routines:
            for routine in template.routines {
                let name = Build.label(routine.name, font: .systemFont(ofSize: 13, weight: .semibold))
                let schedule = Build.label(Format.schedule(routine.scheduleText), font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
                views.append(Build.stack([name, schedule, paragraph(routine.prompt)], spacing: 3))
            }
            views.append(paragraph(L("They start paused. %@ asks whether to turn them on.", template.name), color: .tertiaryLabelColor))
        case .plugins:
            for id in template.plugins {
                guard let plugin = market.plugin(id) else { continue }
                let state: String
                let color: NSColor
                if let installed = market.installedPlugin(plugin.id) {
                    state = installed.state == .ready ? L("Installed") : installed.detail
                    color = installed.state == .ready ? .systemGreen : installed.stateColor
                } else {
                    state = L("Not installed")
                    color = .secondaryLabelColor
                }
                let label = Build.label(state, font: .systemFont(ofSize: 12), color: color)
                let row = MarketplaceRow(
                    media: PluginIconView(pluginID: plugin.id, symbol: plugin.symbolName, size: 36), title: plugin.name, byline: nil,
                    subtitle: plugin.description, accessory: label)
                row.hoverFill = Theme.codeBackgroundHover
                row.onOpen = { [weak self] in self?.market.openPlugin(plugin) }
                views.append(row)
            }
            if let runner = market.runner {
                views.append(paragraph(L("%@ asks before it installs one on %@.", template.name, runner.name), color: .tertiaryLabelColor))
            }
        }
        let stack = Build.stack(views, spacing: part == .plugins ? 4 : 16)
        for view in views {
            view.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
        }
        return stack
    }
}

/// An entry in the bot page's list: a title over a line, with the picked one on a fill.
private final class RailItem: NSView {
    private let handler: () -> Void

    init(title: String, subtitle: String, isSelected: Bool, action: @escaping () -> Void) {
        handler = action
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let fill = BackgroundView()
        fill.cornerRadius = 10
        fill.fillColor = isSelected ? Theme.botBubble : .clear
        addSubview(fill)
        fill.pin(to: self)
        let titleLabel = Build.label(title, font: .systemFont(ofSize: 13, weight: isSelected ? .semibold : .regular))
        let subtitleLabel = Build.label(subtitle, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        let text = Build.stack([titleLabel, subtitleLabel], spacing: 2)
        addSubview(text)
        text.pin(to: self, insets: NSEdgeInsets(top: 10, left: 14, bottom: 10, right: 10))
        setAccessibilityElement(true)
        setAccessibilityRole(.button)
        setAccessibilityLabel(title)
        setAccessibilitySelected(isSelected)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func mouseUp(with event: NSEvent) {
        guard bounds.contains(convert(event.locationInWindow, from: nil)) else { return }
        handler()
    }

    override func mouseDown(with event: NSEvent) {}

    override func accessibilityPerformPress() -> Bool {
        handler()
        return true
    }
}
