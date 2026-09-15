import AppKit

/// Bot picker for a new chat. A DM pins one bot for good; a group starts with one to six bots
/// and can change members later.
final class NewChatViewController: SheetViewController {
    private let store = AppStore.shared
    private let kindControl = NSSegmentedControl(
        labels: ["Direct Message", "Group Chat"], trackingMode: .selectOne, target: nil, action: nil)
    private let nameField = NSTextField()
    private let footnote = Build.label(
        "", font: Theme.Font.caption, color: .tertiaryLabelColor, lines: 0)
    private var kind: Chat.Kind = .dm
    private var selected: [Bot.ID] = []
    private var rows: [Bot.ID: SelectableBotRow] = [:]

    private let onCreate: (Chat.Kind, [Bot.ID], String?) -> Void

    init(onCreate: @escaping (Chat.Kind, [Bot.ID], String?) -> Void) {
        self.onCreate = onCreate
        super.init(
            title: "New Chat",
            subtitle: "Message one bot directly, or start a group of up to six.",
            width: 440
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        kindControl.segmentDistribution = .fillEqually
        kindControl.selectedSegment = 0
        kindControl.target = self
        kindControl.action = #selector(kindChanged)
        kindControl.translatesAutoresizingMaskIntoConstraints = false

        nameField.placeholderString = "Group name (optional)"
        nameField.translatesAutoresizingMaskIntoConstraints = false
        nameField.isHidden = true

        let list = SectionView(title: "Bots")
        list.setRows(
            store.bots.map { bot in
                let host = store.computer(bot.computerID)
                let row = SelectableBotRow()
                row.configure(
                    bot: bot,
                    detail: "\(bot.tagline) · \(host?.name ?? "unassigned")",
                    isOffline: host?.status == .offline
                )
                row.onToggle = { [weak self] in self?.toggle(bot.id) }
                rows[bot.id] = row
                return row
            })

        contentStack.addArrangedSubview(kindControl)
        contentStack.addArrangedSubview(list)
        contentStack.addArrangedSubview(nameField)
        contentStack.addArrangedSubview(footnote)
        contentStack.setCustomSpacing(16, after: kindControl)

        NSLayoutConstraint.activate([
            kindControl.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            list.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            nameField.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            footnote.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
        ])

        setButtons(confirm: "Create")
        if let first = store.bots.first { toggle(first.id) }
        updateState()
    }

    @objc private func kindChanged() {
        kind = kindControl.selectedSegment == 0 ? .dm : .group
        // A DM has exactly one bot; keep the first pick when narrowing from a group.
        if kind == .dm, selected.count > 1 { selected = [selected[0]] }
        updateState()
    }

    private func toggle(_ id: Bot.ID) {
        switch kind {
        case .dm:
            selected = [id]
        case .group:
            if let index = selected.firstIndex(of: id) {
                selected.remove(at: index)
            } else if selected.count < Chat.maxGroupBots {
                selected.append(id)
            } else {
                NSSound.beep()
            }
        }
        updateState()
    }

    /// The DM that already exists for the single selected bot, if any.
    private var existingDM: Chat? {
        guard kind == .dm, selected.count == 1 else { return nil }
        return store.chats.first { $0.isDM && $0.botIDs == selected }
    }

    private func updateState() {
        let roomLeft = kind == .dm || selected.count < Chat.maxGroupBots
        for (id, row) in rows {
            row.isSelected = selected.contains(id)
            row.isEnabled = selected.contains(id) || roomLeft
        }

        nameField.isHidden = kind == .dm
        confirmButton.isEnabled = !selected.isEmpty
        confirmButton.title = existingDM == nil ? "Create" : "Open"

        switch (kind, selected.count) {
        case (.dm, 0):
            footnote.stringValue = "Pick a bot to message."
        case (.dm, _):
            let bot = selected.first.flatMap(store.bot)
            let host = bot.flatMap { store.computer($0.computerID) }
            footnote.stringValue =
                existingDM != nil
                ? "You already have a direct message with \(bot?.name ?? "this bot"). Open goes to that thread."
                : "Turns run on \(host?.name ?? "its Computer") with that machine's \(bot?.provider.rawValue ?? "provider") credentials. A DM stays one-to-one."
        case (.group, 0):
            footnote.stringValue = "Pick at least one bot. You can add more later."
        case (.group, 1):
            let bot = selected.first.flatMap(store.bot)
            let host = bot.flatMap { store.computer($0.computerID) }
            footnote.stringValue =
                "A group of one on \(host?.name ?? "its Computer"). Add bots any time from the inspector."
        case (.group, let count):
            let hosts = Set(selected.compactMap { store.bot($0)?.computerID })
            footnote.stringValue =
                "\(count) bots across \(hosts.count) Computer\(hosts.count == 1 ? "" : "s"). Address one with @Name, or all of them with @everyone."
        }
    }

    override func confirmTapped() {
        guard !selected.isEmpty else { return }
        let name = nameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        onCreate(kind, selected, kind == .group && !name.isEmpty ? name : nil)
        dismiss(nil)
    }
}

/// Single-select variant used when adding a bot to an existing chat.
final class BotPickerViewController: SheetViewController {
    private let bots: [Bot]
    private let onPick: (Bot.ID) -> Void
    private var selected: Bot.ID?
    private var rows: [Bot.ID: SelectableBotRow] = [:]

    init(title: String, bots: [Bot], onPick: @escaping (Bot.ID) -> Void) {
        self.bots = bots
        self.onPick = onPick
        super.init(title: title, subtitle: "A group chat holds up to six bots.", width: 420)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        let store = AppStore.shared
        let list = SectionView(title: "Available")
        list.setRows(
            bots.map { bot in
                let host = store.computer(bot.computerID)
                let row = SelectableBotRow()
                row.configure(
                    bot: bot,
                    detail: "\(bot.tagline) · \(host?.name ?? "unassigned")",
                    isOffline: host?.status == .offline
                )
                row.onToggle = { [weak self] in self?.select(bot.id) }
                rows[bot.id] = row
                return row
            })

        contentStack.addArrangedSubview(list)
        list.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        setButtons(confirm: "Add")
        if let first = bots.first { select(first.id) }
    }

    private func select(_ id: Bot.ID) {
        selected = id
        for (rowID, row) in rows { row.isSelected = rowID == id }
        confirmButton.isEnabled = true
    }

    override func confirmTapped() {
        guard let selected else { return }
        onPick(selected)
        dismiss(nil)
    }
}

final class SelectableBotRow: NSView {
    private let check = NSImageView()
    private let avatar = AvatarView(diameter: 28)
    private let name = Build.label("", font: .systemFont(ofSize: 13, weight: .medium))
    private let detail = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private let offline = Build.label(
        "offline", font: .systemFont(ofSize: 10, weight: .medium), color: .systemOrange)
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    var onToggle: (() -> Void)?

    var isSelected = false {
        didSet {
            check.image = NSImage(
                systemSymbolName: isSelected ? "checkmark.circle.fill" : "circle",
                accessibilityDescription: nil)
            check.contentTintColor = isSelected ? .controlAccentColor : .tertiaryLabelColor
        }
    }

    var isEnabled = true {
        didSet { alphaValue = isEnabled ? 1 : 0.45 }
    }

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true

        check.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .regular)
        check.translatesAutoresizingMaskIntoConstraints = false
        offline.setContentCompressionResistancePriority(.required, for: .horizontal)

        let text = Build.stack([name, detail], spacing: 1)
        addSubview(check)
        addSubview(avatar)
        addSubview(text)
        addSubview(offline)

        NSLayoutConstraint.activate([
            heightAnchor.constraint(greaterThanOrEqualToConstant: 46),
            check.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            check.centerYAnchor.constraint(equalTo: centerYAnchor),
            avatar.leadingAnchor.constraint(equalTo: check.trailingAnchor, constant: 10),
            avatar.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.leadingAnchor.constraint(equalTo: avatar.trailingAnchor, constant: 9),
            text.centerYAnchor.constraint(equalTo: centerYAnchor),
            text.trailingAnchor.constraint(lessThanOrEqualTo: offline.leadingAnchor, constant: -8),
            offline.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            offline.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])

        isSelected = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(bot: Bot, detail detailText: String, isOffline: Bool) {
        avatar.content = .bot(symbolName: bot.symbolName, accent: bot.accent)
        name.stringValue = bot.name
        detail.stringValue = detailText
        offline.isHidden = !isOffline
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

    override func mouseEntered(with event: NSEvent) { isHovered = true }
    override func mouseExited(with event: NSEvent) { isHovered = false }

    override func mouseUp(with event: NSEvent) {
        guard isEnabled else { return }
        onToggle?()
    }

    override func draw(_ dirtyRect: NSRect) {
        guard isHovered, isEnabled else { return }
        NSColor.labelColor.withAlphaComponent(0.05).setFill()
        bounds.fill()
    }
}
