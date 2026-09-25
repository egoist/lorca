import AppKit

/// Titled card used by the inspector and the settings panes.
final class SectionView: NSView {
    var title: String {
        didSet { applyStyle() }
    }

    /// The inspector's small capitals, or System Settings' group title: the row text's size in
    /// bold, on the rows' text column.
    enum Style {
        case caption
        case heading
    }

    var style = Style.caption {
        didSet { applyStyle() }
    }

    private var headerLeading: NSLayoutConstraint!
    private var cardTop: NSLayoutConstraint!
    private var headerAccessory: NSView?
    private var headerAccessoryConstraints: [NSLayoutConstraint] = []

    // Row hover fills are plain rectangles; the stack fills the card, so clipping it to the
    // same radius keeps them inside the corners.
    private func setCornerRadius(_ radius: CGFloat) {
        card.cornerRadius = radius
        rows.layer?.cornerRadius = radius
    }

    private func applyStyle() {
        switch style {
        case .caption:
            header.stringValue = title.uppercased()
            header.font = .systemFont(ofSize: 10, weight: .semibold)
            header.textColor = .tertiaryLabelColor
            headerLeading.constant = 4
            cardTop.constant = 6
            card.borderColor = Theme.botBubbleBorder
            setCornerRadius(9)
        case .heading:
            header.stringValue = title
            header.font = .systemFont(ofSize: 13, weight: .bold)
            header.textColor = .labelColor
            headerLeading.constant = 12
            cardTop.constant = 9
            // System Settings' cards are a fill alone.
            card.borderColor = nil
            setCornerRadius(12)
        }
        for (index, end) in dividerEnds.enumerated() {
            end.constant = index.isMultiple(of: 2) ? dividerInset : -dividerInset
        }
    }
    private let header: NSTextField
    private let card = BackgroundView()
    private let rows = Build.stack([], spacing: 0)

    init(title: String) {
        header = Build.label(
            title.uppercased(), font: .systemFont(ofSize: 10, weight: .semibold),
            color: .tertiaryLabelColor)
        self.title = title
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

        headerLeading = header.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 4)
        cardTop = card.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 6)
        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: topAnchor),
            headerLeading,
            header.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),

            cardTop,
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

    /// The rows' dividers run edge to edge under a caption, and between the rows' text margins
    /// under a heading, as System Settings' do.
    private var dividerInset: CGFloat { style == .heading ? 12 : 0 }
    private var dividerEnds: [NSLayoutConstraint] = []
    private var shownRows: [NSView] = []

    func setRows(_ views: [NSView]) {
        // The rows it shows already, updated in place, keep their places and dividers.
        guard !views.elementsEqual(shownRows, by: ===) else { return }
        shownRows = views
        dividerEnds = []
        for view in rows.arrangedSubviews {
            rows.removeArrangedSubview(view)
            view.removeFromSuperview()
        }
        for (index, view) in views.enumerated() {
            if index > 0 {
                // The line sits in a full-width strip, so the style can pull its ends in.
                let strip = NSView()
                strip.translatesAutoresizingMaskIntoConstraints = false
                let divider = HairlineView()
                strip.addSubview(divider)
                let leading = divider.leadingAnchor.constraint(equalTo: strip.leadingAnchor, constant: dividerInset)
                let trailing = divider.trailingAnchor.constraint(equalTo: strip.trailingAnchor, constant: -dividerInset)
                dividerEnds += [leading, trailing]
                NSLayoutConstraint.activate([
                    leading, trailing,
                    divider.topAnchor.constraint(equalTo: strip.topAnchor),
                    divider.bottomAnchor.constraint(equalTo: strip.bottomAnchor),
                ])
                rows.addArrangedSubview(strip)
                strip.widthAnchor.constraint(equalTo: rows.widthAnchor).isActive = true
            }
            rows.addArrangedSubview(view)
            view.widthAnchor.constraint(equalTo: rows.widthAnchor).isActive = true
        }
    }

    /// A small action beside the section title, such as Settings' add buttons.
    func setHeaderAccessory(_ view: NSView?) {
        NSLayoutConstraint.deactivate(headerAccessoryConstraints)
        headerAccessoryConstraints = []
        headerAccessory?.removeFromSuperview()
        headerAccessory = view
        guard let view else { return }
        view.translatesAutoresizingMaskIntoConstraints = false
        addSubview(view)
        headerAccessoryConstraints = [
            header.trailingAnchor.constraint(lessThanOrEqualTo: view.leadingAnchor, constant: -8),
            view.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            view.centerYAnchor.constraint(equalTo: header.centerYAnchor),
        ]
        NSLayoutConstraint.activate(headerAccessoryConstraints)
    }

    /// The row showing `label`, or the whole card when `label` is this section's title.
    func target(labelled label: String) -> NSView? {
        if title == label { return rows }
        return rows.arrangedSubviews.first { $0.showsLabel(label) }
    }
}

extension NSView {
    fileprivate func showsLabel(_ label: String) -> Bool {
        if let field = self as? NSTextField, !field.isEditable, field.stringValue == label { return true }
        return subviews.contains { $0.showsLabel(label) }
    }
}

/// A brief accent wash over a view, marking where a search result landed.
final class FlashView: NSView {
    static func flash(_ target: NSView) {
        target.subviews.filter { $0 is FlashView }.forEach { $0.removeFromSuperview() }
        let flash = FlashView(frame: target.bounds)
        flash.autoresizingMask = [.width, .height]
        flash.wantsLayer = true
        flash.layer?.backgroundColor = NSColor.controlAccentColor.withAlphaComponent(0.22).cgColor
        target.addSubview(flash)

        let fade = CABasicAnimation(keyPath: "opacity")
        fade.fromValue = 1
        fade.toValue = 0
        fade.beginTime = CACurrentMediaTime() + 0.5
        fade.duration = 0.7
        fade.fillMode = .both
        fade.isRemovedOnCompletion = false
        CATransaction.begin()
        CATransaction.setCompletionBlock { [weak flash] in flash?.removeFromSuperview() }
        flash.layer?.add(fade, forKey: "fade")
        CATransaction.commit()
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
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
            lines: 0,
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
            key.firstBaselineAnchor.constraint(equalTo: value.firstBaselineAnchor),
            value.leadingAnchor.constraint(greaterThanOrEqualTo: key.trailingAnchor, constant: 10),
            value.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            value.topAnchor.constraint(greaterThanOrEqualTo: topAnchor, constant: 8),
            value.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -8),
            value.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    func setValue(_ text: String) {
        guard value.stringValue != text else { return }
        value.stringValue = text
    }

    // A wrapping label only reports a multi-line intrinsic height once it
    // knows its width, so feed it the width left over after the key.
    override func layout() {
        super.layout()
        let available = bounds.width - 12 - key.frame.width - 10 - 12
        if available > 0, value.preferredMaxLayoutWidth != available {
            value.preferredMaxLayoutWidth = available
            invalidateIntrinsicContentSize()
            needsLayout = true
        }
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
    /// A click on the avatar itself, for changing the bot's look. Set, the avatar shows a
    /// pointing hand and takes the click instead of the row.
    var onAvatarClick: (() -> Void)? {
        didSet { avatar.onClick = onAvatarClick }
    }

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
        avatar.content = AvatarView.content(for: bot)
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

/// Row with a leading symbol, a title/subtitle pair and a trailing state pill, or the state as
/// a symbol alone. A state with details behind it (what a plugin needs) shows them in a popover
/// when clicked.
final class StatusRow: NSView, NSGestureRecognizerDelegate {
    private let icon = NSImageView()
    private let title = Build.label("", font: .systemFont(ofSize: 12.5, weight: .medium))
    private let subtitle = Build.label(
        "", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
    private let state = Build.label("", font: .systemFont(ofSize: 11, weight: .medium), alignment: .right)
    private let stateIcon = NSImageView()
    private let stateClick = NSClickGestureRecognizer()
    private let stateIconClick = NSClickGestureRecognizer()
    private var stateDetail: String?
    private let action = NSButton()
    private var textTrailingPlain: NSLayoutConstraint!
    private var textTrailingState: NSLayoutConstraint!
    private var textTrailingStateIcon: NSLayoutConstraint!
    private var textTrailingAction: NSLayoutConstraint!

    var onAction: (() -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.contentTintColor = .secondaryLabelColor
        stateIcon.translatesAutoresizingMaskIntoConstraints = false
        stateIcon.isHidden = true

        for (click, view) in [(stateClick, state as NSView), (stateIconClick, stateIcon)] {
            click.target = self
            click.action = #selector(showStateDetail(_:))
            click.delegate = self
            click.isEnabled = false
            view.addGestureRecognizer(click)
        }

        action.bezelStyle = .rounded
        action.controlSize = .small
        action.target = self
        action.action = #selector(actionTapped)
        action.translatesAutoresizingMaskIntoConstraints = false
        action.isHidden = true
        action.setContentHuggingPriority(.required, for: .horizontal)
        action.setContentCompressionResistancePriority(.required, for: .horizontal)
        state.setContentHuggingPriority(.required, for: .horizontal)
        state.setContentCompressionResistancePriority(.required, for: .horizontal)
        stateIcon.setContentHuggingPriority(.required, for: .horizontal)
        stateIcon.setContentCompressionResistancePriority(.required, for: .horizontal)

        let text = Build.stack([title, subtitle], spacing: 1)
        addSubview(icon)
        addSubview(text)
        addSubview(state)
        addSubview(stateIcon)
        addSubview(action)

        textTrailingPlain = text.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12)
        textTrailingState = text.trailingAnchor.constraint(equalTo: state.leadingAnchor, constant: -8)
        textTrailingStateIcon = text.trailingAnchor.constraint(equalTo: stateIcon.leadingAnchor, constant: -8)
        textTrailingAction = text.trailingAnchor.constraint(equalTo: action.leadingAnchor, constant: -8)
        textTrailingPlain.isActive = true

        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 18),
            text.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 10),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.topAnchor.constraint(greaterThanOrEqualTo: topAnchor, constant: 8),
            text.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -8),
            state.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            state.centerYAnchor.constraint(equalTo: centerYAnchor),
            stateIcon.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            stateIcon.centerYAnchor.constraint(equalTo: centerYAnchor),
            action.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            action.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(
        symbol: String,
        image: NSImage? = nil,
        title titleText: String,
        subtitle subtitleText: String,
        state stateText: String?,
        stateSymbol: String? = nil,
        stateColor: NSColor = .secondaryLabelColor,
        stateDetail: String? = nil,
        actionTitle: String? = nil,
        destructive: Bool = false
    ) {
        icon.image = image ?? NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 13, weight: .regular)
        title.stringValue = titleText
        subtitle.stringValue = subtitleText
        // With a symbol, the state's words are its tooltip and what VoiceOver reads.
        let showsSymbol = stateText != nil && stateSymbol != nil
        state.stringValue = stateText ?? ""
        state.textColor = stateColor
        state.isHidden = stateText == nil || showsSymbol
        stateIcon.image = stateSymbol.flatMap { NSImage(systemSymbolName: $0, accessibilityDescription: stateText) }
        stateIcon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 12, weight: .semibold)
        stateIcon.contentTintColor = stateColor
        stateIcon.toolTip = stateText
        stateIcon.setAccessibilityLabel(stateText)
        stateIcon.isHidden = !showsSymbol
        self.stateDetail = stateDetail
        stateClick.isEnabled = stateDetail != nil
        stateIconClick.isEnabled = stateDetail != nil

        NSLayoutConstraint.deactivate([textTrailingPlain, textTrailingState, textTrailingStateIcon, textTrailingAction])
        if let actionTitle {
            // The same bezel for both; a destructive action has a red title. Neither
            // `contentTintColor` nor `hasDestructiveAction` colors a rounded bezel's title, so
            // the title is attributed.
            let font = NSFont.systemFont(ofSize: NSFont.systemFontSize(for: .small))
            let color: NSColor = destructive ? .systemRed : .controlTextColor
            action.attributedTitle = NSAttributedString(string: actionTitle, attributes: [.foregroundColor: color, .font: font])
            action.isHidden = false
            state.isHidden = true
            stateIcon.isHidden = true
            textTrailingAction.isActive = true
        } else {
            action.isHidden = true
            (stateText == nil ? textTrailingPlain : showsSymbol ? textTrailingStateIcon : textTrailingState).isActive = true
        }
    }

    /// A plugin and its state as a symbol: a check when it is ready, an exclamation mark when
    /// it needs something. What the Runner says it needs (a variable, a sign-in, an error's
    /// message) is the symbol's tooltip and a click away, so a long one never widens the row.
    func configure(plugin: InstalledPlugin) {
        let ready = plugin.state == .ready
        configure(
            symbol: plugin.symbolName,
            image: PluginLogo.tile(for: plugin.id, size: 18),
            title: plugin.name,
            subtitle: plugin.description,
            state: ready ? L("Ready") : plugin.detail,
            stateSymbol: ready ? "checkmark" : "exclamationmark.circle.fill",
            stateColor: ready ? .systemGreen : .systemOrange,
            stateDetail: ready ? nil : plugin.detail
        )
    }

    @objc private func actionTapped() {
        onAction?()
    }

    @objc private func showStateDetail(_ sender: NSClickGestureRecognizer) {
        guard let stateDetail, let view = sender.view else { return }
        TextPopover.show(stateDetail, relativeTo: view.bounds, of: view)
    }

    /// A click on the state shows its details instead of doing what a click on the row does.
    func gestureRecognizer(
        _ gestureRecognizer: NSGestureRecognizer, shouldBeRequiredToFailBy otherGestureRecognizer: NSGestureRecognizer
    ) -> Bool {
        otherGestureRecognizer.view === self
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
    private let button = CopyFeedbackButton()
    var onAction: (() -> Void)?

    /// A monospaced value is something to copy (a sign-in code), so it is also selectable.
    init(key keyText: String, value valueText: String, tint: NSColor, actionTitle: String?, monospaced: Bool = false) {
        key = Build.label(keyText, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        value = Build.label(
            valueText,
            font: monospaced ? .monospacedSystemFont(ofSize: 12, weight: .semibold) : .systemFont(ofSize: 12),
            color: tint, alignment: .right)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        key.setContentCompressionResistancePriority(.required, for: .horizontal)
        value.lineBreakMode = .byTruncatingTail
        value.isSelectable = monospaced

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

    func setValue(_ text: String) {
        guard value.stringValue != text else { return }
        value.stringValue = text
    }

    /// The action copied something: its title reads Copied for a moment.
    func showCopied() {
        button.showCopied()
    }

    @objc private func tapped() {
        onAction?()
    }
}


/// A key and action on the first line, with a wrapping two-line preview below them. With no
/// preview, the key and action sit alone on one line, centered like the rows around them.
final class SummaryActionRow: NSView {
    private let value: NSTextField
    private let button = NSButton()
    private var withPreview: [NSLayoutConstraint] = []
    private var withoutPreview: [NSLayoutConstraint] = []
    var onAction: (() -> Void)?

    init(key keyText: String, value valueText: String, actionTitle: String) {
        let key = Build.label(keyText, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        value = Build.label(valueText, font: Theme.Font.caption, color: .secondaryLabelColor, lines: 2)
        value.lineBreakMode = .byTruncatingTail
        value.cell?.wraps = true
        value.cell?.truncatesLastVisibleLine = true
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        button.title = actionTitle
        button.isBordered = false
        button.font = .systemFont(ofSize: 12, weight: .medium)
        button.contentTintColor = .controlAccentColor
        button.target = self
        button.action = #selector(tapped)
        button.translatesAutoresizingMaskIntoConstraints = false
        button.setContentCompressionResistancePriority(.required, for: .horizontal)

        let spacer = NSView()
        spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
        let header = Build.stack([key, spacer, button], orientation: .horizontal, spacing: 8)
        addSubview(header)
        addSubview(value)
        withPreview = [
            header.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            value.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
        ]
        withoutPreview = [
            heightAnchor.constraint(equalToConstant: 32),
            header.centerYAnchor.constraint(equalTo: centerYAnchor),
        ]
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 32),
            header.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            header.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            value.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 2),
            value.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            value.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
        ])
        setValue(valueText)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// An empty value hides the preview line rather than leaving it blank.
    func setValue(_ text: String) {
        value.stringValue = text
        value.isHidden = text.isEmpty
        NSLayoutConstraint.deactivate(text.isEmpty ? withPreview : withoutPreview)
        NSLayoutConstraint.activate(text.isEmpty ? withoutPreview : withPreview)
    }

    override func layout() {
        super.layout()
        let width = bounds.width - 24
        if width > 0, value.preferredMaxLayoutWidth != width {
            value.preferredMaxLayoutWidth = width
            invalidateIntrinsicContentSize()
            needsLayout = true
        }
    }

    @objc private func tapped() {
        onAction?()
    }
}


/// Key on the left, an editable value on the right. Looks like a value until it is clicked;
/// commits when editing ends (Return, Tab, or focus leaving the field).
final class EditableRow: NSView, NSTextFieldDelegate {
    private let key: NSTextField
    let field: NSTextField
    var onCommit: (() -> Void)?

    var value: String {
        field.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    init(key keyText: String, placeholder: String, multiline: Bool = false) {
        key = Build.label(keyText, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        field = multiline ? WrappingTextField() : NSTextField()
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        field.translatesAutoresizingMaskIntoConstraints = false
        field.isBordered = false
        field.drawsBackground = false
        field.focusRingType = .none
        field.font = .systemFont(ofSize: 12)
        field.textColor = .labelColor
        field.placeholderString = placeholder
        field.delegate = self
        field.usesSingleLineMode = !multiline
        field.maximumNumberOfLines = multiline ? 0 : 1
        field.lineBreakMode = multiline ? .byWordWrapping : .byTruncatingTail
        field.cell?.wraps = multiline
        field.cell?.isScrollable = !multiline
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        key.setContentCompressionResistancePriority(.required, for: .horizontal)

        addSubview(key)
        addSubview(field)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 32),
            key.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            key.widthAnchor.constraint(equalToConstant: 76),
            key.firstBaselineAnchor.constraint(equalTo: field.firstBaselineAnchor),
            field.leadingAnchor.constraint(equalTo: key.trailingAnchor, constant: 10),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            field.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            field.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Shows a value from the model unless the user is typing in the field right now.
    func setValue(_ text: String) {
        guard field.currentEditor() == nil, field.stringValue != text else { return }
        field.stringValue = text
    }

    func controlTextDidEndEditing(_ obj: Notification) {
        onCommit?()
    }
}

/// Editable text field whose height follows its wrapped text at the width Auto Layout gave it,
/// including while the user types.
final class WrappingTextField: NSTextField {
    override var intrinsicContentSize: NSSize {
        guard let cell, bounds.width > 0 else { return super.intrinsicContentSize }
        let bounds = NSRect(x: 0, y: 0, width: bounds.width, height: .greatestFiniteMagnitude)
        return NSSize(width: NSView.noIntrinsicMetric, height: ceil(cell.cellSize(forBounds: bounds).height))
    }

    override func setFrameSize(_ newSize: NSSize) {
        let widthChanged = newSize.width != frame.width
        super.setFrameSize(newSize)
        if widthChanged { invalidateIntrinsicContentSize() }
    }

    override func textDidChange(_ notification: Notification) {
        super.textDidChange(notification)
        invalidateIntrinsicContentSize()
    }
}


/// A row with an icon for its state, a title over a detail line, and a switch: a routine that
/// pauses or resumes, a plugin a bot may use. Clicking the row opens its details.
final class SwitchRow: NSView {
    private let icon = NSImageView()
    private let name = Build.label("", font: .systemFont(ofSize: 12.5, weight: .medium))
    private let detail = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private let toggle = NSSwitch()
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    var onToggle: ((Bool) -> Void)?
    var onClick: (() -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        icon.translatesAutoresizingMaskIntoConstraints = false
        toggle.controlSize = .mini
        toggle.target = self
        toggle.action = #selector(toggled)
        toggle.translatesAutoresizingMaskIntoConstraints = false
        toggle.setContentCompressionResistancePriority(.required, for: .horizontal)

        let text = Build.stack([name, detail], spacing: 1)
        addSubview(icon)
        addSubview(text)
        addSubview(toggle)
        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 44),
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: 18),
            text.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 10),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.trailingAnchor.constraint(lessThanOrEqualTo: toggle.leadingAnchor, constant: -8),
            toggle.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -10),
            toggle.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
        // A click anywhere on the row but the switch opens the routine, labels included.
        addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(clicked(_:))))
    }

    @objc private func clicked(_ gesture: NSClickGestureRecognizer) {
        guard !toggle.frame.contains(gesture.location(in: self)) else { return }
        onClick?()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(routine: Routine) {
        let symbol = routine.isRunning ? "arrow.triangle.2.circlepath" : (routine.isEnabled ? "clock" : "pause.circle")
        configure(
            symbol: symbol,
            tint: routine.isRunning ? .controlAccentColor : (routine.isEnabled ? .secondaryLabelColor : .tertiaryLabelColor),
            title: routine.name, detail: routine.detail, isOn: routine.isEnabled,
            toggleTooltip: routine.isEnabled ? L("Pause %@", routine.name) : L("Resume %@", routine.name), tooltip: routine.prompt)
    }

    func configure(symbol: String, tint: NSColor, title: String, detail detailText: String, isOn: Bool, toggleTooltip: String, tooltip: String) {
        icon.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 13, weight: .regular)
        icon.contentTintColor = tint
        name.stringValue = title
        detail.stringValue = detailText
        toggle.state = isOn ? .on : .off
        toggle.toolTip = toggleTooltip
        toolTip = tooltip
    }

    @objc private func toggled() {
        onToggle?(toggle.state == .on)
    }

    override var allowsVibrancy: Bool { false }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { isHovered = onClick != nil }
    override func mouseExited(with event: NSEvent) { isHovered = false }

    override func draw(_ dirtyRect: NSRect) {
        guard isHovered else { return }
        NSColor.labelColor.withAlphaComponent(0.05).setFill()
        bounds.fill()
    }
}

/// A sentence inside a section card, for an empty state.
final class NoteRow: NSView {
    init(text: String) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let label = Build.label(text, font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
        addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            label.topAnchor.constraint(equalTo: topAnchor, constant: 10),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -10),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}
