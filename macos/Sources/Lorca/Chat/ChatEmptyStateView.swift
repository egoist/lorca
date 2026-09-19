import AppKit

final class ChatEmptyStateView: NSView {
    private let avatars = AvatarStackView(diameter: 52, overlap: 16)
    private let title = Build.label("", font: .systemFont(ofSize: 19, weight: .semibold), alignment: .center)
    private let subtitle = Build.label(
        "", font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor, lines: 2, alignment: .center)
    private let suggestions = Build.stack([], spacing: 8)

    var onPick: ((String) -> Void)?

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        suggestions.alignment = .centerX

        let column = Build.stack([avatars, title, subtitle, suggestions], spacing: 12)
        column.alignment = .centerX
        column.setCustomSpacing(16, after: avatars)
        column.setCustomSpacing(18, after: subtitle)

        addSubview(column)
        NSLayoutConstraint.activate([
            column.centerXAnchor.constraint(equalTo: centerXAnchor),
            column.centerYAnchor.constraint(equalTo: centerYAnchor, constant: -20),
            column.widthAnchor.constraint(equalToConstant: 440),
            subtitle.widthAnchor.constraint(equalToConstant: 400),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(chat: Chat, bots: [Bot]) {
        let store = AppStore.shared
        avatars.configure(with: bots)
        title.stringValue = store.title(for: chat)

        if chat.isDM, let only = bots.first {
            let host = store.device(only.runnerID)
            subtitle.stringValue =
                "\(only.label)\n" + L("Runs on %@ with %@", host?.name ?? L("an unassigned Runner"), only.provider.rawValue)
        } else if bots.count > 1 {
            let names = bots.map(\.name).joined(separator: L(", "))
            subtitle.stringValue = "\(names)\n" + L("Address one with @, or say @everyone to hear from all of them.")
        } else if let only = bots.first {
            subtitle.stringValue = "\(only.label)\n" + L("A group of one for now. Add bots from the inspector.")
        } else {
            subtitle.stringValue = L("No bots in this group yet.")
        }

        rebuildSuggestions(for: bots)
    }

    private func rebuildSuggestions(for bots: [Bot]) {
        for view in suggestions.arrangedSubviews {
            suggestions.removeArrangedSubview(view)
            view.removeFromSuperview()
        }

        for prompt in prompts(for: bots) {
            let chip = SuggestionChip(text: prompt)
            chip.onClick = { [weak self] in self?.onPick?(prompt) }
            suggestions.addArrangedSubview(chip)
        }
    }

    private func prompts(for bots: [Bot]) -> [String] {
        guard let first = bots.first else { return [] }
        if bots.count > 1 {
            return [
                L("@%@ break this down and hand off what you can", first.name),
                L("@everyone what would you check first?"),
            ]
        }
        switch first.id {
        case "bot-patch": return [L("Write the smallest version that works"), L("What would you delete first?")]
        case "bot-scout": return [L("Find the prior art and cite it"), L("What do we not know yet?")]
        case "bot-quill": return [L("Rewrite this without adjectives"), L("One paragraph, no hype")]
        case "bot-ember": return [L("What is the blast radius?"), L("Status on the last deploy")]
        default: return [L("What should I work on next?"), L("Plan this and delegate the parts")]
        }
    }
}

final class SuggestionChip: NSView {
    private let label: NSTextField
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    var onClick: (() -> Void)?

    init(text: String) {
        label = Build.label(text, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            label.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            label.topAnchor.constraint(equalTo: topAnchor, constant: 6),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -6),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var allowsVibrancy: Bool { false }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) {
        isHovered = true
        NSCursor.pointingHand.set()
    }

    override func mouseExited(with event: NSEvent) {
        isHovered = false
        NSCursor.arrow.set()
    }

    override func mouseUp(with event: NSEvent) { onClick?() }

    override func draw(_ dirtyRect: NSRect) {
        let path = NSBezierPath(roundedRect: bounds, xRadius: bounds.height / 2, yRadius: bounds.height / 2)
        (isHovered ? NSColor.controlAccentColor.withAlphaComponent(0.12) : Theme.chipBackground).setFill()
        path.fill()
        (isHovered ? NSColor.controlAccentColor.withAlphaComponent(0.4) : NSColor.separatorColor).setStroke()
        path.lineWidth = 1
        path.stroke()
        label.textColor = isHovered ? .controlAccentColor : .secondaryLabelColor
    }
}
