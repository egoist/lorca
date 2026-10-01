import AppKit

/// Non-activating child window so `@` completion never pulls focus out of the composer. It is
/// tall enough for five bots and scrolls to the rest.
@MainActor
final class MentionPanel {
    private var panel: NSPanel?
    private var rows: [MentionRowView] = []
    private let stack = Build.stack([], spacing: 0)
    private let scrollView = NSScrollView()
    private let document = FlippedView()
    private var bots: [Bot] = []
    private var selectedIndex = 0
    /// Where the pointer was when the list last moved under it: shown, filtered, or scrolled by
    /// the arrow keys. A row arriving beneath a resting pointer leaves the highlight alone; only
    /// a real move highlights.
    private var restingPointer = NSEvent.mouseLocation

    var onPick: ((Bot) -> Void)?

    var isVisible: Bool { panel?.isVisible ?? false }

    private static let rowHeight: CGFloat = 38
    private static let maxRows = 5
    private static let width: CGFloat = 268
    /// Above the first row and below the last.
    private static let inset: CGFloat = 5

    func show(bots newBots: [Bot], near caretRect: NSRect, relativeTo view: NSView) {
        guard let parent = view.window else { return }
        bots = newBots
        selectedIndex = min(selectedIndex, max(0, bots.count - 1))
        restingPointer = NSEvent.mouseLocation

        let panel = self.panel ?? makePanel()
        self.panel = panel

        syncRows()
        document.frame = NSRect(
            x: 0, y: 0, width: Self.width, height: CGFloat(bots.count) * Self.rowHeight + Self.inset * 2)

        let visibleRows = CGFloat(min(bots.count, Self.maxRows))
        let height = visibleRows * Self.rowHeight + Self.inset * 2
        var origin = NSPoint(x: caretRect.minX - 10, y: caretRect.maxY + 8)

        if let screen = parent.screen, origin.y + height > screen.visibleFrame.maxY {
            origin.y = caretRect.minY - height - 8
        }

        panel.setFrame(NSRect(origin: origin, size: NSSize(width: Self.width, height: height)), display: true)
        panel.contentView?.layoutSubtreeIfNeeded()
        scrollSelectionToVisible()

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
        restingPointer = NSEvent.mouseLocation
        syncSelection()
        scrollSelectionToVisible()
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
        stack.edgeInsets = NSEdgeInsets(top: Self.inset, left: 0, bottom: Self.inset, right: 0)

        // The rows stack in a document as tall as all of them, which `show` sizes.
        document.addSubview(stack)
        NSLayoutConstraint.activate([
            stack.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            stack.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            stack.topAnchor.constraint(equalTo: document.topAnchor),
        ])
        scrollView.documentView = document
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = true
        scrollView.autohidesScrollers = true
        scrollView.scrollerStyle = .overlay
        scrollView.verticalScrollElasticity = .none
        effect.addSubview(scrollView)
        scrollView.pin(to: effect)

        panel.contentView = effect
        return panel
    }

    /// Whether the pointer moved since the list last moved under it.
    private func pointerMoved() -> Bool {
        let pointer = NSEvent.mouseLocation
        guard abs(pointer.x - restingPointer.x) > 1 || abs(pointer.y - restingPointer.y) > 1 else { return false }
        restingPointer = NSPoint(x: CGFloat.infinity, y: CGFloat.infinity)
        return true
    }

    /// Brings the highlighted row into view, and the list's inset with the first or last row.
    private func scrollSelectionToVisible() {
        guard rows.indices.contains(selectedIndex) else { return }
        let row = rows[selectedIndex]
        document.scrollToVisible(document.convert(row.bounds, from: row).insetBy(dx: 0, dy: -Self.inset))
    }

    private func syncRows() {
        while rows.count < bots.count {
            let row = MentionRowView()
            row.onHover = { [weak self] view in
                guard let self, let index = rows.firstIndex(of: view), pointerMoved() else { return }
                selectedIndex = index
                syncSelection()
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
        avatar.content = AvatarView.content(for: bot)
        name.stringValue = bot.name
        detail.stringValue = runnerName.isEmpty ? bot.provider.name : L("on %@", runnerName)
    }

    override var allowsVibrancy: Bool { false }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .mouseMoved, .activeAlways], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    // A move inside the row counts too: the panel ignores a row that scrolled in under a
    // resting pointer until the pointer moves.
    override func mouseEntered(with event: NSEvent) { onHover?(self) }
    override func mouseMoved(with event: NSEvent) { onHover?(self) }
    override func mouseUp(with event: NSEvent) { onClick?(self) }

    override func draw(_ dirtyRect: NSRect) {
        guard isHighlighted else { return }
        NSColor.controlAccentColor.setFill()
        NSBezierPath(roundedRect: bounds.insetBy(dx: 5, dy: 2), xRadius: 6, yRadius: 6).fill()
    }
}
