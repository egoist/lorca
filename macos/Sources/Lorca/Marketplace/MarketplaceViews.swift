import AppKit

/// A plugin or a bot, as the marketplace's rows list them.
enum MarketplaceItem {
    case plugin(MarketplacePlugin)
    case bot(BotTemplate)

    var category: String {
        switch self {
        case let .plugin(plugin): plugin.category
        case let .bot(template): template.category
        }
    }

    /// Every word of the query appears in its name, description, category, or maker.
    func matches(_ words: [String]) -> Bool {
        let fields: [String] =
            switch self {
            case let .plugin(plugin):
                [plugin.name, plugin.description, plugin.author, MarketplaceCategory.title(plugin.category)] + plugin.tags
            case let .bot(template):
                [template.name, template.summary, template.description, template.author, MarketplaceCategory.title(template.category)]
            }
        let text = fields.joined(separator: " ")
        return words.allSatisfy { text.localizedCaseInsensitiveContains($0) }
    }
}

extension Marketplace {
    /// The plugins, then the bots.
    var items: [MarketplaceItem] {
        plugins.map(MarketplaceItem.plugin) + bots.map(MarketplaceItem.bot)
    }
}

/// The sections the home page lists after the featured ones, as Grok Bot's filter categories.
enum MarketplaceCategory {
    static let order = [
        "credentials", "productivity", "communication", "design", "code", "data", "sales", "finance", "research", "support",
    ]

    static func title(_ key: String) -> String {
        switch key {
        case "credentials": L("Login and Credential Management")
        case "productivity": L("Productivity")
        case "communication": L("Communication")
        case "design": L("Design")
        case "code": L("Code")
        case "data": L("Data")
        case "sales": L("Sales")
        case "finance": L("Finance")
        case "research": L("Research")
        case "support": L("Support")
        case "": L("More")
        default: key.replacingOccurrences(of: "-", with: " ").capitalized
        }
    }

    /// The categories in use, known ones in their order, then the rest, then the uncategorized.
    static func keys(in items: [MarketplaceItem]) -> [String] {
        let used = Set(items.map(\.category))
        let known = order.filter(used.contains)
        let other = used.subtracting(order).subtracting([""]).sorted()
        return known + other + (used.contains("") ? [""] : [])
    }
}

// MARK: - Rows

/// A plugin's icon, as Grok Bot draws a plugin's logo: its real mark on a white tile
/// (`PluginLogo`), or its SF Symbol on a light tile with a hairline.
final class PluginIconView: BackgroundView {
    private let image = NSImageView()

    init(pluginID: String, symbol: String, size: CGFloat) {
        super.init()
        cornerRadius = size * 0.25
        image.translatesAutoresizingMaskIntoConstraints = false
        addSubview(image)
        var constraints = [
            widthAnchor.constraint(equalToConstant: size),
            heightAnchor.constraint(equalToConstant: size),
            image.centerXAnchor.constraint(equalTo: centerXAnchor),
            image.centerYAnchor.constraint(equalTo: centerYAnchor),
        ]
        if let tile = PluginLogo.tile(for: pluginID, size: size) {
            image.image = tile
            constraints += [image.widthAnchor.constraint(equalToConstant: size), image.heightAnchor.constraint(equalToConstant: size)]
        } else {
            fillColor = .textBackgroundColor
            borderColor = .separatorColor
            image.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
                ?? NSImage(systemSymbolName: "puzzlepiece.extension", accessibilityDescription: nil)
            image.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: size * 0.42, weight: .regular)
            image.contentTintColor = .labelColor
        }
        NSLayoutConstraint.activate(constraints)
    }
}

/// One entry in a marketplace grid: the icon or avatar, the name with its maker, a line about
/// it, and what can be done with it on the Runner. A click anywhere but the button opens it.
final class MarketplaceRow: NSView {
    private let hover = BackgroundView()
    private var pressed = false
    /// Set by the page for the row under the pointer.
    var isHovered = false { didSet { hover.fillColor = isHovered ? hoverFill : .clear } }
    var onOpen: (() -> Void)?
    /// The fill under the pointer; a row on a card's fill takes a darker one.
    var hoverFill: NSColor = Theme.botBubble

    init(media: NSView, title: String, byline: String?, subtitle: String, accessory: NSView?) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        hover.fillColor = .clear
        hover.cornerRadius = 10
        addSubview(hover)
        hover.pin(to: self)

        let titleLine = NSMutableAttributedString(
            string: title, attributes: [.font: NSFont.systemFont(ofSize: 13, weight: .semibold), .foregroundColor: NSColor.labelColor])
        if let byline, !byline.isEmpty {
            titleLine.append(NSAttributedString(
                string: "  " + byline,
                attributes: [.font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.secondaryLabelColor]))
        }
        let titleLabel = Build.label("", font: .systemFont(ofSize: 13, weight: .semibold))
        titleLabel.attributedStringValue = titleLine
        let subtitleLabel = Build.label(subtitle, font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor)
        let text = Build.stack([titleLabel, subtitleLabel], spacing: 2)
        text.setHuggingPriority(.defaultLow, for: .horizontal)

        media.translatesAutoresizingMaskIntoConstraints = false
        addSubview(media)
        addSubview(text)
        var constraints = [
            heightAnchor.constraint(equalToConstant: 64),
            media.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            media.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.leadingAnchor.constraint(equalTo: media.trailingAnchor, constant: 12),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            titleLabel.widthAnchor.constraint(lessThanOrEqualTo: text.widthAnchor),
            subtitleLabel.widthAnchor.constraint(lessThanOrEqualTo: text.widthAnchor),
        ]
        if let accessory {
            accessory.translatesAutoresizingMaskIntoConstraints = false
            accessory.setContentHuggingPriority(.required, for: .horizontal)
            accessory.setContentCompressionResistancePriority(.required, for: .horizontal)
            addSubview(accessory)
            constraints += [
                accessory.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
                accessory.centerYAnchor.constraint(equalTo: centerYAnchor),
                text.trailingAnchor.constraint(equalTo: accessory.leadingAnchor, constant: -12),
            ]
        } else {
            constraints.append(text.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12))
        }
        NSLayoutConstraint.activate(constraints)

        setAccessibilityElement(true)
        setAccessibilityRole(.button)
        setAccessibilityLabel([title, byline ?? "", subtitle].filter { !$0.isEmpty }.joined(separator: ", "))
        toolTip = subtitle
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func mouseDown(with event: NSEvent) {
        pressed = true
    }

    override func mouseUp(with event: NSEvent) {
        defer { pressed = false }
        guard pressed, bounds.contains(convert(event.locationInWindow, from: nil)) else { return }
        onOpen?()
    }

    override func accessibilityPerformPress() -> Bool {
        onOpen?()
        return onOpen != nil
    }
}

/// Rows two to a line, Grok Bot's grid: equal columns 8 apart, a line 2 below the one before.
final class MarketplaceGrid: NSView {
    init(rows: [NSView], columns: Int = 2) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let lines = Build.stack([], spacing: 2)
        lines.alignment = .leading
        addSubview(lines)
        lines.pin(to: self)
        for start in stride(from: 0, to: rows.count, by: columns) {
            var cells = Array(rows[start..<min(start + columns, rows.count)])
            // A short last line keeps its cells in their columns.
            while cells.count < columns {
                let filler = NSView()
                filler.translatesAutoresizingMaskIntoConstraints = false
                cells.append(filler)
            }
            let line = Build.stack(cells, orientation: .horizontal, spacing: 8)
            line.distribution = .fillEqually
            lines.addArrangedSubview(line)
            line.widthAnchor.constraint(equalTo: lines.widthAnchor).isActive = true
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

/// A section of the marketplace: its title with View all when there is more, over its grid.
final class MarketplaceSection: NSView {
    private var onViewAll: (() -> Void)?

    init(title: String, rows: [NSView], accessory: NSView? = nil, onViewAll: (() -> Void)? = nil) {
        self.onViewAll = onViewAll
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let heading = Build.label(title, font: .systemFont(ofSize: 14, weight: .semibold))
        let header = NSView()
        header.translatesAutoresizingMaskIntoConstraints = false
        header.addSubview(heading)
        var constraints = [
            header.heightAnchor.constraint(equalToConstant: 28),
            heading.leadingAnchor.constraint(equalTo: header.leadingAnchor, constant: 12),
            heading.centerYAnchor.constraint(equalTo: header.centerYAnchor),
        ]
        var trailing = accessory
        // The hover fill reaches past the words, so the words end where the rows' buttons do.
        var inset: CGFloat = 12
        if onViewAll != nil {
            trailing = HoverButton(title: L("View all"), target: self, action: #selector(viewAll))
            inset = 4
        }
        if let trailing {
            trailing.translatesAutoresizingMaskIntoConstraints = false
            header.addSubview(trailing)
            constraints += [
                trailing.trailingAnchor.constraint(equalTo: header.trailingAnchor, constant: -inset),
                trailing.centerYAnchor.constraint(equalTo: header.centerYAnchor),
                heading.trailingAnchor.constraint(lessThanOrEqualTo: trailing.leadingAnchor, constant: -8),
            ]
        }
        let grid = MarketplaceGrid(rows: rows)
        let stack = Build.stack([header, grid], spacing: 6)
        addSubview(stack)
        stack.pin(to: self)
        constraints += [
            header.widthAnchor.constraint(equalTo: stack.widthAnchor),
            grid.widthAnchor.constraint(equalTo: stack.widthAnchor),
        ]
        NSLayoutConstraint.activate(constraints)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func viewAll() {
        onViewAll?()
    }
}

/// A button with a closure, for the rows' Add and Connect. A page's main action is larger and
/// in the accent color, without taking Return: adding is never one keystroke away.
final class ActionButton: NSButton {
    private let handler: () -> Void

    init(title: String, prominent: Bool = false, action: @escaping () -> Void) {
        handler = action
        super.init(frame: .zero)
        self.title = title
        translatesAutoresizingMaskIntoConstraints = false
        bezelStyle = .push
        controlSize = prominent ? .large : .regular
        if prominent { bezelColor = .controlAccentColor }
        target = self
        self.action = #selector(tapped)
        setContentHuggingPriority(.required, for: .horizontal)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func tapped() { handler() }
}

extension MarketplaceViewController {
    /// A plugin's row: its tile, name, and line, and Add, Added, Connect, or Set Up on the
    /// picked Runner.
    func row(for plugin: MarketplacePlugin) -> MarketplaceRow {
        let row = MarketplaceRow(
            media: PluginIconView(pluginID: plugin.id, symbol: plugin.symbolName, size: 40), title: plugin.name, byline: nil,
            subtitle: plugin.description, accessory: accessory(for: plugin))
        row.onOpen = { [weak self] in self?.openPlugin(plugin) }
        return row
    }

    func row(for template: BotTemplate) -> MarketplaceRow {
        let avatar = AvatarView(diameter: 40)
        avatar.content = .bot(symbolName: template.symbolName, accent: template.accent)
        let add = ActionButton(title: L("Add")) { [weak self] in self?.add(template) }
        add.isEnabled = runner != nil
        add.toolTip = runner.map { L("Add %@ to %@", template.name, $0.name) } ?? L("Pair a Runner first.")
        let row = MarketplaceRow(
            media: avatar, title: template.name, byline: template.author.isEmpty ? nil : L("by %@", template.author),
            subtitle: template.summary, accessory: add)
        row.onOpen = { [weak self] in self?.openBot(template) }
        return row
    }

    func row(for item: MarketplaceItem) -> MarketplaceRow {
        switch item {
        case let .plugin(plugin): row(for: plugin)
        case let .bot(template): row(for: template)
        }
    }

    /// What a plugin's row offers on the picked Runner.
    func accessory(for plugin: MarketplacePlugin) -> NSView {
        if installing.contains(plugin.id) {
            let spinner = NSProgressIndicator()
            spinner.style = .spinning
            spinner.controlSize = .small
            spinner.startAnimation(nil)
            spinner.setAccessibilityLabel(L("Adding %@", plugin.name))
            return spinner
        }
        guard let installed = installedPlugin(plugin.id) else {
            let add = ActionButton(title: L("Add")) { [weak self] in self?.install(plugin) }
            add.isEnabled = runner != nil
            add.toolTip = runner.map { L("Install %@ on %@, for every bot there", plugin.name, $0.name) } ?? L("Pair a Runner first.")
            return add
        }
        switch installed.state {
        case .ready:
            let check = NSImageView(image: NSImage(systemSymbolName: "checkmark", accessibilityDescription: nil) ?? NSImage())
            check.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .semibold)
            check.contentTintColor = .systemGreen
            let label = Build.label(L("Added"), font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor)
            label.setContentCompressionResistancePriority(.required, for: .horizontal)
            return Build.stack([check, label], orientation: .horizontal, spacing: 5)
        case .needsAuth:
            return ActionButton(title: L("Connect")) { [weak self] in self?.manage(plugin.id) }
        case .needsSetup:
            return ActionButton(title: L("Set Up")) { [weak self] in self?.manage(plugin.id) }
        case .connecting, .error, .unknown:
            let label = Build.label(installed.detail, font: .systemFont(ofSize: 12), color: installed.stateColor)
            label.setContentCompressionResistancePriority(.required, for: .horizontal)
            return label
        }
    }
}
