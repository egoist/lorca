import AppKit

/// An email or Slack message a bot wrote, as a card the user edits in place, then sends or
/// discards. While it waits: who drafted it and the account it goes out from, the header rows as
/// Mail's compose window has them (To, Cc, Bcc, Subject), the text, the attachments, and Discard
/// apart on the left from Send. A Slack card also offers Always Send, which sends it and has the
/// bot send its next messages directly, as Always allow on a permission card allows the next
/// calls. Once sent or discarded the card keeps the message as it went, with how it ended on the
/// title's line. In a group the card sits in the bubbles' column, the bot's avatar beside its
/// bottom edge.
final class DraftCellView: TranscriptCellView, NSTextFieldDelegate, NSTextViewDelegate {
    static let identifier = NSUserInterfaceItemIdentifier("DraftCell")

    static let width: CGFloat = 440

    /// The box's height at `rowWidth`, starting at `indent`, from the same `Layout` the cell
    /// uses, so a card is exactly as tall as what it shows.
    static func height(for card: DraftCard, rowWidth: CGFloat, indent: CGFloat) -> CGFloat {
        Layout(card: card, rowWidth: rowWidth, indent: indent).height
    }

    /// For a screen reader: the title, how it ended, and the message.
    static func spokenText(card: DraftCard, botName: String) -> String {
        let fields = card.shown
        var parts = [card.title(botName: botName)]
        if let state = card.stateText { parts.append(state) }
        if !fields.to.isEmpty { parts.append(L("To %@", fields.to.joined(separator: ", "))) }
        if !fields.subject.isEmpty { parts.append(fields.subject) }
        parts.append(fields.body)
        return parts.joined(separator: ": ")
    }

    /// Under the buttons of a Slack card: what Always Send changes.
    static func footnote(for card: DraftCard) -> String? {
        card.isPending && card.direct ? L("Always Send also sends this bot's next Slack messages directly.") : nil
    }

    /// The header rows a card shows: To, then Cc and Bcc when the message has them, then an
    /// email's Subject.
    enum Row: CaseIterable {
        case to, cc, bcc, subject

        var label: String {
            switch self {
            case .to: L("To")
            case .cc: L("Cc")
            case .bcc: L("Bcc")
            case .subject: L("Subject")
            }
        }

        static func shown(for fields: DraftCard.Fields) -> [Row] {
            guard fields.isEmail else { return [.to] }
            return allCases.filter { row in
                switch row {
                case .cc: !fields.cc.isEmpty
                case .bcc: !fields.bcc.isEmpty
                default: true
                }
            }
        }
    }

    /// Where each part of a card sits, relative to the box.
    @MainActor private struct Layout {
        static let textX: CGFloat = 38
        static let titleFont = NSFont.systemFont(ofSize: 12.5, weight: .semibold)
        static let fieldFont = NSFont.systemFont(ofSize: 12)
        static let bodyFont = NSFont.systemFont(ofSize: 13)
        static let noteFont = NSFont.systemFont(ofSize: 11)
        static let rowHeight: CGFloat = 26
        static let fileHeight: CGFloat = 20
        static let controlHeight: CGFloat = 22
        /// The text grows to this many lines, then scrolls; a card that waits leaves room to write.
        static let maxBodyLines = 14
        static let minEditingLines = 3

        var width: CGFloat
        var height: CGFloat = 0
        var title = NSRect.zero
        var account = NSRect.zero
        var labelWidth: CGFloat = 0
        var rows: [(Row, NSRect)] = []
        var body = NSRect.zero
        var files: [NSRect] = []
        var note: NSRect?
        var buttonsY: CGFloat?
        var footnote: NSRect?

        init(card: DraftCard, rowWidth: CGFloat, indent: CGFloat) {
            width = min(DraftCellView.width, rowWidth - indent - ChatMetrics.horizontalInset)
            let textWidth = width - Self.textX - 14
            let fields = card.shown
            title = NSRect(x: Self.textX, y: 11, width: textWidth, height: 17)
            account = NSRect(x: Self.textX, y: 29, width: textWidth, height: 16)
            var bottom = account.maxY + 6
            let shownRows = Row.shown(for: fields)
            labelWidth = shownRows.map { Self.labelWidth($0.label) }.max() ?? 0
            for row in shownRows {
                rows.append((row, NSRect(x: Self.textX, y: bottom, width: textWidth, height: Self.rowHeight)))
                bottom += Self.rowHeight
            }
            // The text's room comes from the message as the bot wrote it, so typing never moves
            // the rows under the card; a longer message scrolls.
            let lines = Self.lines(of: card.fields.body, width: textWidth)
            let shownLines = min(Self.maxBodyLines, card.isPending ? max(Self.minEditingLines, lines) : max(1, lines))
            body = NSRect(x: Self.textX, y: bottom + 8, width: textWidth, height: Self.lineHeight * CGFloat(shownLines))
            bottom = body.maxY
            if !fields.attachments.isEmpty {
                bottom += 6
                for _ in fields.attachments {
                    files.append(NSRect(x: Self.textX, y: bottom, width: textWidth, height: Self.fileHeight))
                    bottom += Self.fileHeight
                }
            }
            if let text = card.note {
                let row = NSRect(x: Self.textX, y: bottom + 8, width: textWidth, height: Self.measure(text, font: Self.noteFont, width: textWidth, maxLines: 3))
                note = row
                bottom = row.maxY
            }
            if card.isPending {
                buttonsY = bottom + 12
                bottom += 12 + Self.controlHeight
            }
            if let text = DraftCellView.footnote(for: card) {
                let row = NSRect(x: Self.textX, y: bottom + 7, width: textWidth, height: Self.measure(text, font: Self.noteFont, width: textWidth, maxLines: 2))
                footnote = row
                bottom = row.maxY
            }
            height = bottom + 12
        }

        /// One line of the text, as the text view lays it out.
        static let lineHeight: CGFloat = {
            let manager = NSLayoutManager()
            return ceil(manager.defaultLineHeight(for: bodyFont))
        }()

        /// How many lines `text` takes at `width` in the text view.
        static func lines(of text: String, width: CGFloat) -> Int {
            let size = TextMeasure.textSize(of: NSAttributedString(string: text.isEmpty ? " " : text, attributes: [.font: bodyFont]), width: width)
            return max(1, Int((size.height / lineHeight).rounded()))
        }

        static func labelWidth(_ text: String) -> CGFloat {
            ceil((text as NSString).size(withAttributes: [.font: fieldFont]).width) + 4
        }

        /// The height a label needs for `text` at `width`, cut to `maxLines` when set.
        static func measure(_ text: String, font: NSFont, width: CGFloat, maxLines: Int = 0) -> CGFloat {
            let attributes: [NSAttributedString.Key: Any] = [.font: font]
            let full = TextMeasure.labelSize(of: NSAttributedString(string: text, attributes: attributes), width: width).height
            guard maxLines > 0 else { return full }
            let lines = Array(repeating: "X", count: maxLines).joined(separator: "\n")
            return min(full, TextMeasure.labelSize(of: NSAttributedString(string: lines, attributes: attributes), width: width).height)
        }
    }

    private let avatar = AvatarView(diameter: ChatMetrics.avatarSize)
    private let box = BackgroundView()
    private let icon = NSImageView()
    private let title = Build.label("", font: Layout.titleFont)
    private let stateLabel = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private let account = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor)
    private var labels: [Row: NSTextField] = [:]
    private var fields: [Row: NSTextField] = [:]
    private var separators: [Row: NSBox] = [:]
    private let bodyScroll = NSTextView.scrollableTextView()
    private let bodyView: NSTextView
    private var files: [AttachmentRow] = []
    private let note = Build.label("", font: Layout.noteFont, color: .secondaryLabelColor, lines: 3)
    private let discardButton = NSButton()
    private let alwaysButton = NSButton()
    private let sendButton = NSButton()
    private let footnote = Build.label("", font: Layout.noteFont, color: .secondaryLabelColor, lines: 2)
    private var groupStart = true
    private var messageID: Message.ID?
    private var card: DraftCard?
    private var botName = ""
    private var busy = false { didSet { updateControls() } }

    /// The user changed what the card shows; nothing is sent until Send.
    var onEdit: ((DraftCard.Fields) -> Void)?
    /// Sends the message as the card shows it; `always` has the bot send directly from now on.
    var onSend: ((Bool) async throws -> Void)?
    var onDiscard: (() async throws -> Void)?

    init() {
        bodyView = bodyScroll.documentView as? NSTextView ?? NSTextView()
        super.init(frame: .zero)
        box.cornerRadius = 12
        box.fillColor = Theme.botBubble
        box.borderColor = Theme.botBubbleBorder
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
        icon.contentTintColor = .controlAccentColor
        stateLabel.alignment = .right
        for view in [avatar, box, icon, title, stateLabel, account] as [NSView] {
            addSubview(view.framePositioned())
        }
        for row in Row.allCases {
            let label = Build.label(row.label, font: Layout.fieldFont, color: .secondaryLabelColor)
            let field = NSTextField()
            field.isBordered = false
            field.drawsBackground = false
            field.focusRingType = .none
            field.font = Layout.fieldFont
            field.lineBreakMode = .byTruncatingTail
            field.cell?.isScrollable = true
            field.delegate = self
            field.setAccessibilityLabel(row.label)
            // Addresses are never words to complete or correct.
            field.isAutomaticTextCompletionEnabled = row == .subject
            let separator = NSBox()
            separator.boxType = .separator
            labels[row] = label
            fields[row] = field
            separators[row] = separator
            for view in [label, field, separator] as [NSView] {
                addSubview(view.framePositioned())
            }
        }
        bodyScroll.drawsBackground = false
        bodyScroll.borderType = .noBorder
        bodyScroll.hasVerticalScroller = true
        bodyScroll.autohidesScrollers = true
        bodyView.drawsBackground = false
        bodyView.isRichText = false
        bodyView.allowsUndo = true
        bodyView.font = Layout.bodyFont
        bodyView.textColor = .labelColor
        bodyView.textContainerInset = .zero
        bodyView.textContainer?.lineFragmentPadding = 0
        bodyView.delegate = self
        bodyView.setAccessibilityLabel(L("Message"))
        addSubview(bodyScroll.framePositioned())
        note.lineBreakMode = .byWordWrapping
        footnote.lineBreakMode = .byWordWrapping
        for (button, label, action) in [(discardButton, L("Discard"), #selector(discard(_:))), (alwaysButton, L("Always Send"), #selector(send(_:))), (sendButton, L("Send"), #selector(send(_:)))] {
            button.title = label
            button.bezelStyle = .rounded
            button.controlSize = .small
            button.font = .systemFont(ofSize: 11)
            button.target = self
            button.action = action
        }
        // Send is the card's one primary action; Return in the composer stays the chat's.
        sendButton.bezelColor = .controlAccentColor
        for view in [note, discardButton, alwaysButton, sendButton, footnote] as [NSView] {
            addSubview(view.framePositioned())
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    /// `avatar` is the bot's in a group, nil in a DM.
    func configure(card: DraftCard, messageID: Message.ID, botName: String, avatar avatarContent: AvatarView.Content?, groupStart: Bool) {
        // The same card keeps what the user is typing; another one, in a reused cell, starts
        // from what it shows.
        let same = self.messageID == messageID && self.card?.version == card.version && self.card?.isPending == card.isPending
        if !same { busy = false }
        self.messageID = messageID
        self.groupStart = groupStart
        self.card = card
        self.botName = botName
        avatar.isHidden = avatarContent == nil
        if let avatarContent { avatar.content = avatarContent }
        icon.image = NSImage(systemSymbolName: card.fields.isEmail ? "envelope" : "bubble.left", accessibilityDescription: nil)
        title.stringValue = card.title(botName: botName)
        stateLabel.stringValue = card.stateText ?? ""
        stateLabel.textColor = card.needsAttention ? (card.state == "failed" ? .systemRed : .systemOrange) : .secondaryLabelColor
        account.stringValue = card.account
        let shown = card.shown
        for row in Row.allCases {
            guard let field = fields[row] else { continue }
            field.isEditable = card.isPending
            field.isSelectable = true
            let value = Self.value(of: row, in: shown)
            // A field being typed in keeps its text; setting it would move the caret.
            if !(same && field.currentEditor() != nil), field.stringValue != value { field.stringValue = value }
        }
        bodyView.isEditable = card.isPending
        if !(same && window?.firstResponder === bodyView), bodyView.string != shown.body { bodyView.string = shown.body }
        configureFiles(shown.attachments, editable: card.isPending)
        note.stringValue = card.note ?? ""
        note.textColor = card.state == "failed" ? .systemRed : (card.note == nil ? .secondaryLabelColor : .systemOrange)
        footnote.stringValue = Self.footnote(for: card) ?? ""
        updateControls()
        needsLayout = true
    }

    private static func value(of row: Row, in fields: DraftCard.Fields) -> String {
        switch row {
        case .to: fields.to.joined(separator: ", ")
        case .cc: fields.cc.joined(separator: ", ")
        case .bcc: fields.bcc.joined(separator: ", ")
        case .subject: fields.subject
        }
    }

    private func configureFiles(_ attachments: [DraftCard.Fields.File], editable: Bool) {
        while files.count < attachments.count {
            let file = AttachmentRow()
            file.onRemove = { [weak self, weak file] in
                guard let self, let file, let index = self.files.firstIndex(where: { $0 === file }) else { return }
                self.removeAttachment(at: index)
            }
            files.append(file)
            addSubview(file.framePositioned())
        }
        for (index, file) in files.enumerated() {
            file.isHidden = index >= attachments.count
            if index < attachments.count { file.configure(attachments[index], removable: editable) }
        }
    }

    /// What the card shows now, with the user's changes.
    private var edited: DraftCard.Fields? {
        guard var fields = card?.shown else { return nil }
        let list = { (row: Row) in
            (self.fields[row]?.stringValue ?? "").split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty }
        }
        fields.to = list(.to)
        if fields.isEmail {
            fields.cc = list(.cc)
            fields.bcc = list(.bcc)
            fields.subject = self.fields[.subject]?.stringValue ?? ""
        }
        fields.body = bodyView.string
        return fields
    }

    private func removeAttachment(at index: Int) {
        guard var fields = edited, fields.attachments.indices.contains(index) else { return }
        fields.attachments.remove(at: index)
        onEdit?(fields)
    }

    func controlTextDidChange(_ notification: Notification) {
        if let edited { onEdit?(edited) }
        updateControls()
    }

    func textDidChange(_ notification: Notification) {
        if let edited { onEdit?(edited) }
        updateControls()
    }

    private func updateControls() {
        let pending = card?.isPending ?? false
        discardButton.isHidden = !pending
        sendButton.isHidden = !pending
        alwaysButton.isHidden = !pending || !(card?.direct ?? false)
        let sendable = edited.map { !$0.to.isEmpty && !$0.body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } ?? false
        sendButton.isEnabled = !busy && sendable
        alwaysButton.isEnabled = !busy && sendable
        discardButton.isEnabled = !busy
        for field in fields.values { field.isEnabled = !busy }
    }

    @objc private func send(_ sender: NSButton) {
        guard let onSend, !busy else { return }
        let always = sender === alwaysButton
        if let edited { onEdit?(edited) }
        busy = true
        Task { @MainActor in
            defer { busy = false }
            do { try await onSend(always) } catch { show(error, title: L("Couldn't send this draft")) }
        }
    }

    @objc private func discard(_ sender: NSButton) {
        guard let onDiscard, !busy else { return }
        busy = true
        Task { @MainActor in
            defer { busy = false }
            do { try await onDiscard() } catch { show(error, title: L("Couldn't discard this draft")) }
        }
    }

    private func show(_ error: Error, title: String) {
        guard let window else { return }
        let alert = NSAlert()
        alert.messageText = title
        alert.informativeText = error.localizedDescription
        alert.beginSheetModal(for: window)
    }

    override func layout() {
        super.layout()
        guard let card else { return }
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let x = ChatMetrics.indent(showsAvatar: !avatar.isHidden)
        let layout = Layout(card: card, rowWidth: bounds.width, indent: x)
        func place(_ rect: NSRect) -> NSRect { rect.offsetBy(dx: x, dy: top) }

        // `layout.height` is the box alone; the row adds `top` above it.
        box.frame = NSRect(x: x, y: top, width: layout.width, height: layout.height)
        avatar.frame = NSRect(
            x: ChatMetrics.horizontalInset, y: top + layout.height - ChatMetrics.avatarSize,
            width: ChatMetrics.avatarSize, height: ChatMetrics.avatarSize)
        icon.frame = NSRect(x: x + 12, y: top + 12, width: 18, height: 18)
        var titleFrame = place(layout.title)
        if !stateLabel.stringValue.isEmpty {
            // How it ended ends the title's line.
            let width = ceil(stateLabel.cell?.cellSize.width ?? 0) + 2
            stateLabel.frame = NSRect(x: titleFrame.maxX - width, y: titleFrame.minY + 1, width: width, height: 16)
            titleFrame.size.width -= width + 8
        }
        stateLabel.isHidden = stateLabel.stringValue.isEmpty
        title.frame = titleFrame
        account.frame = place(layout.account)
        let shownRows = Set(layout.rows.map(\.0))
        for row in Row.allCases {
            let hidden = !shownRows.contains(row)
            labels[row]?.isHidden = hidden
            fields[row]?.isHidden = hidden
            separators[row]?.isHidden = hidden
        }
        for (row, rect) in layout.rows {
            let frame = place(rect)
            labels[row]?.frame = NSRect(x: frame.minX, y: frame.midY - 8, width: layout.labelWidth, height: 16)
            let fieldX = frame.minX + layout.labelWidth + 6
            fields[row]?.frame = NSRect(x: fieldX, y: frame.midY - 8, width: frame.maxX - fieldX, height: 16)
            separators[row]?.frame = NSRect(x: frame.minX + 2, y: frame.maxY - 1, width: frame.width - 2, height: 1)
        }
        // The text view draws its text at its edge, where a label insets it by 2.
        bodyScroll.frame = place(layout.body).insetBy(dx: 1, dy: 0).offsetBy(dx: 1, dy: 0)
        bodyView.frame.size.width = bodyScroll.contentSize.width
        bodyView.textContainer?.containerSize = NSSize(width: bodyScroll.contentSize.width, height: .greatestFiniteMagnitude)
        for (file, rect) in zip(files, layout.files) {
            file.frame = place(rect).insetBy(dx: 1, dy: 0).offsetBy(dx: 1, dy: 0)
            file.needsLayout = true
        }
        note.isHidden = layout.note == nil
        note.frame = layout.note.map(place) ?? .zero
        if let buttonsY = layout.buttonsY {
            let y = top + buttonsY
            let right = x + layout.width - 12
            let discardSize = discardButton.intrinsicContentSize
            discardButton.frame = NSRect(x: x + Layout.textX - 2, y: y, width: discardSize.width + 4, height: Layout.controlHeight)
            let sendSize = sendButton.intrinsicContentSize
            sendButton.frame = NSRect(x: right - sendSize.width - 4, y: y, width: sendSize.width + 4, height: Layout.controlHeight)
            let alwaysSize = alwaysButton.intrinsicContentSize
            alwaysButton.frame = NSRect(x: sendButton.frame.minX - 8 - alwaysSize.width - 4, y: y, width: alwaysSize.width + 4, height: Layout.controlHeight)
        }
        footnote.isHidden = layout.footnote == nil
        footnote.frame = layout.footnote.map(place) ?? .zero
    }
}

/// One file a draft carries: its name and size, and while the draft waits, a button that leaves
/// it out.
private final class AttachmentRow: NSView {
    var onRemove: (() -> Void)?
    private let icon = NSImageView()
    private let name = Build.label("", font: .systemFont(ofSize: 12))
    private let size = Build.label("", font: .systemFont(ofSize: 11), color: .secondaryLabelColor)
    private let remove = NSButton()

    init() {
        super.init(frame: .zero)
        icon.image = NSImage(systemSymbolName: "paperclip", accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .regular)
        icon.contentTintColor = .secondaryLabelColor
        remove.image = NSImage(systemSymbolName: "xmark.circle.fill", accessibilityDescription: L("Remove"))
        remove.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .regular)
        remove.isBordered = false
        remove.contentTintColor = .tertiaryLabelColor
        remove.target = self
        remove.action = #selector(removeFile(_:))
        for view in [icon, name, size, remove] as [NSView] {
            addSubview(view.framePositioned())
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(_ file: DraftCard.Fields.File, removable: Bool) {
        name.stringValue = file.name
        name.toolTip = file.name
        size.stringValue = file.size > 0 ? ByteCountFormatter.string(fromByteCount: file.size, countStyle: .file) : ""
        remove.isHidden = !removable
        remove.toolTip = L("Remove %@", file.name)
        remove.setAccessibilityLabel(L("Remove %@", file.name))
        needsLayout = true
    }

    @objc private func removeFile(_ sender: NSButton) { onRemove?() }

    override func layout() {
        super.layout()
        icon.frame = NSRect(x: 0, y: (bounds.height - 14) / 2, width: 14, height: 14)
        let sizeWidth = ceil(size.cell?.cellSize.width ?? 0) + 2
        let removeWidth: CGFloat = remove.isHidden ? 0 : 18
        let nameWidth = min(ceil(name.cell?.cellSize.width ?? 0) + 2, bounds.width - 20 - sizeWidth - removeWidth - 12)
        name.frame = NSRect(x: 20, y: (bounds.height - 16) / 2, width: max(0, nameWidth), height: 16)
        size.frame = NSRect(x: name.frame.maxX + 6, y: (bounds.height - 15) / 2, width: sizeWidth, height: 15)
        remove.frame = NSRect(x: size.frame.maxX + 4, y: (bounds.height - 16) / 2, width: 16, height: 16)
    }
}
