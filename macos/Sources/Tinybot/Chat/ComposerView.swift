import AppKit

final class ComposerTextView: NSTextView {
    var onKeyCommand: ((Selector) -> Bool)?
    var placeholder: String = "" {
        didSet { needsDisplay = true }
    }

    override func doCommand(by selector: Selector) {
        if onKeyCommand?(selector) == true { return }
        super.doCommand(by: selector)
    }

    override func draw(_ dirtyRect: NSRect) {
        super.draw(dirtyRect)
        guard string.isEmpty, !placeholder.isEmpty else { return }
        let attributes: [NSAttributedString.Key: Any] = [
            .font: font ?? Theme.Font.message,
            .foregroundColor: NSColor.placeholderTextColor,
        ]
        let origin = NSPoint(
            x: textContainerInset.width + (textContainer?.lineFragmentPadding ?? 0),
            y: textContainerInset.height
        )
        placeholder.draw(at: origin, withAttributes: attributes)
    }
}

/// Round symbol button for the composer: a solid disc for the primary action,
/// a quiet disc for secondary ones, or bare glyph that only fills under the pointer.
final class ComposerButton: NSButton {
    enum Style { case primary, secondary, plain }

    var style: Style = .plain {
        didSet { needsDisplay = true }
    }

    private let diameter: CGFloat = 28
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    init(symbol: String, pointSize: CGFloat, weight: NSFont.Weight, tooltip: String, target: AnyObject?, action: Selector) {
        super.init(frame: .zero)
        image = NSImage(systemSymbolName: symbol, accessibilityDescription: tooltip)
        symbolConfiguration = NSImage.SymbolConfiguration(pointSize: pointSize, weight: weight)
        imagePosition = .imageOnly
        isBordered = false
        toolTip = tooltip
        self.target = target
        self.action = action
        translatesAutoresizingMaskIntoConstraints = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize { NSSize(width: diameter, height: diameter) }
    override var alignmentRectInsets: NSEdgeInsets { NSEdgeInsets() }

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

    override func draw(_ dirtyRect: NSRect) {
        let fill: NSColor?
        switch style {
        case .primary:
            fill = Theme.composerPrimary.withAlphaComponent(isHighlighted ? 0.8 : 1)
            contentTintColor = Theme.composerPrimaryContent
        case .secondary:
            fill = Theme.composerControl.withAlphaComponent(isHighlighted || isHovered ? 0.18 : 0.1)
            contentTintColor = .labelColor
        case .plain:
            fill = isHighlighted || isHovered ? Theme.composerControl : nil
            contentTintColor = .secondaryLabelColor
        }
        if let fill {
            fill.setFill()
            NSBezierPath(ovalIn: bounds).fill()
        }
        super.draw(dirtyRect)
    }
}

final class ComposerView: NSView {
    private let field = BackgroundView()
    private let scrollView = NSScrollView()
    private let textView: ComposerTextView
    private let attachButton: ComposerButton
    private let voiceButton: ComposerButton
    private let sendButton: ComposerButton
    private let stopButton: ComposerButton
    private let trailing: NSStackView
    private let mentions = MentionPanel()

    /// Single line: controls sit beside the text in a pill.
    /// Expanded: text spans the field with the controls in a row underneath.
    private enum Mode { case compact, expanded }
    private var mode: Mode = .compact
    private var compactConstraints: [NSLayoutConstraint] = []
    private var expandedConstraints: [NSLayoutConstraint] = []
    private var heightConstraint: NSLayoutConstraint!

    private let controlSize: CGFloat = 28
    private let controlInset: CGFloat = 8
    private let controlSpacing: CGFloat = 4
    private let textInset: CGFloat = 8
    private let expandedTextInset: CGFloat = 12
    private let maxTextHeight: CGFloat = 168
    private var lineHeight: CGFloat {
        ceil(textView.layoutManager?.defaultLineHeight(for: Theme.Font.message) ?? 17)
    }
    private var minTextHeight: CGFloat { lineHeight + textView.textContainerInset.height * 2 }

    var onSend: ((String) -> Void)?
    var onStop: (() -> Void)?
    var onAttach: (() -> Void)?
    var onVoice: (() -> Void)?
    var mentionableBots: [Bot] = []

    var isResponding = false {
        didSet {
            updateButtons()
            updateLayout()
        }
    }

    var text: String {
        get { textView.string }
        set {
            textView.string = newValue
            handleTextChange()
        }
    }

    init() {
        // An explicit TextKit 1 stack keeps `layoutManager` available for height tracking.
        let storage = NSTextStorage()
        let layoutManager = NSLayoutManager()
        storage.addLayoutManager(layoutManager)
        let container = NSTextContainer(size: NSSize(width: 0, height: CGFloat.greatestFiniteMagnitude))
        container.widthTracksTextView = true
        layoutManager.addTextContainer(container)
        textView = ComposerTextView(frame: .zero, textContainer: container)

        attachButton = ComposerButton(
            symbol: "plus", pointSize: 14, weight: .medium, tooltip: "Attach",
            target: nil, action: #selector(attach))
        voiceButton = ComposerButton(
            symbol: "mic.fill", pointSize: 13, weight: .medium, tooltip: "Dictate",
            target: nil, action: #selector(voice))
        sendButton = ComposerButton(
            symbol: "arrow.up", pointSize: 13, weight: .bold, tooltip: "Send",
            target: nil, action: #selector(send))
        stopButton = ComposerButton(
            symbol: "stop.fill", pointSize: 11, weight: .bold, tooltip: "Stop responding (⌘.)",
            target: nil, action: #selector(stop))
        trailing = Build.stack([voiceButton, sendButton, stopButton], orientation: .horizontal, spacing: 4)

        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true

        configureTextView()
        for button in [attachButton, voiceButton, sendButton, stopButton] {
            button.target = self
        }
        attachButton.style = .secondary
        sendButton.style = .primary
        stopButton.style = .primary
        trailing.spacing = controlSpacing

        field.cornerRadius = (controlSize + controlInset * 2) / 2
        field.fillColor = Theme.composerField
        field.borderColor = Theme.composerBorder

        scrollView.documentView = textView
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = false
        scrollView.verticalScrollElasticity = .none
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        addSubview(field)
        field.addSubview(scrollView)
        field.addSubview(attachButton)
        field.addSubview(trailing)

        heightConstraint = scrollView.heightAnchor.constraint(equalToConstant: minTextHeight)

        NSLayoutConstraint.activate([
            field.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 20),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -20),
            field.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -14),

            attachButton.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: controlInset),
            attachButton.widthAnchor.constraint(equalToConstant: controlSize),
            attachButton.heightAnchor.constraint(equalToConstant: controlSize),

            trailing.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -controlInset),
            trailing.heightAnchor.constraint(equalToConstant: controlSize),
            trailing.centerYAnchor.constraint(equalTo: attachButton.centerYAnchor),

            heightConstraint,
        ])

        compactConstraints = [
            attachButton.centerYAnchor.constraint(equalTo: field.centerYAnchor),
            attachButton.topAnchor.constraint(equalTo: field.topAnchor, constant: controlInset),
            scrollView.leadingAnchor.constraint(equalTo: attachButton.trailingAnchor, constant: textInset),
            scrollView.trailingAnchor.constraint(equalTo: trailing.leadingAnchor, constant: -textInset),
            scrollView.centerYAnchor.constraint(equalTo: field.centerYAnchor),
        ]

        expandedConstraints = [
            scrollView.topAnchor.constraint(equalTo: field.topAnchor, constant: expandedTextInset),
            scrollView.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: expandedTextInset),
            scrollView.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -expandedTextInset),
            attachButton.topAnchor.constraint(equalTo: scrollView.bottomAnchor, constant: 6),
            attachButton.bottomAnchor.constraint(equalTo: field.bottomAnchor, constant: -controlInset),
        ]

        NSLayoutConstraint.activate(compactConstraints)

        mentions.onPick = { [weak self] bot in self?.insertMention(bot) }
        updateButtons()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private func configureTextView() {
        textView.delegate = self
        textView.font = Theme.Font.message
        textView.textColor = .labelColor
        textView.drawsBackground = false
        textView.isRichText = false
        textView.isEditable = true
        textView.isSelectable = true
        textView.allowsUndo = true
        textView.isVerticallyResizable = true
        textView.isHorizontallyResizable = false
        textView.autoresizingMask = [.width]
        textView.textContainerInset = NSSize(width: 2, height: 4)
        textView.insertionPointColor = .controlAccentColor
        textView.onKeyCommand = { [weak self] selector in
            self?.handle(selector) ?? false
        }
    }

    // MARK: - Focus

    func focus() {
        window?.makeFirstResponder(textView)
    }

    func configure(placeholder: String, bots: [Bot]) {
        textView.placeholder = placeholder
        mentionableBots = bots
        updateButtons()
    }

    // MARK: - Actions

    @objc private func send() {
        let value = textView.string.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !value.isEmpty else { return }
        textView.string = ""
        mentions.dismiss()
        handleTextChange()
        onSend?(value)
    }

    @objc private func stop() {
        onStop?()
    }

    @objc private func attach() {
        onAttach?()
    }

    @objc private func voice() {
        onVoice?()
    }

    private func handle(_ selector: Selector) -> Bool {
        if mentions.isVisible {
            switch selector {
            case #selector(NSResponder.moveUp(_:)):
                mentions.moveSelection(by: -1)
                return true
            case #selector(NSResponder.moveDown(_:)):
                mentions.moveSelection(by: 1)
                return true
            case #selector(NSResponder.insertTab(_:)), #selector(NSResponder.insertNewline(_:)):
                mentions.commitSelection()
                return true
            case #selector(NSResponder.cancelOperation(_:)):
                mentions.dismiss()
                return true
            default:
                break
            }
        }

        guard selector == #selector(NSResponder.insertNewline(_:)) else { return false }

        // Shift-Return always breaks the line; plain Return sends unless the
        // preference reserves sending for ⌘Return.
        let shift = NSApp.currentEvent?.modifierFlags.contains(.shift) ?? false
        if Preferences.sendOnReturn {
            if shift { return false }
            send()
            return true
        }
        let command = NSApp.currentEvent?.modifierFlags.contains(.command) ?? false
        if command {
            send()
            return true
        }
        return false
    }

    // MARK: - Layout

    override func layout() {
        super.layout()
        // Width changes can wrap a line that fit, or fit one that wrapped.
        updateLayout()
    }

    /// Text width available beside the controls in the pill.
    private var compactTextWidth: CGFloat {
        let visible = CGFloat(trailing.arrangedSubviews.filter { !$0.isHidden }.count)
        let trailingWidth = visible * controlSize + max(0, visible - 1) * controlSpacing
        let chrome = controlInset * 2 + controlSize + textInset * 2 + trailingWidth
        let containerPadding = (textView.textContainer?.lineFragmentPadding ?? 5) * 2
        return field.bounds.width - chrome - textView.textContainerInset.width * 2 - containerPadding
    }

    private func updateLayout() {
        guard let layoutManager = textView.layoutManager, let container = textView.textContainer
        else { return }
        layoutManager.ensureLayout(for: container)
        let used = layoutManager.usedRect(for: container)

        let wanted: Mode
        if textView.string.contains("\n") || used.height > lineHeight * 1.5 {
            wanted = .expanded
        } else if mode == .expanded, used.width > compactTextWidth - 12 {
            // One line in the wide layout that would wrap beside the controls.
            wanted = .expanded
        } else {
            wanted = .compact
        }

        if wanted != mode {
            mode = wanted
            NSLayoutConstraint.deactivate(wanted == .compact ? expandedConstraints : compactConstraints)
            NSLayoutConstraint.activate(wanted == .compact ? compactConstraints : expandedConstraints)
            // Re-measure at the new width before sizing the text.
            layoutSubtreeIfNeeded()
            layoutManager.ensureLayout(for: container)
        }

        let textHeight = layoutManager.usedRect(for: container).height + textView.textContainerInset.height * 2
        let clamped = min(maxTextHeight, max(minTextHeight, ceil(textHeight)))
        if abs(clamped - heightConstraint.constant) > 0.5 {
            heightConstraint.constant = clamped
            scrollView.hasVerticalScroller = clamped >= maxTextHeight
        }
    }

    private func updateButtons() {
        let hasText = !textView.string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        sendButton.isHidden = isResponding || !hasText
        stopButton.isHidden = !isResponding
        // Dictation is the primary action only while nothing else is.
        voiceButton.style = sendButton.isHidden && stopButton.isHidden ? .primary : .plain
        let mentionHint = mentionableBots.count > 1 ? " · @ to mention" : ""
        sendButton.toolTip =
            Preferences.sendOnReturn
            ? "Send (Return) · Shift-Return for a new line\(mentionHint)"
            : "Send (⌘Return)\(mentionHint)"
    }

    // MARK: - Mentions

    /// Range of the `@token` the caret currently sits in, if any.
    private func mentionRange() -> NSRange? {
        let text = textView.string as NSString
        let caret = textView.selectedRange().location
        guard caret <= text.length, caret > 0 else { return nil }

        var index = caret - 1
        while index >= 0 {
            let character = text.character(at: index)
            let scalar = UnicodeScalar(character).map(Character.init)
            if scalar == "@" {
                let isStart = index == 0 || isBoundary(text.character(at: index - 1))
                guard isStart else { return nil }
                return NSRange(location: index, length: caret - index)
            }
            if let scalar, scalar == " " || scalar == "\n" { return nil }
            if caret - index > 24 { return nil }
            index -= 1
        }
        return nil
    }

    private func isBoundary(_ character: unichar) -> Bool {
        guard let scalar = UnicodeScalar(character) else { return true }
        return CharacterSet.whitespacesAndNewlines.contains(scalar)
    }

    private func updateMentions() {
        // Any chat with a bot completes `@`; a direct chat just offers its one bot.
        guard !mentionableBots.isEmpty, let range = mentionRange() else {
            mentions.dismiss()
            return
        }

        let text = textView.string as NSString
        let query = text.substring(with: range).dropFirst().lowercased()
        let matches = mentionableBots.filter {
            query.isEmpty || $0.name.lowercased().hasPrefix(query)
        }

        guard !matches.isEmpty else {
            mentions.dismiss()
            return
        }

        let caretRect = textView.firstRect(forCharacterRange: range, actualRange: nil)
        mentions.show(bots: matches, near: caretRect, relativeTo: textView)
    }

    private func insertMention(_ bot: Bot) {
        guard let range = mentionRange() else { return }
        textView.insertText("@\(bot.name) ", replacementRange: range)
        mentions.dismiss()
        handleTextChange()
    }

    /// Tints `@Name` runs so mentions read as addressed, not as typed punctuation.
    private func highlightMentions() {
        guard let storage = textView.textStorage else { return }
        let full = NSRange(location: 0, length: storage.length)
        storage.removeAttribute(.foregroundColor, range: full)
        storage.addAttribute(.foregroundColor, value: NSColor.labelColor, range: full)
        storage.addAttribute(.font, value: Theme.Font.message, range: full)

        let text = storage.string as NSString
        for bot in mentionableBots {
            var searchRange = NSRange(location: 0, length: text.length)
            while searchRange.location < text.length {
                let found = text.range(
                    of: "@\(bot.name)", options: [.caseInsensitive], range: searchRange)
                guard found.location != NSNotFound else { break }
                storage.addAttribute(.foregroundColor, value: bot.accent.color, range: found)
                storage.addAttribute(.font, value: Theme.Font.messageBold, range: found)
                let next = found.location + found.length
                searchRange = NSRange(location: next, length: max(0, text.length - next))
            }
        }
    }
}

extension ComposerView: NSTextViewDelegate {
    func textDidChange(_ notification: Notification) {
        handleTextChange()
    }

    private func handleTextChange() {
        highlightMentions()
        updateButtons()
        updateLayout()
        updateMentions()
        textView.needsDisplay = true
    }

    func textViewDidChangeSelection(_ notification: Notification) {
        updateMentions()
    }
}
