import AppKit

extension Computer {
    var symbolName: String {
        if model.contains("MacBook") { return "laptopcomputer" }
        if model.contains("Studio") { return "macstudio" }
        if model.contains("mini") { return "macmini" }
        return "desktopcomputer"
    }
}

final class SidebarNode: NSObject {
    enum Kind: Hashable {
        case header(String)
        case chat(Chat.ID)
        case computer(Computer.ID)
    }

    let kind: Kind
    var children: [SidebarNode] = []

    init(_ kind: Kind) {
        self.kind = kind
    }

    var selection: Selection? {
        switch kind {
        case .header: nil
        case let .chat(id): .chat(id)
        case let .computer(id): .computer(id)
        }
    }

    var isHeader: Bool {
        if case .header = kind { return true }
        return false
    }
}

/// Shared column geometry. Chat and computer rows use the same leading inset and
/// icon slot so their titles line up down the whole sidebar.
enum SidebarMetric {
    static let inset: CGFloat = 8
    static let slot: CGFloat = 38
    static let gap: CGFloat = 10
    static let trailingInset: CGFloat = 8
    static var textLeading: CGFloat { inset + slot + gap }
}

// MARK: - Header

final class SidebarHeaderCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("SidebarHeaderCell")

    private let label = Build.label(
        "", font: .systemFont(ofSize: 11, weight: .bold), color: .secondaryLabelColor)

    init() {
        super.init(frame: .zero)
        addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: SidebarMetric.inset + 2),
            label.trailingAnchor.constraint(lessThanOrEqualTo: trailingAnchor),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -5),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(_ title: String) {
        label.stringValue = title
    }
}

// MARK: - Chat row

final class SidebarChatCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("SidebarChatCell")

    private let avatars = AvatarClusterView(slot: SidebarMetric.slot)
    private let title = Build.label("", font: Theme.Font.sidebarTitle)
    private let preview = Build.label(
        "", font: Theme.Font.sidebarPreview, color: .secondaryLabelColor)
    private let stamp = Build.label(
        "", font: Theme.Font.caption, color: .tertiaryLabelColor, alignment: .right)
    private let pin = NSImageView()
    private let badge = BackgroundView()
    private let badgeLabel = Build.label(
        "", font: .systemFont(ofSize: 10, weight: .semibold), color: .white, alignment: .center)

    // Hidden views still occupy their intrinsic width in Auto Layout, so the title
    // and preview claim the reclaimed space by switching which view they stop at.
    private lazy var titleBeforePin = title.trailingAnchor.constraint(
        lessThanOrEqualTo: pin.leadingAnchor, constant: -5)
    private lazy var titleBeforeStamp = title.trailingAnchor.constraint(
        lessThanOrEqualTo: stamp.leadingAnchor, constant: -6)
    private lazy var previewBeforeBadge = preview.trailingAnchor.constraint(
        lessThanOrEqualTo: badge.leadingAnchor, constant: -6)
    private lazy var previewBeforeEdge = preview.trailingAnchor.constraint(
        lessThanOrEqualTo: trailingAnchor, constant: -SidebarMetric.trailingInset)

    init() {
        super.init(frame: .zero)

        pin.image = NSImage(systemSymbolName: "pin.fill", accessibilityDescription: "Pinned")
        pin.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 9, weight: .medium)
        pin.contentTintColor = .tertiaryLabelColor
        pin.translatesAutoresizingMaskIntoConstraints = false
        pin.isHidden = true

        badge.cornerRadius = 8
        badge.addSubview(badgeLabel)
        badge.isHidden = true

        for tight in [stamp, pin, badge] as [NSView] {
            tight.setContentCompressionResistancePriority(.required, for: .horizontal)
            tight.setContentHuggingPriority(.required, for: .horizontal)
        }

        for subview in [avatars, title, preview, stamp, pin, badge] as [NSView] {
            addSubview(subview)
        }

        NSLayoutConstraint.activate([
            avatars.leadingAnchor.constraint(equalTo: leadingAnchor, constant: SidebarMetric.inset),
            avatars.centerYAnchor.constraint(equalTo: centerYAnchor),

            title.leadingAnchor.constraint(equalTo: leadingAnchor, constant: SidebarMetric.textLeading),
            title.topAnchor.constraint(equalTo: topAnchor, constant: 11),

            pin.trailingAnchor.constraint(equalTo: stamp.leadingAnchor, constant: -4),
            pin.firstBaselineAnchor.constraint(equalTo: title.firstBaselineAnchor),

            stamp.trailingAnchor.constraint(
                equalTo: trailingAnchor, constant: -SidebarMetric.trailingInset),
            stamp.firstBaselineAnchor.constraint(equalTo: title.firstBaselineAnchor),

            preview.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            preview.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 2),

            badge.trailingAnchor.constraint(
                equalTo: trailingAnchor, constant: -SidebarMetric.trailingInset),
            badge.centerYAnchor.constraint(equalTo: preview.centerYAnchor),
            badge.heightAnchor.constraint(equalToConstant: 16),
            badge.widthAnchor.constraint(greaterThanOrEqualToConstant: 18),

            badgeLabel.centerXAnchor.constraint(equalTo: badge.centerXAnchor),
            badgeLabel.centerYAnchor.constraint(equalTo: badge.centerYAnchor),
            badgeLabel.leadingAnchor.constraint(equalTo: badge.leadingAnchor, constant: 5),
            badgeLabel.trailingAnchor.constraint(equalTo: badge.trailingAnchor, constant: -5),
        ])

        textField = title
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(chat: Chat, store: AppStore) {
        avatars.configure(with: store.bots(in: chat))
        title.stringValue = store.title(for: chat)
        preview.stringValue = store.preview(for: chat)
        stamp.stringValue = Format.stamp(chat.lastActivity)

        pin.isHidden = !chat.isPinned
        titleBeforePin.isActive = chat.isPinned
        titleBeforeStamp.isActive = !chat.isPinned

        let unread = chat.unreadCount
        badge.isHidden = unread == 0
        badge.fillColor = .controlAccentColor
        badgeLabel.stringValue = unread > 99 ? "99+" : "\(unread)"
        previewBeforeBadge.isActive = unread > 0
        previewBeforeEdge.isActive = unread == 0

        title.font = unread > 0 ? .systemFont(ofSize: 13, weight: .semibold) : Theme.Font.sidebarTitle
        applyBackgroundStyle()
    }

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { applyBackgroundStyle() }
    }

    private func applyBackgroundStyle() {
        let emphasized = backgroundStyle == .emphasized
        preview.textColor = emphasized ? NSColor.white.withAlphaComponent(0.75) : .secondaryLabelColor
        stamp.textColor = emphasized ? NSColor.white.withAlphaComponent(0.65) : .tertiaryLabelColor
        pin.contentTintColor = emphasized ? NSColor.white.withAlphaComponent(0.7) : .tertiaryLabelColor
        badge.fillColor = emphasized ? NSColor.white.withAlphaComponent(0.9) : .controlAccentColor
        badgeLabel.textColor = emphasized ? .controlAccentColor : .white
    }
}

// MARK: - Computer row

final class SidebarComputerCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("SidebarComputerCell")

    private let icon = NSImageView()
    private let title = Build.label("", font: .systemFont(ofSize: 13))
    private let detail = Build.label(
        "", font: Theme.Font.caption, color: .tertiaryLabelColor, alignment: .right)
    private let dot = StatusDotView(size: 6)

    init() {
        super.init(frame: .zero)
        icon.translatesAutoresizingMaskIntoConstraints = false
        detail.setContentCompressionResistancePriority(.required, for: .horizontal)
        detail.setContentHuggingPriority(.required, for: .horizontal)

        for subview in [icon, title, detail, dot] as [NSView] { addSubview(subview) }

        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: SidebarMetric.inset),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: SidebarMetric.slot),

            title.leadingAnchor.constraint(equalTo: leadingAnchor, constant: SidebarMetric.textLeading),
            title.centerYAnchor.constraint(equalTo: centerYAnchor),
            title.trailingAnchor.constraint(lessThanOrEqualTo: detail.leadingAnchor, constant: -6),

            detail.trailingAnchor.constraint(equalTo: dot.leadingAnchor, constant: -6),
            detail.centerYAnchor.constraint(equalTo: centerYAnchor),

            dot.trailingAnchor.constraint(
                equalTo: trailingAnchor, constant: -SidebarMetric.trailingInset),
            dot.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])

        textField = title
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(computer: Computer, store: AppStore) {
        icon.image = NSImage(systemSymbolName: computer.symbolName, accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 15, weight: .regular)
        title.stringValue = computer.name
        let count = store.bots(on: computer.id).count
        detail.stringValue = computer.isThisComputer ? "This Mac" : "\(count) bot\(count == 1 ? "" : "s")"
        dot.status = computer.status
        applyBackgroundStyle()
    }

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { applyBackgroundStyle() }
    }

    private func applyBackgroundStyle() {
        let emphasized = backgroundStyle == .emphasized
        detail.textColor = emphasized ? NSColor.white.withAlphaComponent(0.7) : .tertiaryLabelColor
        icon.contentTintColor = emphasized ? .white : .secondaryLabelColor
    }
}
