import AppKit

/// A bot's Access: how far it may use each plugin on its Runner, down to single tools, and
/// whether it reads or changes files and runs shell commands there. Only the user changes it;
/// a chat's access request opens this sheet too.
final class BotAccessViewController: SheetViewController {
    private let store = AppStore.shared
    private let botID: Bot.ID
    private let runner: Device?
    private let saved: BotPermissions

    private let pluginsSection = SectionView(title: L("Plugins"))
    private let pluginsNote = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let runnerSection: SectionView
    private let files = SettingsPopUpButton()
    private let shell = NSSwitch()

    /// The plugins listed: the Runner's own list until it says what tools each one has.
    private var plugins: [BotAccessCatalog.Plugin] = []
    private var levels: [String: AccessLevel] = [:]
    /// The tools chosen for a plugin; none here is all of them, one it adds later included.
    private var chosenTools: [String: Set<String>] = [:]
    private var expanded: Set<String> = []
    private var rows: [String: PluginAccessRow] = [:]

    init(botID: Bot.ID) {
        self.botID = botID
        let bot = AppStore.shared.bot(botID)
        runner = bot.flatMap { AppStore.shared.device($0.runnerID) }
        saved = bot?.permissions ?? BotPermissions()
        // Files and shell commands are the Runner's, so its card goes by the Runner's name.
        runnerSection = SectionView(title: runner?.name ?? L("Runner"))
        super.init(title: L("Access for %@", bot?.name ?? ""), subtitle: "", width: 460)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        for level in [AccessLevel.write, .read, .none] {
            files.addItem(withTitle: level.title)
            files.lastItem?.representedObject = level.rawValue
        }
        files.selectItem(at: [AccessLevel.write, .read, .none].firstIndex(of: saved.filesystem) ?? 2)
        shell.controlSize = .small
        shell.state = saved.shell ? .on : .off
        shell.setAccessibilityLabel(L("Shell commands"))
        runnerSection.setRows([AccessoryRow(key: L("Files"), accessory: files), AccessoryRow(key: L("Shell commands"), accessory: shell)])
        let boundary = Build.label(
            L("Shell commands run as you on %@ and can reach anything you can there.", runner?.name ?? L("its Runner")),
            font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)

        for view in [pluginsSection, pluginsNote, runnerSection, boundary] {
            contentStack.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }
        contentStack.setCustomSpacing(16, after: pluginsSection)
        contentStack.setCustomSpacing(16, after: pluginsNote)
        contentStack.setCustomSpacing(6, after: runnerSection)
        pluginsNote.isHidden = true
        setButtons(confirm: L("Save"))

        // The Runner's own list first, so the sheet opens complete while it is asked for tools.
        show((runner?.plugins ?? []).map { BotAccessCatalog.Plugin(id: $0.id, name: $0.name, tools: []) })
        Task { [weak self] in
            guard let self else { return }
            do {
                let catalog = try await store.botAccessCatalog(botID)
                show(catalog.connections)
            } catch {
                pluginsNote.stringValue = L("Couldn't get the tools from %@.", runner?.name ?? L("its Runner"))
                pluginsNote.isHidden = plugins.isEmpty
                contentStack.setCustomSpacing(6, after: pluginsSection)
                fitSheetToContent()
            }
        }
    }

    /// Lists `listed`, keeping what the user already chose for a plugin listed before.
    private func show(_ listed: [BotAccessCatalog.Plugin]) {
        plugins = listed
        for plugin in listed where levels[plugin.id] == nil {
            levels[plugin.id] = saved.level(of: plugin.id)
            chosenTools[plugin.id] = saved.connections?[plugin.id]?.tools
        }
        pluginsSection.isHidden = listed.isEmpty
        render()
    }

    private func render() {
        var views: [NSView] = []
        for plugin in plugins {
            let row = rows[plugin.id] ?? makeRow(for: plugin)
            rows[plugin.id] = row
            let level = levels[plugin.id] ?? .none
            if plugin.tools.isEmpty || level == .none { expanded.remove(plugin.id) }
            configure(row, for: plugin)
            views.append(row)
            if expanded.contains(plugin.id) {
                let list = ToolChecklist(tools: plugin.tools, chosen: chosenTools[plugin.id], level: level)
                list.onChange = { [weak self, weak row] chosen in
                    guard let self, let row else { return }
                    // Every tool chosen is all of them, a tool the plugin adds later included.
                    self.chosenTools[plugin.id] = Set(plugin.tools.map(\.name)).isSubset(of: chosen) ? nil : chosen
                    self.configure(row, for: plugin)
                }
                views.append(list)
            }
        }
        pluginsSection.setRows(views)
        fitSheetToContent()
    }

    private func configure(_ row: PluginAccessRow, for plugin: BotAccessCatalog.Plugin) {
        let level = levels[plugin.id] ?? .none
        row.configure(
            plugin: plugin, level: level, toolsSummary: toolsSummary(of: plugin, level: level),
            canExpand: !plugin.tools.isEmpty && level != .none, isExpanded: expanded.contains(plugin.id))
    }

    private func makeRow(for plugin: BotAccessCatalog.Plugin) -> PluginAccessRow {
        let row = PluginAccessRow()
        // A named account (Gmail · Work) has its service's mark.
        let installed = runner?.plugins.first { $0.id == plugin.id }
        row.icon = PluginLogo.tile(for: installed?.marketplaceID ?? plugin.id, size: 18)
            ?? NSImage(systemSymbolName: installed?.symbolName ?? "puzzlepiece.extension", accessibilityDescription: nil)
        row.onLevel = { [weak self] level in
            self?.levels[plugin.id] = level
            self?.render()
        }
        row.onToggle = { [weak self] in
            guard let self else { return }
            if !self.expanded.insert(plugin.id).inserted { self.expanded.remove(plugin.id) }
            self.render()
        }
        return row
    }

    /// "All tools", or how many of them the bot may use. Nothing before the plugin has
    /// connected once, or when it is off.
    private func toolsSummary(of plugin: BotAccessCatalog.Plugin, level: AccessLevel) -> String {
        guard !plugin.tools.isEmpty, level != .none else { return "" }
        guard let chosen = chosenTools[plugin.id] else { return L("All tools") }
        return L("%d of %d tools", plugin.tools.filter { chosen.contains($0.name) }.count, plugin.tools.count)
    }

    override func confirmTapped() {
        var policy = BotPermissions()
        // Every plugin fully open is the Runner's every plugin, one installed later included.
        let open = plugins.allSatisfy { levels[$0.id] == .write && chosenTools[$0.id] == nil }
        if !open {
            var connections: [String: BotPermissions.Connection] = [:]
            for plugin in plugins where levels[plugin.id] != AccessLevel.none {
                connections[plugin.id] = .init(capabilities: levels[plugin.id]?.capabilities ?? [], tools: chosenTools[plugin.id])
            }
            policy.connections = connections
        }
        policy.filesystem = (files.selectedItem?.representedObject as? String).flatMap(AccessLevel.init(rawValue:)) ?? .none
        policy.shell = shell.state == .on
        if policy != saved { store.setBotPermissions(botID, policy) }
        dismiss(nil)
    }
}

/// A plugin in the Access sheet: its logo and name, how many of its tools the bot may use,
/// and how far. A click on the row shows the tools.
private final class PluginAccessRow: NSView {
    private let disclosure = NSButton()
    private let iconView = NSImageView()
    private let name = Build.label("", font: .systemFont(ofSize: 12.5, weight: .medium))
    private let detail = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private let level = SettingsPopUpButton()
    private var canExpand = false
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    var onLevel: ((AccessLevel) -> Void)?
    var onToggle: (() -> Void)?
    var icon: NSImage? {
        get { iconView.image }
        set { iconView.image = newValue }
    }

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        disclosure.bezelStyle = .disclosure
        disclosure.setButtonType(.pushOnPushOff)
        disclosure.title = ""
        disclosure.target = self
        disclosure.action = #selector(toggle)
        disclosure.translatesAutoresizingMaskIntoConstraints = false
        iconView.translatesAutoresizingMaskIntoConstraints = false
        iconView.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 13, weight: .regular)
        iconView.contentTintColor = .secondaryLabelColor
        level.target = self
        level.action = #selector(levelChanged)
        level.translatesAutoresizingMaskIntoConstraints = false
        name.lineBreakMode = .byTruncatingTail
        name.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        let text = Build.stack([name, detail], spacing: 1)
        for view in [disclosure, iconView, text, level] as [NSView] { addSubview(view) }
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            disclosure.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 6),
            disclosure.centerYAnchor.constraint(equalTo: centerYAnchor),
            disclosure.widthAnchor.constraint(equalToConstant: 16),
            iconView.leadingAnchor.constraint(equalTo: disclosure.trailingAnchor, constant: 4),
            iconView.centerYAnchor.constraint(equalTo: centerYAnchor),
            iconView.widthAnchor.constraint(equalToConstant: 18),
            iconView.heightAnchor.constraint(equalToConstant: 18),
            text.leadingAnchor.constraint(equalTo: iconView.trailingAnchor, constant: 10),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.topAnchor.constraint(greaterThanOrEqualTo: topAnchor, constant: 8),
            text.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -8),
            text.trailingAnchor.constraint(lessThanOrEqualTo: level.leadingAnchor, constant: -8),
            level.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            level.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The levels it offers: Read and draft only for a plugin with a tool that drafts.
    func configure(plugin: BotAccessCatalog.Plugin, level current: AccessLevel, toolsSummary: String, canExpand: Bool, isExpanded: Bool) {
        name.stringValue = plugin.name
        detail.stringValue = toolsSummary
        detail.isHidden = toolsSummary.isEmpty
        let drafts = plugin.tools.contains { $0.capability == "draft" } || current == .draft
        level.removeAllItems()
        for choice in AccessLevel.allCases where choice != .draft || drafts {
            level.addItem(withTitle: choice.title)
            level.lastItem?.representedObject = choice.rawValue
        }
        level.selectItem(withTitle: current.title)
        level.setAccessibilityLabel(L("Access to %@", plugin.name))
        self.canExpand = canExpand
        disclosure.isHidden = !canExpand
        disclosure.state = isExpanded ? .on : .off
        disclosure.setAccessibilityLabel(L("%@ tools", plugin.name))
    }

    @objc private func levelChanged() {
        guard let raw = level.selectedItem?.representedObject as? String, let choice = AccessLevel(rawValue: raw) else { return }
        onLevel?(choice)
    }

    @objc private func toggle() {
        onToggle?()
    }

    // A click anywhere on the row but its pop-up shows or hides the tools.
    override func mouseDown(with event: NSEvent) {
        guard canExpand else { return super.mouseDown(with: event) }
        onToggle?()
    }

    override var allowsVibrancy: Bool { false }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { isHovered = canExpand }
    override func mouseExited(with event: NSEvent) { isHovered = false }

    override func draw(_ dirtyRect: NSRect) {
        guard isHovered else { return }
        NSColor.labelColor.withAlphaComponent(0.05).setFill()
        bounds.fill()
    }
}

/// A plugin's tools, each with a checkbox for whether the bot may use it and what it does. A
/// tool beyond the plugin's level stays off until the level reaches it. Long lists scroll.
private final class ToolChecklist: NSView {
    private let tools: [BotAccessCatalog.Plugin.Tool]
    private var chosen: Set<String>
    var onChange: ((Set<String>) -> Void)?

    init(tools: [BotAccessCatalog.Plugin.Tool], chosen: Set<String>?, level: AccessLevel) {
        self.tools = tools
        self.chosen = chosen ?? Set(tools.map(\.name))
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let rows = tools.enumerated().map { index, tool in row(tool, index: index, level: level) }
        let stack = Build.stack(rows, spacing: 0)
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(stack)
        let scroll = NSScrollView()
        scroll.documentView = document
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        addSubview(scroll)
        scroll.pin(to: self)
        NSLayoutConstraint.activate([
            scroll.heightAnchor.constraint(equalToConstant: min(CGFloat(tools.count) * Self.rowHeight + 10, 168)),
            document.widthAnchor.constraint(equalTo: scroll.widthAnchor),
            stack.topAnchor.constraint(equalTo: document.topAnchor, constant: 5),
            stack.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            stack.bottomAnchor.constraint(equalTo: document.bottomAnchor, constant: -5),
        ])
        for row in stack.arrangedSubviews {
            row.widthAnchor.constraint(equalTo: stack.widthAnchor).isActive = true
            row.heightAnchor.constraint(equalToConstant: Self.rowHeight).isActive = true
        }
    }

    private static let rowHeight: CGFloat = 24

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func row(_ tool: BotAccessCatalog.Plugin.Tool, index: Int, level: AccessLevel) -> NSView {
        let capability = tool.capability ?? "write"
        let reached = level.allows(capability)
        let box = NSButton(checkboxWithTitle: tool.title ?? tool.name, target: self, action: #selector(toggled(_:)))
        box.font = .systemFont(ofSize: 12)
        box.tag = index
        box.state = reached && chosen.contains(tool.name) ? .on : .off
        box.isEnabled = reached
        box.toolTip = tool.description
        box.lineBreakMode = .byTruncatingTail
        box.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        let does = Build.label(
            [("read", L("Reads")), ("draft", L("Drafts")), ("write", L("Changes"))].first { $0.0 == capability }?.1 ?? L("Changes"),
            font: .systemFont(ofSize: 11), color: .tertiaryLabelColor)
        does.setContentCompressionResistancePriority(.required, for: .horizontal)
        let spacer = NSView()
        spacer.setContentHuggingPriority(.init(1), for: .horizontal)
        let row = Build.stack([box, spacer, does], orientation: .horizontal, spacing: 8)
        // The boxes line up under the plugin's name.
        row.edgeInsets = NSEdgeInsets(top: 0, left: 52, bottom: 0, right: 14)
        return row
    }

    @objc private func toggled(_ sender: NSButton) {
        guard tools.indices.contains(sender.tag) else { return }
        let name = tools[sender.tag].name
        if sender.state == .on { chosen.insert(name) } else { chosen.remove(name) }
        onChange?(chosen)
    }
}
