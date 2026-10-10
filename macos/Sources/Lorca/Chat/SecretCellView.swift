import AppKit

/// A bot asking for a secret: who asks and where it goes ("Developer needs a secret for
/// github.com"), why, a field for each value that hides what is typed, Save and Not now, and a
/// note that the value stays on the Runner and the bot never sees it. Save sends the values,
/// sealed to the bot's Runner by the CLI; a save that fails says why in the note's place. Once
/// answered, the card is the answer and what was asked for: "Saved · npm token". In a group the
/// card sits in the bubbles' column, the bot's avatar beside its bottom edge.
final class SecretCellView: TranscriptCellView, NSTextFieldDelegate {
    static let identifier = NSUserInterfaceItemIdentifier("SecretCell")

    static let width: CGFloat = 440

    /// The box's height at `rowWidth`, starting at `indent`. The table's row height and the
    /// cell's own layout come from the same `Layout`, so a card is exactly as tall as what it shows.
    static func height(for request: PermissionRequest, rowWidth: CGFloat, indent: CGFloat) -> CGFloat {
        Layout(request: request, rowWidth: rowWidth, indent: indent).height
    }

    /// Under the title: why, while it asks; the answer and what was asked for, once answered.
    static func caption(for request: PermissionRequest) -> String {
        request.isPending ? (request.reason ?? "") : "\(request.decisionText) · \(request.summary)"
    }

    /// For a screen reader: the title and what the card says under it.
    static func spokenText(request: PermissionRequest, botName: String) -> String {
        "\(botName) \(request.verbPhrase): \(caption(for: request))"
    }

    /// Where each part of a card sits, relative to the box.
    @MainActor private struct Layout {
        static let textX: CGFloat = 38
        static let titleFont = NSFont.systemFont(ofSize: 12.5, weight: .semibold)
        static let captionFont = NSFont.systemFont(ofSize: 11.5)
        static let noteFont = NSFont.systemFont(ofSize: 11)
        static let controlHeight: CGFloat = 22
        static let fieldGap: CGFloat = 6

        var width: CGFloat
        var height: CGFloat = 0
        var caption = NSRect.zero
        var fieldsY: CGFloat?
        var buttonsY: CGFloat?
        var note: NSRect?

        init(request: PermissionRequest, rowWidth: CGFloat, indent: CGFloat) {
            width = min(SecretCellView.width, rowWidth - indent - ChatMetrics.horizontalInset)
            let textWidth = width - Self.textX - 12
            let text = SecretCellView.caption(for: request)
            caption = NSRect(x: Self.textX, y: 30, width: textWidth, height: text.isEmpty ? 0 : Self.measure(text, font: Self.captionFont, width: textWidth, maxLines: 4))
            var bottom = caption.maxY
            if request.isPending {
                let count = CGFloat(request.secret?.fields.count ?? 0)
                fieldsY = bottom + 9
                bottom += 9 + count * Self.controlHeight + max(0, count - 1) * Self.fieldGap
                buttonsY = bottom + 9
                bottom += 9 + Self.controlHeight
                // One line, which a save that fails takes over to say why, so the transcript never moves.
                note = NSRect(x: Self.textX, y: bottom + 7, width: textWidth, height: Self.measure("X", font: Self.noteFont, width: textWidth))
                bottom = note!.maxY
            }
            height = bottom + 12
        }

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
    private let caption = Build.label("", font: Layout.captionFont, color: .secondaryLabelColor, lines: 4)
    private var fields: [NSSecureTextField] = []
    private let saveButton = NSButton()
    private let notNowButton = NSButton()
    /// Where the value stays, or why a save did not go through.
    private let note = Build.label("", font: Layout.noteFont, color: .secondaryLabelColor, lines: 1)
    private var groupStart = true
    private var messageID: Message.ID?
    private var request: PermissionRequest?
    private var keptNote = ""
    private var saveError: String? { didSet { updateNote() } }
    private var busy = false { didSet { updateControls() } }

    /// Sends the values, by field name; throws why they could not go.
    var onSave: (([String: String]) async throws -> Void)?
    /// Not now.
    var onDecline: (() -> Void)?

    init() {
        super.init(frame: .zero)
        box.cornerRadius = 12
        box.fillColor = Theme.botBubble
        box.borderColor = Theme.botBubbleBorder
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
        icon.image = NSImage(systemSymbolName: "key", accessibilityDescription: nil)
        caption.lineBreakMode = .byWordWrapping
        for (button, label, action) in [(saveButton, L("Save"), #selector(save(_:))), (notNowButton, L("Not now"), #selector(decline(_:)))] {
            button.title = label
            button.bezelStyle = .rounded
            button.controlSize = .small
            button.font = .systemFont(ofSize: 11)
            button.target = self
            button.action = action
        }
        note.lineBreakMode = .byTruncatingTail
        for view in [avatar, box, icon, title, caption, saveButton, notNowButton, note] as [NSView] {
            addSubview(view.framePositioned())
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    /// `avatar` is the bot's in a group, nil in a DM. `runnerName` is where the values stay.
    func configure(request: PermissionRequest, messageID: Message.ID, botName: String, runnerName: String, avatar avatarContent: AvatarView.Content?, groupStart: Bool) {
        // Another card, in a reused cell, starts empty; the same one keeps what is being typed.
        if self.messageID != messageID || !request.isPending {
            fields.forEach { $0.removeFromSuperview() }
            fields = []
            busy = false
            saveError = nil
        }
        self.messageID = messageID
        self.request = request
        self.groupStart = groupStart
        avatar.isHidden = avatarContent == nil
        if let avatarContent { avatar.content = avatarContent }
        icon.contentTintColor = request.isPending ? .controlAccentColor : .secondaryLabelColor
        title.stringValue = "\(botName) \(request.verbPhrase)"
        title.toolTip = title.stringValue
        caption.stringValue = Self.caption(for: request)
        let asked = request.isPending ? request.secret?.fields ?? [] : []
        if fields.count != asked.count {
            fields.forEach { $0.removeFromSuperview() }
            fields = asked.map { _ in
                let field = NSSecureTextField()
                field.bezelStyle = .roundedBezel
                field.controlSize = .small
                field.font = .systemFont(ofSize: 12)
                field.delegate = self
                field.target = self
                field.action = #selector(submit(_:))
                // A secret is never a word to complete.
                field.isAutomaticTextCompletionEnabled = false
                addSubview(field.framePositioned())
                return field
            }
        }
        for (field, asked) in zip(fields, asked) {
            field.placeholderString = asked.label
            field.setAccessibilityLabel(asked.label)
        }
        keptNote = L("Saved on %@. %@ never sees it.", runnerName, botName)
        updateNote()
        updateControls()
        needsLayout = true
    }

    private func updateControls() {
        let pending = request?.isPending ?? false
        saveButton.isHidden = !pending
        notNowButton.isHidden = !pending
        let filled = !fields.isEmpty && fields.allSatisfy { !$0.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
        saveButton.isEnabled = filled && !busy
        notNowButton.isEnabled = !busy
        fields.forEach { $0.isEnabled = !busy }
    }

    private func updateNote() {
        note.stringValue = saveError ?? keptNote
        note.textColor = saveError == nil ? .secondaryLabelColor : .systemRed
        note.toolTip = saveError
    }

    func controlTextDidChange(_ notification: Notification) {
        if saveError != nil { saveError = nil }
        updateControls()
    }

    /// Return in a field: on to the next empty one, or Save once every one has a value.
    @objc private func submit(_ sender: NSSecureTextField) {
        if let empty = fields.first(where: { $0.stringValue.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }) {
            window?.makeFirstResponder(empty)
            return
        }
        save(sender)
    }

    @objc private func save(_ sender: Any?) {
        guard let onSave, let request, let asked = request.secret?.fields, saveButton.isEnabled else { return }
        let values = Dictionary(uniqueKeysWithValues: zip(asked.map(\.name), fields.map(\.stringValue)))
        busy = true
        saveError = nil
        Task { @MainActor in
            defer { busy = false }
            do {
                try await onSave(values)
            } catch {
                saveError = error.localizedDescription
            }
        }
    }

    @objc private func decline(_ sender: Any?) {
        guard !busy else { return }
        onDecline?()
    }

    override func layout() {
        super.layout()
        guard let request else { return }
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let x = ChatMetrics.indent(showsAvatar: !avatar.isHidden)
        let layout = Layout(request: request, rowWidth: bounds.width, indent: x)
        func place(_ rect: NSRect) -> NSRect { rect.offsetBy(dx: x, dy: top) }

        // `layout.height` is the box alone; the row adds `top` above it.
        box.frame = NSRect(x: x, y: top, width: layout.width, height: layout.height)
        avatar.frame = NSRect(
            x: ChatMetrics.horizontalInset, y: top + layout.height - ChatMetrics.avatarSize,
            width: ChatMetrics.avatarSize, height: ChatMetrics.avatarSize)
        icon.frame = NSRect(x: x + 12, y: top + 12, width: 18, height: 18)
        title.frame = place(NSRect(x: Layout.textX, y: 11, width: layout.width - Layout.textX - 12, height: 17))
        caption.frame = place(layout.caption)
        caption.isHidden = layout.caption.height == 0
        var y = top + (layout.fieldsY ?? 0)
        for field in fields {
            field.frame = NSRect(x: x + Layout.textX, y: y, width: layout.width - Layout.textX - 12, height: Layout.controlHeight)
            y += Layout.controlHeight + Layout.fieldGap
        }
        if let buttonsY = layout.buttonsY {
            var buttonX = x + Layout.textX - 2
            for button in [saveButton, notNowButton] {
                let size = button.intrinsicContentSize
                button.frame = NSRect(x: buttonX, y: top + buttonsY, width: size.width + 4, height: Layout.controlHeight)
                buttonX += size.width + 12
            }
        }
        note.isHidden = layout.note == nil
        note.frame = layout.note.map(place) ?? .zero
    }
}
