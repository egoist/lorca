import AppKit

extension Device {
    var symbolName: String {
        switch os {
        case .macos:
            if model.contains("MacBook") { return "laptopcomputer" }
            if model.contains("Studio") { return "macstudio" }
            if model.contains("mini") { return "macmini" }
            return "desktopcomputer"
        case .linux: return "server.rack"
        case .windows: return "pc"
        case .ios: return "iphone"
        case .ipados: return "ipad"
        case .android: return "smartphone"
        }
    }
}

final class SidebarNode: NSObject {
    enum Kind: Hashable {
        case header(String)
        case chat(Chat.ID)
        case pane(SettingsPane)
        /// A search result in the settings sidebar: one setting on its pane.
        case setting(SettingsEntry)
        case device(Device.ID)
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
        case let .pane(pane): .settings(pane)
        case let .setting(entry): .settings(entry.pane)
        case let .device(id): .device(id)
        }
    }

    var isHeader: Bool {
        if case .header = kind { return true }
        return false
    }
}

/// Shared column geometry. Chat and device rows use the same leading inset and
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
    private lazy var button = HoverButton(
        symbol: "plus", pointSize: 11, tooltip: "", target: self, action: #selector(performAction))
    private var onAction: (() -> Void)?

    init() {
        super.init(frame: .zero)
        button.translatesAutoresizingMaskIntoConstraints = false
        button.isHidden = true
        addSubview(label)
        addSubview(button)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: leadingAnchor, constant: SidebarMetric.inset + 2),
            label.trailingAnchor.constraint(lessThanOrEqualTo: button.leadingAnchor, constant: -4),
            label.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -5),

            button.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -SidebarMetric.trailingInset + 4),
            button.centerYAnchor.constraint(equalTo: label.centerYAnchor),
            button.widthAnchor.constraint(equalToConstant: 20),
            button.heightAnchor.constraint(equalToConstant: 20),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// A header may carry one action for its section, shown as a plus at the trailing edge.
    func configure(_ title: String, actionTooltip: String? = nil, onAction: (() -> Void)? = nil) {
        label.stringValue = title
        self.onAction = onAction
        button.isHidden = onAction == nil
        button.toolTip = actionTooltip
        button.setAccessibilityLabel(actionTooltip)
    }

    @objc private func performAction() {
        onAction?()
    }
}

// MARK: - Settings pane row

extension SettingsPane {
    var title: String {
        switch self {
        case .general: "General"
        case .providers: "Providers"
        case .autoReview: "Auto-review"
        case .advanced: "Advanced"
        }
    }

    var symbolName: String {
        switch self {
        case .general: "gearshape"
        case .providers: "key"
        case .autoReview: "checkmark.shield"
        case .advanced: "slider.horizontal.3"
        }
    }
}

final class SidebarPaneCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("SidebarPaneCell")

    private let icon = NSImageView()
    private let title = Build.label("", font: .systemFont(ofSize: 13))

    init() {
        super.init(frame: .zero)
        icon.translatesAutoresizingMaskIntoConstraints = false
        addSubview(icon)
        addSubview(title)

        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor, constant: SidebarMetric.inset),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            icon.widthAnchor.constraint(equalToConstant: SidebarMetric.slot),

            title.leadingAnchor.constraint(equalTo: leadingAnchor, constant: SidebarMetric.textLeading),
            title.centerYAnchor.constraint(equalTo: centerYAnchor),
            title.trailingAnchor.constraint(
                lessThanOrEqualTo: trailingAnchor, constant: -SidebarMetric.trailingInset),
        ])

        textField = title
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(pane: SettingsPane) {
        icon.image = NSImage(systemSymbolName: pane.symbolName, accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 15, weight: .regular)
        title.stringValue = pane.title
        applyBackgroundStyle()
    }

    override var backgroundStyle: NSView.BackgroundStyle {
        didSet { applyBackgroundStyle() }
    }

    private func applyBackgroundStyle() {
        icon.contentTintColor = backgroundStyle == .emphasized ? .white : .secondaryLabelColor
    }
}

// MARK: - Setting search result

/// A setting found by the settings search, listed under its pane with the text on the pane
/// title's column.
final class SidebarSettingCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("SidebarSettingCell")

    private let title = Build.label("", font: .systemFont(ofSize: 12))

    init() {
        super.init(frame: .zero)
        addSubview(title)
        NSLayoutConstraint.activate([
            title.leadingAnchor.constraint(equalTo: leadingAnchor, constant: SidebarMetric.textLeading),
            title.centerYAnchor.constraint(equalTo: centerYAnchor),
            title.trailingAnchor.constraint(
                lessThanOrEqualTo: trailingAnchor, constant: -SidebarMetric.trailingInset),
        ])
        textField = title
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(entry: SettingsEntry) {
        title.stringValue = entry.title
        toolTip = entry.title
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
    /// Unread activity: a dot, not a count.
    private let badge = BackgroundView()

    /// The row's ⌘-number, shown in the stamp's place while ⌘ is held.
    var shortcutNumber: Int? {
        didSet { if shortcutNumber != oldValue { updateStamp() } }
    }
    private var stampText = ""

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

        badge.cornerRadius = 4
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
                equalTo: trailingAnchor, constant: -SidebarMetric.trailingInset - 2),
            badge.centerYAnchor.constraint(equalTo: preview.centerYAnchor),
            badge.heightAnchor.constraint(equalToConstant: 8),
            badge.widthAnchor.constraint(equalToConstant: 8),
        ])

        textField = title
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(chat: Chat, store: AppStore) {
        avatars.configure(with: store.bots(in: chat))
        avatars.isWorking = chat.botIDs.contains { store.isWorking($0) }
        title.stringValue = store.title(for: chat)
        preview.stringValue = store.preview(for: chat)
        stampText = Format.stamp(chat.lastActivity)
        updateStamp()

        pin.isHidden = !chat.isPinned
        titleBeforePin.isActive = chat.isPinned
        titleBeforeStamp.isActive = !chat.isPinned

        let unread = chat.unreadCount
        badge.isHidden = unread == 0
        badge.setAccessibilityLabel(unread > 0 ? "Unread activity" : nil)
        previewBeforeBadge.isActive = unread > 0
        previewBeforeEdge.isActive = unread == 0
        setAccessibilityLabel(
            [title.stringValue, unread > 0 ? "Unread activity" : nil, avatars.isWorking ? "Working" : nil]
                .compactMap { $0 }.joined(separator: ", "))
        applyBackgroundStyle()
    }

    private func updateStamp() {
        stamp.stringValue = shortcutNumber.map { "⌘\($0)" } ?? stampText
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
    }
}

// MARK: - Device row

final class SidebarDeviceCell: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("SidebarDeviceCell")

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

    func configure(device: Device, store: AppStore) {
        icon.image = NSImage(systemSymbolName: device.symbolName, accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 15, weight: .regular)
        title.stringValue = device.name
        let count = store.bots(on: device.id).count
        detail.stringValue =
            if device.isThisDevice {
                "This Mac"
            } else if device.isRunner {
                "\(count) bot\(count == 1 ? "" : "s")"
            } else {
                device.os.displayName
            }
        dot.status = device.status
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
