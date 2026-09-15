import AppKit

/// Titled card used by the inspector and the Device pane.
final class SectionView: NSView {
    private let header: NSTextField
    private let card = BackgroundView()
    private let rows = Build.stack([], spacing: 0)

    init(title: String) {
        header = Build.label(
            title.uppercased(), font: .systemFont(ofSize: 10, weight: .semibold),
            color: .tertiaryLabelColor)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        let cornerRadius: CGFloat = 9
        card.cornerRadius = cornerRadius
        card.fillColor = Theme.botBubble
        card.borderColor = Theme.botBubbleBorder

        rows.orientation = .vertical
        rows.alignment = .leading
        rows.distribution = .fill
        // Row hover fills are plain rectangles; the stack fills the card, so clipping it to the
        // same radius keeps them inside the corners.
        rows.wantsLayer = true
        rows.layer?.cornerRadius = cornerRadius
        rows.layer?.masksToBounds = true

        addSubview(header)
        addSubview(card)
        card.addSubview(rows)

        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: topAnchor),
            header.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 4),
            header.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),

            card.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 6),
            card.leadingAnchor.constraint(equalTo: leadingAnchor),
            card.trailingAnchor.constraint(equalTo: trailingAnchor),
            card.bottomAnchor.constraint(equalTo: bottomAnchor),

            rows.topAnchor.constraint(equalTo: card.topAnchor),
            rows.leadingAnchor.constraint(equalTo: card.leadingAnchor),
            rows.trailingAnchor.constraint(equalTo: card.trailingAnchor),
            rows.bottomAnchor.constraint(equalTo: card.bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func setRows(_ views: [NSView]) {
        for view in rows.arrangedSubviews {
            rows.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        for (index, view) in views.enumerated() {
            if index > 0 {
                let divider = HairlineView()
                rows.addArrangedSubview(divider)
                divider.widthAnchor.constraint(equalTo: rows.widthAnchor).isActive = true
            }
            rows.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: rows.widthAnchor).isActive = true
        }
    }
}

final class KeyValueRow: NSView {
    private let key: NSTextField
    private let value: NSTextField

    init(key keyText: String, value valueText: String, monospaced: Bool = false, tint: NSColor? = nil) {
        key = Build.label(keyText, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        value = Build.label(
            valueText,
            font: monospaced ? .monospacedSystemFont(ofSize: 11, weight: .regular) : .systemFont(ofSize: 12),
            color: tint ?? .labelColor,
            alignment: .right
        )
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        key.setContentCompressionResistancePriority(.required, for: .horizontal)
        value.isSelectable = true

        addSubview(key)
        addSubview(value)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 32),
            key.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            key.centerYAnchor.constraint(equalTo: centerYAnchor),
            value.leadingAnchor.constraint(greaterThanOrEqualTo: key.trailingAnchor, constant: 10),
            value.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            value.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

/// Bot line used in the inspector, the Device pane and pickers.
final class BotRow: NSView {
    private let avatar = AvatarView(diameter: 28)
    private let name = Build.label("", font: .systemFont(ofSize: 13, weight: .medium))
    private let detail = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private let accessory = NSButton()
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    var onAccessory: (() -> Void)?
    var onClick: (() -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true

        accessory.isBordered = false
        accessory.bezelStyle = .accessoryBarAction
        accessory.target = self
        accessory.action = #selector(accessoryTapped)
        accessory.translatesAutoresizingMaskIntoConstraints = false
        accessory.isHidden = true

        let text = Build.stack([name, detail], spacing: 1)
        addSubview(avatar)
        addSubview(text)
        addSubview(accessory)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 46),
            avatar.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            avatar.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.leadingAnchor.constraint(equalTo: avatar.trailingAnchor, constant: 9),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.trailingAnchor.constraint(lessThanOrEqualTo: accessory.leadingAnchor, constant: -8),
            accessory.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            accessory.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @discardableResult
    func configure(bot: Bot, detailText: String, accessorySymbol: String? = nil, tooltip: String = "")
        -> BotRow
    {
        avatar.content = .bot(symbolName: bot.symbolName, accent: bot.accent)
        name.stringValue = bot.name
        detail.stringValue = detailText
        if let accessorySymbol {
            accessory.image = NSImage(systemSymbolName: accessorySymbol, accessibilityDescription: tooltip)
            accessory.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .medium)
            accessory.contentTintColor = .tertiaryLabelColor
            accessory.toolTip = tooltip
            accessory.isHidden = false
        } else {
            accessory.isHidden = true
        }
        return self
    }

    @objc private func accessoryTapped() {
        onAccessory?()
    }

    override var allowsVibrancy: Bool { false }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { isHovered = onClick != nil }
    override func mouseExited(with event: NSEvent) { isHovered = false }
    override func mouseUp(with event: NSEvent) { onClick?() }

    override func draw(_ dirtyRect: NSRect) {
        guard isHovered else { return }
        NSColor.labelColor.withAlphaComponent(0.05).setFill()
        bounds.fill()
    }
}

/// Row with a leading symbol, a title/subtitle pair and a trailing state pill.
final class StatusRow: NSView {
    private let icon = NSImageView()
    private let title = Build.label("", font: .systemFont(ofSize: 12.5, weight: .medium))
    private let subtitle = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private let state = Build.label("", font: .systemFont(ofSize: 11, weight: .medium), alignment: .right)
    private let action = NSButton()

    var onAction: (() -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.contentTintColor = .secondaryLabelColor

        action.bezelStyle = .rounded
        action.controlSize = .small
        action.target = self
        action.action = #selector(actionTapped)
        action.translatesAutoresizingMaskIntoConstraints = false
        action.isHidden = true

        let text = Build.stack([title, subtitle], spacing: 1)
        addSubview(icon)
        addSubview(text)
        addSubview(state)
        addSubview(action)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 18),
            text.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 10),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.trailingAnchor.constraint(lessThanOrEqualTo: state.leadingAnchor, constant: -8),
            state.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            state.centerYAnchor.constraint(equalTo: centerYAnchor),
            action.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            action.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(
        symbol: String,
        title titleText: String,
        subtitle subtitleText: String,
        state stateText: String?,
        stateColor: NSColor = .secondaryLabelColor,
        actionTitle: String? = nil
    ) {
        icon.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 13, weight: .regular)
        title.stringValue = titleText
        subtitle.stringValue = subtitleText
        state.stringValue = stateText ?? ""
        state.textColor = stateColor
        state.isHidden = stateText == nil

        if let actionTitle {
            action.title = actionTitle
            action.isHidden = false
            state.isHidden = true
        } else {
            action.isHidden = true
        }
    }

    @objc private func actionTapped() {
        onAction?()
    }
}


/// Label on the left, a pop-up on the right. Used for settings inside a section card.
final class PopUpRow: NSView {
    private let key: NSTextField
    let popUp = NSPopUpButton()
    var onChange: ((Int) -> Void)?

    init(key keyText: String, items: [String], selected: Int) {
        key = Build.label(keyText, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        popUp.translatesAutoresizingMaskIntoConstraints = false
        popUp.controlSize = .small
        popUp.font = .systemFont(ofSize: 11)
        popUp.addItems(withTitles: items)
        if items.indices.contains(selected) { popUp.selectItem(at: selected) }
        popUp.target = self
        popUp.action = #selector(changed)
        popUp.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        popUp.lineBreakMode = .byTruncatingTail
        key.setContentCompressionResistancePriority(.required, for: .horizontal)

        addSubview(key)
        addSubview(popUp)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 34),
            key.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            key.centerYAnchor.constraint(equalTo: centerYAnchor),
            popUp.leadingAnchor.constraint(greaterThanOrEqualTo: key.trailingAnchor, constant: 10),
            popUp.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -8),
            popUp.centerYAnchor.constraint(equalTo: centerYAnchor),
            popUp.widthAnchor.constraint(equalToConstant: 150),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func changed() {
        onChange?(popUp.indexOfSelectedItem)
    }
}


/// Key on the left, a status value on the right, and an inline text action after it.
final class ActionRow: NSView {
    private let key: NSTextField
    private let value: NSTextField
    private let button = NSButton()
    var onAction: (() -> Void)?

    init(key keyText: String, value valueText: String, tint: NSColor, actionTitle: String?) {
        key = Build.label(keyText, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        value = Build.label(valueText, font: .systemFont(ofSize: 12), color: tint, alignment: .right)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        key.setContentCompressionResistancePriority(.required, for: .horizontal)
        value.lineBreakMode = .byTruncatingTail

        button.title = actionTitle ?? ""
        button.isBordered = false
        button.font = .systemFont(ofSize: 12, weight: .medium)
        button.contentTintColor = .controlAccentColor
        button.target = self
        button.action = #selector(tapped)
        button.isHidden = actionTitle == nil
        button.translatesAutoresizingMaskIntoConstraints = false
        button.setContentCompressionResistancePriority(.required, for: .horizontal)

        addSubview(key)
        addSubview(value)
        addSubview(button)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 32),
            key.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            key.centerYAnchor.constraint(equalTo: centerYAnchor),
            value.leadingAnchor.constraint(greaterThanOrEqualTo: key.trailingAnchor, constant: 10),
            value.centerYAnchor.constraint(equalTo: centerYAnchor),
            button.leadingAnchor.constraint(equalTo: value.trailingAnchor, constant: actionTitle == nil ? 0 : 8),
            button.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            button.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func tapped() {
        onAction?()
    }
}
