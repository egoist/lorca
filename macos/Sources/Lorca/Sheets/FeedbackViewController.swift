import AppKit

// Workflow feedback: what the user says about a bot's messages, the changes to its routines and
// skills the bot suggests from it, and the changes the user accepted. The bot's Runner keeps all
// of it and applies every decision; these sheets only ask.

extension FeedbackNote.Kind {
    var symbol: String {
        switch self {
        case .accepted: "checkmark.circle"
        case .rejected: "xmark.circle"
        case .edited: "pencil"
        case .explicit: "bubble.left"
        case .routineFailure: "exclamationmark.triangle"
        case .ignoredAlert: "eye.slash"
        }
    }

    var title: String {
        switch self {
        case .accepted: L("Good")
        case .rejected: L("Not right")
        case .edited: L("Corrected")
        case .explicit: L("Note")
        case .routineFailure: L("Run failed")
        case .ignoredAlert: L("Ignored")
        }
    }
}

/// Words for what the Runner refused, in the app's terms.
private func feedbackProblem(_ error: Error, name: String) -> String {
    let message = error.localizedDescription
    if message.contains("review a fresh proposal") || message.contains("cannot overwrite later edits") {
        return L("“%@” changed after this, so the change can't be made.", name)
    }
    if message.contains("displayed diff changed") || message.contains("no longer pending") {
        return L("This suggestion changed on another Device.")
    }
    if message.contains("no longer included") {
        return L("This suggestion is based on feedback you excluded.")
    }
    return message
}

private func showProblem(_ title: String, _ text: String, in controller: NSViewController) {
    let alert = NSAlert()
    alert.messageText = title
    alert.informativeText = text
    if let window = controller.view.window { alert.beginSheetModal(for: window) } else { alert.runModal() }
}

/// A note, a suggestion, or a change as a row: a symbol, a title over a detail line. Clicking
/// it opens what it is about; it fills on hover, as a routine's row does.
final class FeedbackRow: NSView {
    private let row = StatusRow()
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }
    var onClick: (() -> Void)?

    init(symbol: String, title: String, detail: String, tooltip: String? = nil) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        row.configure(symbol: symbol, title: title, subtitle: detail, state: nil)
        toolTip = tooltip
        addSubview(row)
        NSLayoutConstraint.activate([
            row.leadingAnchor.constraint(equalTo: leadingAnchor),
            row.trailingAnchor.constraint(equalTo: trailingAnchor),
            row.topAnchor.constraint(equalTo: topAnchor),
            row.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
        addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(clicked)))
    }

    /// A note: its words, or the routine a failed run belongs to, over its kind and when.
    convenience init(note: FeedbackNote, in feedback: BotFeedback) {
        let failed = note.kind == .routineFailure
        let title = failed ? note.target.map(feedback.name(of:)) ?? note.text : note.text
        self.init(
            symbol: note.kind.symbol, title: title.isEmpty ? note.kind.title : title,
            detail: "\(note.kind.title) · \(Format.stamp(note.createdAt))", tooltip: note.text)
    }

    /// A suggestion: what it changes; why is its tooltip and in its sheet.
    convenience init(suggestion: FeedbackSuggestion, in feedback: BotFeedback) {
        self.init(symbol: "sparkles", title: feedback.name(of: suggestion.target), detail: L("Suggested change"), tooltip: suggestion.explanation)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    @objc private func clicked() { onClick?() }

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

/// The changed lines of a suggestion or a change: what goes on red, what comes in on green, as a
/// document's tracked changes read. Long changes scroll.
final class FeedbackDiffView: NSView {
    init(diff: String, width: CGFloat) {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        let lines = Build.stack([], spacing: 0)
        lines.alignment = .leading
        for line in DiffLine.lines(of: diff) {
            let (mark, text, color): (String, String, NSColor) =
                switch line {
                case let .removed(text): ("−", text, .systemRed)
                case let .added(text): ("+", text, .systemGreen)
                }
            let band = BackgroundView()
            band.cornerRadius = 0
            band.fillColor = color.withAlphaComponent(0.13)
            let gutter = Build.label(mark, font: .monospacedSystemFont(ofSize: 11, weight: .medium), color: color)
            let label = Build.label(text.isEmpty ? " " : text, font: .systemFont(ofSize: 12), lines: 0)
            label.isSelectable = true
            label.preferredMaxLayoutWidth = width - 12 - 12 - 8 - 12
            gutter.setContentHuggingPriority(.required, for: .horizontal)
            band.addSubview(gutter)
            band.addSubview(label)
            NSLayoutConstraint.activate([
                gutter.leadingAnchor.constraint(equalTo: band.leadingAnchor, constant: 12),
                gutter.firstBaselineAnchor.constraint(equalTo: label.firstBaselineAnchor),
                gutter.widthAnchor.constraint(equalToConstant: 12),
                label.leadingAnchor.constraint(equalTo: gutter.trailingAnchor, constant: 8),
                label.trailingAnchor.constraint(lessThanOrEqualTo: band.trailingAnchor, constant: -12),
                label.topAnchor.constraint(equalTo: band.topAnchor, constant: 5),
                label.bottomAnchor.constraint(equalTo: band.bottomAnchor, constant: -5),
            ])
            lines.addArrangedSubview(band)
            band.widthAnchor.constraint(equalTo: lines.widthAnchor).isActive = true
        }

        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(lines)
        let scroll = NSScrollView()
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.documentView = document
        addSubview(scroll)
        NSLayoutConstraint.activate([
            lines.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            lines.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            lines.topAnchor.constraint(equalTo: document.topAnchor),
            lines.bottomAnchor.constraint(equalTo: document.bottomAnchor),
            document.topAnchor.constraint(equalTo: scroll.contentView.topAnchor),
            document.leadingAnchor.constraint(equalTo: scroll.contentView.leadingAnchor),
            document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
            scroll.leadingAnchor.constraint(equalTo: leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: trailingAnchor),
            scroll.topAnchor.constraint(equalTo: topAnchor),
            scroll.bottomAnchor.constraint(equalTo: bottomAnchor),
            scroll.heightAnchor.constraint(equalToConstant: min(lines.fittingSize.height, 240)),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }
}

// MARK: - Giving feedback

/// Feedback on one of a bot's messages: whether it was right, a corrected version, or a note for
/// next time, and optionally which routine or skill it is about. Opened from the message's menu.
final class FeedbackViewController: SheetViewController {
    private let store = AppStore.shared
    private let botID: Bot.ID
    private let chatID: Chat.ID
    private let message: Message

    private static let kinds: [(FeedbackNote.Kind, String)] = [
        (.accepted, L("Looks good")),
        (.rejected, L("Not what I wanted")),
        (.edited, L("I corrected it")),
        (.explicit, L("A note for next time")),
    ]
    private let response = NSPopUpButton()
    private let correctedLabel = Build.label(L("Your version"), font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
    private let correctedScroll = NSScrollView()
    private let corrected = NSTextView()
    private let note = WrappingTextField()
    private let about = NSPopUpButton()
    private var targets: [BotFeedback.Target] = []
    private var grid: NSGridView!

    init(botID: Bot.ID, chatID: Chat.ID, message: Message) {
        self.botID = botID
        self.chatID = chatID
        self.message = message
        let name = AppStore.shared.bot(botID)?.name ?? ""
        super.init(title: L("Feedback"), subtitle: L("%@ uses it to suggest changes to its routines and skills.", name), width: 460)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        response.addItems(withTitles: Self.kinds.map(\.1))
        response.target = self
        response.action = #selector(responseChanged)

        corrected.isRichText = false
        corrected.font = .systemFont(ofSize: 12)
        corrected.textColor = .labelColor
        corrected.allowsUndo = true
        corrected.textContainerInset = NSSize(width: 4, height: 6)
        corrected.isVerticallyResizable = true
        corrected.isHorizontallyResizable = false
        corrected.autoresizingMask = [.width]
        corrected.textContainer?.widthTracksTextView = true
        corrected.string = message.text
        corrected.delegate = self
        corrected.setAccessibilityLabel(L("Your version"))
        correctedScroll.documentView = corrected
        correctedScroll.hasVerticalScroller = true
        correctedScroll.autohidesScrollers = true
        correctedScroll.borderType = .bezelBorder
        correctedScroll.translatesAutoresizingMaskIntoConstraints = false
        correctedScroll.heightAnchor.constraint(equalToConstant: 120).isActive = true

        note.font = .systemFont(ofSize: 13)
        note.lineBreakMode = .byWordWrapping
        note.cell?.wraps = true
        note.cell?.isScrollable = false
        note.maximumNumberOfLines = 0
        note.delegate = self
        note.translatesAutoresizingMaskIntoConstraints = false

        about.addItem(withTitle: L("Any"))
        about.isEnabled = false

        let labels = [L("Response"), L("Your version"), L("Note"), L("Applies to")]
        let width = ceil(labels.map { Build.label($0, font: .systemFont(ofSize: 12)).intrinsicContentSize.width }.max() ?? 0)
        let correctedBox = NSView()
        correctedBox.translatesAutoresizingMaskIntoConstraints = false
        correctedBox.addSubview(correctedLabel)
        NSLayoutConstraint.activate([
            correctedLabel.leadingAnchor.constraint(equalTo: correctedBox.leadingAnchor),
            correctedLabel.trailingAnchor.constraint(equalTo: correctedBox.trailingAnchor),
            correctedLabel.topAnchor.constraint(equalTo: correctedBox.topAnchor, constant: 6),
            correctedBox.bottomAnchor.constraint(greaterThanOrEqualTo: correctedLabel.bottomAnchor),
        ])
        grid = NSGridView(views: [
            [formLabel(labels[0]), response],
            [correctedBox, correctedScroll],
            [formLabel(labels[2]), note],
            [formLabel(labels[3]), about],
        ])
        grid.translatesAutoresizingMaskIntoConstraints = false
        grid.columnSpacing = 10
        grid.rowSpacing = 10
        grid.column(at: 0).width = width
        grid.column(at: 1).xPlacement = .fill
        for row in 0..<grid.numberOfRows { grid.row(at: row).yPlacement = .center }
        grid.row(at: 1).yPlacement = .top
        grid.cell(for: response)?.xPlacement = .leading
        grid.cell(for: about)?.xPlacement = .leading
        contentStack.addArrangedSubview(grid)
        grid.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        setButtons(confirm: L("Save"))
        responseChanged()
        loadTargets()
    }

    override func viewDidAppear() {
        super.viewDidAppear()
        view.window?.makeFirstResponder(note)
    }

    private func formLabel(_ text: String) -> NSTextField {
        Build.label(text, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
    }

    private var kind: FeedbackNote.Kind { Self.kinds[max(0, response.indexOfSelectedItem)].0 }

    /// The routines and skills the note can be about, as the Runner names them.
    private func loadTargets() {
        Task { [weak self] in
            guard let self, let feedback = try? await self.store.feedback(of: self.botID) else { return }
            self.targets = feedback.targets
            self.about.addItems(withTitles: feedback.targets.map(\.name))
            self.about.isEnabled = !feedback.targets.isEmpty
        }
    }

    @objc private func responseChanged() {
        grid.row(at: 1).isHidden = kind != .edited
        note.placeholderString =
            switch kind {
            case .accepted: L("What worked? (optional)")
            case .rejected: L("What was wrong? (optional)")
            case .edited: L("What did you change? (optional)")
            default: L("What should it do next time?")
            }
        updateSave()
        fitSheetToContent()
    }

    private func updateSave() {
        let trimmed = note.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        confirmButton.isEnabled =
            switch kind {
            case .explicit: !trimmed.isEmpty
            case .edited: corrected.string != message.text
            default: true
            }
    }

    override func confirmTapped() {
        let edited = kind == .edited
        let index = about.indexOfSelectedItem
        let target = index > 0 && index <= targets.count ? targets[index - 1].target : nil
        let text = note.stringValue.trimmingCharacters(in: .whitespacesAndNewlines)
        let (kind, before, after) = (kind, edited ? message.text : nil, edited ? corrected.string : nil)
        confirmButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                try await self.store.recordFeedback(
                    botID: self.botID, kind: kind, chatID: self.chatID, messageID: self.message.id, note: text,
                    before: before, after: after, target: target)
                self.dismiss(nil)
            } catch {
                self.updateSave()
                showProblem(L("Couldn't save your feedback"), error.localizedDescription, in: self)
            }
        }
    }
}

extension FeedbackViewController: NSTextFieldDelegate, NSTextViewDelegate {
    func controlTextDidChange(_ obj: Notification) {
        updateSave()
        fitSheetToContent()
    }

    func textDidChange(_ notification: Notification) {
        updateSave()
    }
}

// MARK: - A bot's feedback

/// Everything a bot keeps from the user's feedback: the changes it suggests, how often it looks
/// for them, the changes accepted, and the notes. Opened from the inspector.
final class FeedbackListViewController: SheetViewController {
    private let store = AppStore.shared
    private let bot: Bot
    private var feedback: BotFeedback
    private var looking = false
    private var lookedAndFoundNothing = false

    private let column = Build.stack([], spacing: 18)
    private let scroll = NSScrollView()
    private var scrollHeight: NSLayoutConstraint!
    private let suggestions = SectionView(title: L("Suggestions"))
    private let changes = SectionView(title: L("Changes"))
    private let notes = SectionView(title: L("Notes", context: "feedback"))
    private let lookButton = NSButton()

    var onShowMessage: ((Chat.ID, Message.ID) -> Void)?

    private static let intervals: [Int?] = [nil, 86_400, 7 * 86_400]

    init(bot: Bot, feedback: BotFeedback) {
        self.bot = bot
        self.feedback = feedback
        super.init(
            title: L("Feedback"),
            subtitle: L("%@ suggests changes to its routines and skills from your feedback. Nothing changes until you accept one.", bot.name),
            width: 480)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        column.alignment = .leading
        for section in [suggestions, changes, notes] {
            column.addArrangedSubview(section)
            section.widthAnchor.constraint(equalTo: column.widthAnchor).isActive = true
        }
        let document = FlippedView()
        document.translatesAutoresizingMaskIntoConstraints = false
        document.addSubview(column)
        scroll.translatesAutoresizingMaskIntoConstraints = false
        scroll.drawsBackground = false
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.documentView = document
        contentStack.addArrangedSubview(scroll)
        scrollHeight = scroll.heightAnchor.constraint(equalToConstant: 100)
        NSLayoutConstraint.activate([
            column.leadingAnchor.constraint(equalTo: document.leadingAnchor),
            column.trailingAnchor.constraint(equalTo: document.trailingAnchor),
            column.topAnchor.constraint(equalTo: document.topAnchor),
            column.bottomAnchor.constraint(equalTo: document.bottomAnchor),
            document.topAnchor.constraint(equalTo: scroll.contentView.topAnchor),
            document.leadingAnchor.constraint(equalTo: scroll.contentView.leadingAnchor),
            document.widthAnchor.constraint(equalTo: scroll.contentView.widthAnchor),
            scroll.widthAnchor.constraint(equalTo: contentStack.widthAnchor),
            scrollHeight,
        ])

        lookButton.title = L("Look Now")
        lookButton.bezelStyle = .rounded
        lookButton.target = self
        lookButton.action = #selector(lookNow)
        lookButton.toolTip = L("Look through new feedback for changes to suggest")
        setButtons(confirm: L("Done"), cancel: nil, leading: lookButton)
        render()
    }

    override func viewDidLoad() {
        super.viewDidLoad()
        store.observe(self) { [weak self] event in
            guard let self, case let .feedbackChanged(id) = event, id == self.bot.id else { return }
            self.reload()
        }
    }

    private func reload() {
        Task { [weak self] in
            guard let self, let fresh = try? await self.store.feedback(of: self.bot.id) else { return }
            self.feedback = fresh
            self.render()
        }
    }

    private func render() {
        let interval = PopUpRow(
            key: L("Look for changes"), items: [L("When asked"), L("Daily"), L("Weekly")],
            selected: Self.intervals.firstIndex(of: feedback.reviewEvery) ?? 2)
        interval.onChange = { [weak self] index in self?.setInterval(Self.intervals[index]) }
        var suggestionRows: [NSView] = feedback.suggestions.map { suggestion in
            let row = FeedbackRow(suggestion: suggestion, in: feedback)
            row.onClick = { [weak self] in self?.open(suggestion) }
            return row
        }
        if lookedAndFoundNothing && feedback.suggestions.isEmpty {
            suggestionRows.append(NoteRow(text: L("Nothing to change right now.")))
        }
        suggestions.setRows(suggestionRows + [interval])

        changes.isHidden = feedback.changes.isEmpty
        changes.setRows(feedback.changes.map { change in
            let row = FeedbackRow(
                symbol: change.isUndo ? "arrow.uturn.backward" : "pencil.line", title: feedback.name(of: change.target),
                detail: change.isUndo ? L("Undone %@", Format.stamp(change.createdAt)) : L("Changed %@", Format.stamp(change.createdAt)))
            row.onClick = { [weak self] in self?.open(change) }
            return row
        })

        notes.isHidden = feedback.notes.isEmpty
        notes.setRows(feedback.notes.map { note in
            let row = FeedbackRow(note: note, in: feedback)
            row.onClick = { [weak self] in self?.show(note) }
            row.menu = menu(for: note)
            return row
        })

        lookButton.isEnabled = !looking
        lookButton.title = looking ? L("Looking…") : L("Look Now")
        view.layoutSubtreeIfNeeded()
        scrollHeight.constant = min(column.fittingSize.height, 460)
        fitSheetToContent()
    }

    /// Show in Chat, and the exclusions: the note's words, or everything from its chat.
    private func menu(for note: FeedbackNote) -> NSMenu {
        let menu = NSMenu()
        let chat = store.chat(note.chatID)
        for (title, action) in [
            (L("Show in Chat"), #selector(showFromMenu(_:))),
            (L("Exclude"), #selector(excludeNote(_:))),
            (L("Exclude Everything from “%@”", chat.map(store.title(for:)) ?? ""), #selector(excludeChat(_:))),
        ] {
            let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
            item.target = self
            item.representedObject = note
            menu.addItem(item)
            if action == #selector(showFromMenu(_:)) { menu.addItem(.separator()) }
        }
        return menu
    }

    @objc private func showFromMenu(_ sender: NSMenuItem) {
        (sender.representedObject as? FeedbackNote).map(show)
    }

    @objc private func excludeNote(_ sender: NSMenuItem) {
        guard let note = sender.representedObject as? FeedbackNote else { return }
        exclude(note, wholeChat: false)
    }

    @objc private func excludeChat(_ sender: NSMenuItem) {
        guard let note = sender.representedObject as? FeedbackNote else { return }
        exclude(note, wholeChat: true)
    }

    private func show(_ note: FeedbackNote) {
        dismiss(nil)
        onShowMessage?(note.chatID, note.messageID)
    }

    private func open(_ suggestion: FeedbackSuggestion) {
        let sheet = FeedbackChangeViewController(bot: bot, feedback: feedback, item: .suggestion(suggestion))
        sheet.onShowMessage = { [weak self] chatID, messageID in
            self?.dismiss(nil)
            self?.onShowMessage?(chatID, messageID)
        }
        presentAsSheet(sheet)
    }

    private func open(_ change: FeedbackChange) {
        presentAsSheet(FeedbackChangeViewController(bot: bot, feedback: feedback, item: .change(change)))
    }

    private func exclude(_ note: FeedbackNote, wholeChat: Bool) {
        Task { [weak self] in
            guard let self else { return }
            do { try await self.store.exclude(note: note, wholeChat: wholeChat, botID: self.bot.id) }
            catch { showProblem(L("Couldn't exclude it"), error.localizedDescription, in: self) }
        }
    }

    private func setInterval(_ seconds: Int?) {
        Task { [weak self] in
            guard let self else { return }
            do { try await self.store.setFeedbackReview(every: seconds, botID: self.bot.id) }
            catch {
                self.render()
                showProblem(L("Couldn't change it"), error.localizedDescription, in: self)
            }
        }
    }

    @objc private func lookNow() {
        looking = true
        lookedAndFoundNothing = false
        render()
        Task { [weak self] in
            guard let self else { return }
            do {
                let found = try await self.store.suggestChanges(botID: self.bot.id)
                self.lookedAndFoundNothing = found == 0
            } catch {
                showProblem(L("Couldn't look for changes"), error.localizedDescription, in: self)
            }
            self.looking = false
            self.render()
        }
    }
}

// MARK: - A suggestion or a change

/// One suggestion, with why and the notes it is based on, to accept or reject; or one change
/// accepted before, to undo while the routine or skill still reads as the change left it.
final class FeedbackChangeViewController: SheetViewController {
    enum Item {
        case suggestion(FeedbackSuggestion)
        case change(FeedbackChange)
    }

    private let store = AppStore.shared
    private let bot: Bot
    private let feedback: BotFeedback
    private let item: Item
    private let name: String
    private let kind: String
    private let rejectButton = NSButton()

    var onShowMessage: ((Chat.ID, Message.ID) -> Void)?

    init(bot: Bot, feedback: BotFeedback, item: Item) {
        self.bot = bot
        self.feedback = feedback
        self.item = item
        switch item {
        case let .suggestion(suggestion):
            name = feedback.name(of: suggestion.target)
            kind = suggestion.target.kind
            super.init(title: name, subtitle: L("Suggested change"), width: 480)
        case let .change(change):
            name = feedback.name(of: change.target)
            kind = change.target.kind
            let when = Format.daySeparator(change.createdAt)
            super.init(title: name, subtitle: change.isUndo ? L("Undone %@", when) : L("Changed %@", when), width: 480)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()
        let width: CGFloat = 480 - 40
        switch item {
        case let .suggestion(suggestion):
            let why = Build.label(suggestion.explanation, font: .systemFont(ofSize: 13), lines: 0)
            why.preferredMaxLayoutWidth = width
            why.isSelectable = true
            add(why)
            add(diffSection(suggestion.diff, width: width))
            let notes = suggestion.evidence.compactMap(feedback.note)
            if !notes.isEmpty {
                let basis = SectionView(title: L("Based on"))
                basis.setRows(notes.map { note in
                    let row = FeedbackRow(note: note, in: feedback)
                    row.onClick = { [weak self] in
                        self?.dismiss(nil)
                        self?.onShowMessage?(note.chatID, note.messageID)
                    }
                    return row
                })
                add(basis)
            }
            rejectButton.title = L("Reject")
            rejectButton.bezelStyle = .rounded
            rejectButton.target = self
            rejectButton.action = #selector(reject)
            setButtons(confirm: L("Accept"), leading: rejectButton)
        case let .change(change):
            add(diffSection(change.diff, width: width))
            if change.canUndo {
                let undo = NSButton(title: L("Undo Change"), target: self, action: #selector(undo))
                undo.bezelStyle = .rounded
                setButtons(confirm: L("Done"), cancel: nil, leading: undo)
            } else {
                let note = Build.label(L("“%@” has changed since, so this can't be undone.", name), font: Theme.Font.caption, color: .secondaryLabelColor, lines: 0)
                note.preferredMaxLayoutWidth = width
                add(note)
                setButtons(confirm: L("Done"), cancel: nil)
            }
        }
    }

    private func add(_ view: NSView) {
        contentStack.addArrangedSubview(view)
        view.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true
    }

    /// The diff under what it changes: a routine's task, or a skill's instructions.
    private func diffSection(_ diff: String, width: CGFloat) -> SectionView {
        let section = SectionView(title: kind == "playbook" ? L("Skill") : L("Task"))
        section.setRows([FeedbackDiffView(diff: diff, width: width)])
        return section
    }

    override func confirmTapped() {
        guard case let .suggestion(suggestion) = item else { return dismiss(nil) }
        decide(suggestion, accept: true)
    }

    @objc private func reject() {
        guard case let .suggestion(suggestion) = item else { return }
        decide(suggestion, accept: false)
    }

    private func decide(_ suggestion: FeedbackSuggestion, accept: Bool) {
        confirmButton.isEnabled = false
        rejectButton.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                try await self.store.decide(suggestion, accept: accept, botID: self.bot.id)
                self.dismiss(nil)
            } catch {
                self.confirmButton.isEnabled = true
                self.rejectButton.isEnabled = true
                showProblem(accept ? L("Couldn't make the change") : L("Couldn't reject it"), feedbackProblem(error, name: self.name), in: self)
            }
        }
    }

    @objc private func undo(_ sender: NSButton) {
        guard case let .change(change) = item else { return }
        sender.isEnabled = false
        Task { [weak self] in
            guard let self else { return }
            do {
                try await self.store.undo(change, botID: self.bot.id)
                self.dismiss(nil)
            } catch {
                sender.isEnabled = true
                showProblem(L("Couldn't undo the change"), feedbackProblem(error, name: self.name), in: self)
            }
        }
    }
}
