import AppKit

/// Non-activating child window so `@` completion never pulls focus out of the composer.
@MainActor
final class MentionPanel {
    private var panel: NSPanel?
    private var rows: [MentionRowView] = []
    private let stack = Build.stack([], spacing: 0)
    private var bots: [Bot] = []
    private var selectedIndex = 0

    var onPick: ((Bot) -> Void)?

    var isVisible: Bool { panel?.isVisible ?? false }

    private static let rowHeight: CGFloat = 38
    private static let maxRows = 5
    private static let width: CGFloat = 268

    func show(bots newBots: [Bot], near caretRect: NSRect, relativeTo view: NSView) {
        guard let parent = view.window else { return }
        bots = newBots
        selectedIndex = min(selectedIndex, max(0, bots.count - 1))

        let panel = self.panel ?? makePanel()
        self.panel = panel

        syncRows()

        let visibleRows = CGFloat(min(bots.count, Self.maxRows))
        let height = visibleRows * Self.rowHeight + 10
        var origin = NSPoint(x: caretRect.minX - 10, y: caretRect.maxY + 8)

        if let screen = parent.screen, origin.y + height > screen.visibleFrame.maxY {
            origin.y = caretRect.minY - height - 8
        }

        panel.setFrame(NSRect(origin: origin, size: NSSize(width: Self.width, height: height)), display: true)

        if panel.parent == nil {
            parent.addChildWindow(panel, ordered: .above)
        }
        panel.orderFront(nil)
    }

    func dismiss() {
        guard let panel else { return }
        panel.parent?.removeChildWindow(panel)
        panel.orderOut(nil)
    }

    func moveSelection(by delta: Int) {
        guard !bots.isEmpty else { return }
        selectedIndex = (selectedIndex + delta + bots.count) % bots.count
        syncSelection()
    }

    func commitSelection() {
        guard bots.indices.contains(selectedIndex) else { return }
        onPick?(bots[selectedIndex])
    }

    // MARK: - Views

    private func makePanel() -> NSPanel {
        let panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: Self.width, height: Self.rowHeight),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: true
        )
        panel.isFloatingPanel = true
        panel.becomesKeyOnlyIfNeeded = true
        panel.hidesOnDeactivate = true
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.level = .popUpMenu

        let effect = NSVisualEffectView()
        effect.material = .menu
        effect.blendingMode = .behindWindow
        effect.state = .active
        effect.wantsLayer = true
        effect.layer?.cornerRadius = 10
        effect.layer?.masksToBounds = true
        effect.translatesAutoresizingMaskIntoConstraints = false

        stack.orientation = .vertical
        stack.alignment = .leading
        stack.edgeInsets = NSEdgeInsets(top: 5, left: 0, bottom: 5, right: 0)

        effect.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: effect.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: effect.trailingAnchor),
            stack.topAnchor.constraint(equalTo: effect.topAnchor),
        ])

        panel.contentView = effect
        return panel
    }

    private func syncRows() {
        while rows.count < bots.count {
            let row = MentionRowView()
            row.onHover = { [weak self] view in
                guard let index = self?.rows.firstIndex(of: view) else { return }
                self?.selectedIndex = index
                self?.syncSelection()
            }
            row.onClick = { [weak self] view in
                guard let index = self?.rows.firstIndex(of: view), let bot = self?.bots[index] else {
                    return
                }
                self?.onPick?(bot)
            }
            row.heightAnchor.constraint(equalToConstant: Self.rowHeight).isActive = true
            row.widthAnchor.constraint(equalToConstant: Self.width).isActive = true
            rows.append(row)
            stack.addArrangedSubview(row)
        }
        while rows.count > bots.count {
            let row = rows.removeLast()
            stack.removeArrangedSubview(row)
            row.removeFromSuperview()
        }

        for (index, bot) in bots.enumerated() {
            rows[index].configure(bot: bot, runnerName: AppStore.shared.device(bot.runnerID)?.name ?? "")
        }
        syncSelection()
    }

    private func syncSelection() {
        for (index, row) in rows.enumerated() {
            row.isHighlighted = index == selectedIndex
        }
    }
}

final class MentionRowView: NSView {
    private let avatar = AvatarView(diameter: 22)
    private let name = Build.label("", font: .systemFont(ofSize: 13, weight: .medium))
    private let detail = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private var tracking: NSTrackingArea?

    var onHover: ((MentionRowView) -> Void)?
    var onClick: ((MentionRowView) -> Void)?

    var isHighlighted = false {
        didSet {
            needsDisplay = true
            name.textColor = isHighlighted ? .white : .labelColor
            detail.textColor = isHighlighted ? NSColor.white.withAlphaComponent(0.75) : .secondaryLabelColor
        }
    }

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true

        let text = Build.stack([name, detail], spacing: 0)
        addSubview(avatar)
        addSubview(text)

        NSLayoutConstraint.activate([
            avatar.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            avatar.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.leadingAnchor.constraint(equalTo: avatar.trailingAnchor, constant: 9),
            text.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor, constant: -12),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(bot: Bot, runnerName: String) {
        avatar.content = .bot(symbolName: bot.symbolName, accent: bot.accent)
        name.stringValue = bot.name
        detail.stringValue = runnerName.isEmpty ? bot.label : "on \(runnerName)"
    }

    override var allowsVibrancy: Bool { false }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeAlways], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { onHover?(self) }
    override func mouseUp(with event: NSEvent) { onClick?(self) }

    override func draw(_ dirtyRect: NSRect) {
        guard isHighlighted else { return }
        NSColor.controlAccentColor.setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 5, dy: 2), xRadius: 6, yRadius: 6).fill()
    }
}
