import AppKit

/// Save as Skill on a bot's reply, or Save as Standing Instruction on the user's own message: the
/// messages around it to pick from, and in a group whether the skill is the group's or the bot's.
/// Only the picked messages go to the bot's provider, which writes a draft; the draft opens for
/// review and is used once it is saved.
final class PlaybookCaptureViewController: SheetViewController {
    private let store = AppStore.shared
    private let chat: Chat
    private let botID: Bot.ID
    private let kind: String
    private let candidates: [Message]
    private var picked: Set<Message.ID>
    private var rows: [Message.ID: SelectableMessageRow] = [:]
    private let scopePopup = NSPopUpButton()
    private let scroll = NSScrollView()
    private let spinner = NSProgressIndicator()
    private let status = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
    private var isDrafting = false

    /// Called with the draft, after this sheet closes, to open it for review.
    var onDrafted: ((PlaybookRecord) -> Void)?

    init(chat: Chat, message: Message) {
        self.chat = chat
        kind = message.author.isYou ? "corrections" : "workflow"
        let botID = message.author.botID ?? chat.owner ?? chat.botIDs.first ?? ""
        self.botID = botID
        // The messages up to the one clicked: the user's and this bot's for a workflow, the
        // user's alone for corrections.
        let usable = chat.messages.filter { candidate in
            candidate.canBeQuoted && !candidate.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                && (candidate.author.isYou || (message.author.isYou ? false : candidate.author.botID == botID))
        }
        let end = usable.firstIndex { $0.id == message.id }.map { $0 + 1 } ?? usable.count
        candidates = Array(usable[..<end].suffix(12))
        var picked: Set<Message.ID> = [message.id]
        if !message.author.isYou, let request = usable[..<end].last(where: { $0.author.isYou }) { picked.insert(request.id) }
        self.picked = picked
        super.init(
            title: kind == "workflow" ? L("Save as Skill") : L("Save as Standing Instruction"),
            subtitle: kind == "workflow"
                ? L("Pick the messages that show how it's done. You review the draft next.")
                : L("Pick the corrections it should cover. You review the draft next."),
            width: 480)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        let botName = store.bot(botID)?.name ?? L("The bot")
        if chat.isGroup {
            scopePopup.addItems(withTitles: [L("The bots in %@", store.title(for: chat)), L("%@, in every chat", botName)])
            scopePopup.translatesAutoresizingMaskIntoConstraints = false
            let label = Build.label(L("Used by"), font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
            let row = Build.stack([label, scopePopup], orientation: .horizontal, spacing: 10)
            contentStack.addArrangedSubview(row)
            row.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
        }

        let list = SectionView(title: kind == "workflow" ? L("Messages") : L("Your messages"))
        list.setRows(candidates.map { message in
            let row = SelectableMessageRow()
            // Corrections are all the user's; the author only tells a workflow's turns apart.
            row.configure(author: kind == "workflow" ? (message.author.isYou ? L("You") : botName) : nil, text: message.text)
            row.isSelected = picked.contains(message.id)
            row.onToggle = { [weak self] in self?.toggle(message.id) }
            rows[message.id] = row
            return row
        })
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(list)
        scroll.documentView = document
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        contentStack.addArrangedSubview(scroll)
        let height = scroll.heightAnchor.constraint(equalTo: document.heightAnchor)
        height.priority = .defaultHigh
        NSLayoutConstraint.activate([
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            height,
            scroll.heightAnchor.constraint(lessThanOrEqualToConstant: 320),
            document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
            list.topAnchor.constraint(equalTo: document.topAnchor),
            list.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            list.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            list.bottomAnchor.constraint(equalTo: document.bottomAnchor),
        ])

        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false
        let progress = Build.stack([spinner, status], orientation: .horizontal, spacing: 8)
        setButtons(confirm: L("Continue"), leading: progress)
        update()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        // The message clicked is the last one; the list opens on it.
        if let document = scroll.documentView { document.scroll(NSPoint(x: 0, y: document.bounds.maxY)) }
    }

    private func toggle(_ id: Message.ID) {
        guard !isDrafting else { return }
        if !picked.insert(id).inserted { picked.remove(id) }
        update()
    }

    /// A workflow needs a reply from the bot; a standing instruction, two corrections.
    private var isEnough: Bool {
        let messages = candidates.filter { picked.contains($0.id) }
        return kind == "workflow" ? messages.contains { !$0.author.isYou } : messages.count >= 2
    }

    private func update() {
        for (id, row) in rows { row.isSelected = picked.contains(id) }
        confirmButton.isEnabled = isEnough && !isDrafting
        scopePopup.isEnabled = !isDrafting
    }

    override func confirmTapped() {
        guard isEnough, !isDrafting else { return }
        let scope: PlaybookScope = chat.isGroup && scopePopup.indexOfSelectedItem == 0 ? .group(chat.id) : .bot(botID)
        let ids = candidates.map(\.id).filter { picked.contains($0) }
        isDrafting = true
        spinner.startAnimation(nil)
        status.stringValue = L("Writing a draft…")
        update()
        Task { [weak self] in
            guard let self else { return }
            do {
                let draft = try await store.draftPlaybook(in: scope, botID: botID, chatID: chat.id, kind: kind, messageIDs: ids)
                let onDrafted = onDrafted
                dismiss(nil)
                onDrafted?(draft)
            } catch {
                isDrafting = false
                spinner.stopAnimation(nil)
                status.stringValue = ""
                update()
                guard let window = view.window else { return }
                let alert = NSAlert()
                alert.messageText = L("Couldn't write the draft")
                alert.informativeText = error.localizedDescription
                alert.beginSheetModal(for: window, completionHandler: nil)
            }
        }
    }
}

/// A message to pick for a skill: a check, who wrote it, and its first lines.
final class SelectableMessageRow: NSView {
    private let check = NSImageView()
    private let author = Build.label("", font: .systemFont(ofSize: 12, weight: .semibold))
    private let preview = Build.label("", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 2)
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    var onToggle: (() -> Void)?

    var isSelected = false {
        didSet {
            check.image = NSImage(systemSymbolName: isSelected ? "checkmark.circle.fill" : "circle", accessibilityDescription: nil)
            check.contentTintColor = isSelected ? .controlAccentColor : .tertiaryLabelColor
            setAccessibilityValue(isSelected)
        }
    }

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        check.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .regular)
        check.translatesAutoresizingMaskIntoConstraints = false
        preview.cell?.truncatesLastVisibleLine = true
        let text = Build.stack([author, preview], spacing: 2)
        addSubview(check)
        addSubview(text)
        NSLayoutConstraint.activate([
            check.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 12),
            check.widthAnchor.constraint(equalToConstant: 18),
            check.topAnchor.constraint(equalTo: text.topAnchor, constant: -1),
            text.leadingAnchor.constraint(equalTo: check.trailingAnchor, constant: 10),
            text.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -12),
            text.topAnchor.constraint(equalTo: topAnchor, constant: 9),
            text.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -9),
            preview.widthAnchor.constraint(equalTo: text.widthAnchor),
        ])
        setAccessibilityElement(true)
        setAccessibilityRole(.checkBox)
        isSelected = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(author name: String?, text: String) {
        author.stringValue = name ?? ""
        author.isHidden = name == nil
        preview.textColor = name == nil ? .labelColor : .secondaryLabelColor
        preview.stringValue = text.split(whereSeparator: \.isNewline).joined(separator: " ")
        toolTip = text
        setAccessibilityLabel(name.map { "\($0): \(preview.stringValue)" } ?? preview.stringValue)
    }

    override func accessibilityPerformPress() -> Bool {
        onToggle?()
        return true
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { isHovered = true }
    override func mouseExited(with event: NSEvent) { isHovered = false }
    override func mouseDown(with event: NSEvent) {}
    override func mouseUp(with event: NSEvent) { onToggle?() }

    override func draw(_ dirtyRect: NSRect) {
        guard isHovered else { return }
        NSColor.labelColor.withAlphaComponent(0.05).setFill()
        bounds.fill()
    }
}
