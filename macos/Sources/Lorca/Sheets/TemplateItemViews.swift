import AppKit

/// A piece of a bot template in the export and import sheets, after SelectableBotRow: a check when
/// it can be picked, the bot's avatar or a plugin's logo, its text, and what to look at before
/// sharing it (an email address, a removed key) at the end.
final class TemplateItemRow: NSView {
    private let check = NSImageView()
    private let title: NSTextField
    private let detail = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 2)
    private let flag = Build.label("", font: .systemFont(ofSize: 10, weight: .medium), alignment: .right)
    private let isSelectable: Bool
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    var onToggle: (() -> Void)?

    var isSelected = false {
        didSet { showSelection() }
    }

    private func showSelection() {
        check.image = NSImage(systemSymbolName: isSelected ? "checkmark.circle.fill" : "circle", accessibilityDescription: nil)
        check.contentTintColor = isSelected ? .controlAccentColor : .tertiaryLabelColor
        setAccessibilityValue(isSelected)
    }

    /// A memory's text can run to three lines; a name is one.
    init(item: TemplateItem, isSelectable: Bool, titleLines: Int = 1, media: NSView? = nil) {
        self.isSelectable = isSelectable
        title = Build.label(item.title, font: .systemFont(ofSize: 13, weight: titleLines == 1 ? .medium : .regular), lines: titleLines)
        if titleLines > 1 { title.lineBreakMode = .byTruncatingTail }
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true

        detail.stringValue = item.detail
        detail.lineBreakMode = .byTruncatingTail
        detail.isHidden = item.detail.isEmpty
        let summary = item.flagSummary
        flag.stringValue = summary?.text ?? ""
        flag.textColor = summary?.personal == true ? .systemOrange : .secondaryLabelColor
        flag.isHidden = summary == nil
        flag.setContentCompressionResistancePriority(.required, for: .horizontal)
        flag.setContentHuggingPriority(.required, for: .horizontal)
        toolTip = [item.title, item.detail].filter { !$0.isEmpty }.joined(separator: "\n")

        let text = Build.stack([title, detail], spacing: 2)
        addSubview(text)
        addSubview(flag)
        var constraints = [
            heightAnchor.constraint(greaterThanOrEqualToConstant: 36),
            text.topAnchor.constraint(equalTo: topAnchor, constant: 9),
            text.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -9),
            // The text runs to the flag, or to the edge without one.
            summary == nil
                ? text.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12)
                : text.trailingAnchor.constraint(equalTo: flag.leadingAnchor, constant: -8),
            title.widthAnchor.constraint(equalTo: text.widthAnchor),
            detail.widthAnchor.constraint(equalTo: text.widthAnchor),
            flag.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            flag.firstBaselineAnchor.constraint(equalTo: title.firstBaselineAnchor),
        ]

        var leading = leadingAnchor
        var leadingInset: CGFloat = 12
        if isSelectable {
            check.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .regular)
            check.translatesAutoresizingMaskIntoConstraints = false
            addSubview(check)
            constraints += [
                check.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
                check.widthAnchor.constraint(equalToConstant: 16),
                media.map { check.centerYAnchor.constraint(equalTo: $0.centerYAnchor) } ?? check.centerYAnchor.constraint(equalTo: title.topAnchor, constant: 8),
            ]
            leading = check.trailingAnchor
            leadingInset = 10
        }
        if let media {
            media.translatesAutoresizingMaskIntoConstraints = false
            addSubview(media)
            constraints += [
                media.leadingAnchor.constraint(equalTo: leading, constant: leadingInset),
                media.topAnchor.constraint(equalTo: topAnchor, constant: 9),
                media.bottomAnchor.constraint(lessThanOrEqualTo: bottomAnchor, constant: -9),
            ]
            leading = media.trailingAnchor
            leadingInset = 9
        }
        constraints.append(text.leadingAnchor.constraint(equalTo: leading, constant: leadingInset))
        NSLayoutConstraint.activate(constraints)

        setAccessibilityElement(true)
        setAccessibilityRole(isSelectable ? .checkBox : .staticText)
        setAccessibilityLabel(toolTip)
        showSelection()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    // Wrapping labels report their height once they know their width, which the row's edges set.
    override func layout() {
        super.layout()
        let width = title.frame.width
        guard width > 0, title.preferredMaxLayoutWidth != width else { return }
        title.preferredMaxLayoutWidth = width
        detail.preferredMaxLayoutWidth = width
        needsLayout = true
    }

    override var allowsVibrancy: Bool { false }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        guard isSelectable else { return }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { isHovered = true }
    override func mouseExited(with event: NSEvent) { isHovered = false }
    override func mouseDown(with event: NSEvent) {}

    override func mouseUp(with event: NSEvent) {
        guard isSelectable, bounds.contains(convert(event.locationInWindow, from: nil)) else { return }
        onToggle?()
    }

    override func accessibilityPerformPress() -> Bool {
        guard isSelectable else { return false }
        onToggle?()
        return true
    }

    override func draw(_ dirtyRect: NSRect) {
        guard isHovered, isSelectable else { return }
        NSColor.labelColor.withAlphaComponent(0.05).setFill()
        bounds.fill()
    }
}

extension TemplateItemRow {
    /// The profile's row: the bot's look beside its name and description.
    static func profile(_ item: TemplateItem, isSelectable: Bool) -> TemplateItemRow {
        let avatar = AvatarView(diameter: 28)
        if let look = item.look { avatar.content = .bot(symbolName: look.symbolName, accent: look.accent) }
        NSLayoutConstraint.activate([avatar.widthAnchor.constraint(equalToConstant: 28), avatar.heightAnchor.constraint(equalToConstant: 28)])
        return TemplateItemRow(item: item, isSelectable: isSelectable, media: avatar)
    }

    /// A plugin's row: its logo and name.
    static func plugin(id: String, name: String, isSelectable: Bool) -> TemplateItemRow {
        let logo = NSImageView()
        logo.image = PluginLogo.tile(for: id, size: 18) ?? NSImage(systemSymbolName: "puzzlepiece.extension", accessibilityDescription: nil)
        logo.contentTintColor = .secondaryLabelColor
        NSLayoutConstraint.activate([logo.widthAnchor.constraint(equalToConstant: 18), logo.heightAnchor.constraint(equalToConstant: 18)])
        return TemplateItemRow(item: TemplateItem(id: id, title: name, detail: "", flags: []), isSelectable: isSelectable, media: logo)
    }
}

/// The cards of a template's contents, scrolling past `maxHeight`.
final class TemplateItemList: NSScrollView {
    private let column = Build.stack([], spacing: 16)
    private let document = FlippedView()
    private var height: NSLayoutConstraint!
    private let maxHeight: CGFloat

    init(maxHeight: CGFloat) {
        self.maxHeight = maxHeight
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        drawsBackground = false
        hasVerticalScroller = true
        autohidesScrollers = true
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(column)
        documentView = document
        height = heightAnchor.constraint(equalToConstant: 0)
        NSLayoutConstraint.activate([
            height,
            document.widthAnchor.constraint(equalTo: contentView.widthAnchor),
            column.topAnchor.constraint(equalTo: document.topAnchor),
            column.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            column.bottomAnchor.constraint(equalTo: document.bottomAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// A card per kind that has rows; a kind without any shows nothing. `accessories` go beside
    /// the titles they are keyed by.
    func setSections(_ sections: [(title: String, rows: [NSView])], accessories: [String: NSView] = [:]) {
        column.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for (title, rows) in sections where !rows.isEmpty {
            let section = SectionView(title: title)
            section.setRows(rows)
            section.setHeaderAccessory(accessories[title])
            column.addArrangedSubview(section)
            section.widthAnchor.constraint(equalTo: column.widthAnchor).isActive = true
        }
        fitHeight()
    }

    /// As tall as the cards, up to `maxHeight`.
    func fitHeight() {
        guard bounds.width > 0 else { return }
        document.layoutSubtreeIfNeeded()
        let wanted = min(maxHeight, ceil(column.fittingSize.height))
        if height.constant != wanted { height.constant = wanted }
    }

    override func layout() {
        super.layout()
        let wanted = min(maxHeight, ceil(column.frame.height))
        if column.frame.height > 0, height.constant != wanted { height.constant = wanted }
    }
}
