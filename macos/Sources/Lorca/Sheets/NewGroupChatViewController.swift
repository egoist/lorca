import AppKit

/// Bot picker for a new group chat: one to six bots, with members changeable later. Direct chats
/// need no picker because every bot gets one when it is created.
final class NewGroupChatViewController: SheetViewController {
    private let store = AppStore.shared
    private let nameField = NSTextField()
    private var selected: [Bot.ID] = []
    private var rows: [Bot.ID: SelectableBotRow] = [:]

    private let onCreate: ([Bot.ID], String?) -> Void

    init(onCreate: @escaping ([Bot.ID], String?) -> Void) {
        self.onCreate = onCreate
        super.init(
            title: L("New Group Chat"),
            subtitle: L("Pick up to six bots to talk with together."),
            width: 440
        )
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        nameField.placeholderString = L("Group name (optional)")
        nameField.translatesAutoresizingMaskIntoConstraints = false

        let list = SectionView(title: L("Bots"))
        list.setRows(
            store.bots.map { bot in
                let host = store.device(bot.runnerID)
                let row = SelectableBotRow()
                row.configure(
                    bot: bot,
                    detail: "\(bot.label) · \(host?.name ?? L("unassigned"))",
                    isOffline: host?.status == .offline
                )
                row.onToggle = { [weak self] in self?.toggle(bot.id) }
                rows[bot.id] = row
                return row
            })

        contentStack.addArrangedSubview(list)
        contentStack.addArrangedSubview(nameField)

        NSLayoutConstraint.activate([
            list.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            nameField.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
        ])

        setButtons(confirm: L("Create"))
        if let first = store.bots.first { toggle(first.id) }
        updateState()
    }

    private func toggle(_ id: Bot.ID) {
        if let index = selected.firstIndex(of: id) {
            selected.remove(at: index)
        } else if selected.count < Chat.maxGroupBots {
            selected.append(id)
        } else {
            NSSound.beep()
        }
        updateState()
    }

    private func updateState() {
        let roomLeft = selected.count < Chat.maxGroupBots
        for (id, row) in rows {
            row.isSelected = selected.contains(id)
            row.isEnabled = selected.contains(id) || roomLeft
        }
        confirmButton.isEnabled = !selected.isEmpty
    }

    override func confirmTapped() {
        guard !selected.isEmpty else { return }
        let name = nameField.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        onCreate(selected, name.isEmpty ? nil : name)
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
        super.init(title: title, subtitle: L("A group chat holds up to six bots."), width: 420)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        let store = AppStore.shared
        let list = SectionView(title: L("Available"))
        list.setRows(
            bots.map { bot in
                let host = store.device(bot.runnerID)
                let row = SelectableBotRow()
                row.configure(
                    bot: bot,
                    detail: "\(bot.label) · \(host?.name ?? L("unassigned"))",
                    isOffline: host?.status == .offline
                )
                row.onToggle = { [weak self] in self?.select(bot.id) }
                rows[bot.id] = row
                return row
            })

        contentStack.addArrangedSubview(list)
        list.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        setButtons(confirm: L("Add"))
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
        L("offline"), font: .systemFont(ofSize: 10, weight: .medium), color: .systemOrange)
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
        avatar.content = AvatarView.content(for: bot)
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
