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

final class ComposerView: NSView {
    private let field = BackgroundView()
    private let scrollView = NSScrollView()
    private let textView: ComposerTextView
    private let sendButton = NSButton()
    private let stopButton = NSButton()
    private let hint = Build.label("", font: Theme.Font.caption, color: .tertiaryLabelColor)
    private let mentions = MentionPanel()

    private var heightConstraint: NSLayoutConstraint!
    private let minHeight: CGFloat = 34
    private let maxHeight: CGFloat = 168

    var onSend: ((String) -> Void)?
    var onStop: (() -> Void)?
    var mentionableBots: [Bot] = []

    var isResponding = false {
        didSet { updateButtons() }
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

        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true

        configureTextView()
        configureButtons()

        field.cornerRadius = 10
        field.fillColor = Theme.composerField
        field.borderColor = .separatorColor

        scrollView.documentView = textView
        scrollView.drawsBackground = false
        scrollView.hasVerticalScroller = false
        scrollView.verticalScrollElasticity = .none
        scrollView.translatesAutoresizingMaskIntoConstraints = false

        let divider = HairlineView()

        addSubview(divider)
        addSubview(field)
        field.addSubview(scrollView)
        field.addSubview(sendButton)
        field.addSubview(stopButton)
        addSubview(hint)

        heightConstraint = scrollView.heightAnchor.constraint(equalToConstant: minHeight)

        NSLayoutConstraint.activate([
            divider.topAnchor.constraint(equalTo: topAnchor),
            divider.leadingAnchor.constraint(equalTo: leadingAnchor),
            divider.trailingAnchor.constraint(equalTo: trailingAnchor),

            field.topAnchor.constraint(equalTo: topAnchor, constant: 12),
            field.leadingAnchor.constraint(equalTo: leadingAnchor, constant: 20),
            field.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -20),

            scrollView.topAnchor.constraint(equalTo: field.topAnchor, constant: 6),
            scrollView.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: 6),
            scrollView.trailingAnchor.constraint(equalTo: sendButton.leadingAnchor, constant: -6),
            scrollView.bottomAnchor.constraint(equalTo: field.bottomAnchor, constant: -6),
            heightConstraint,

            sendButton.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -6),
            sendButton.bottomAnchor.constraint(equalTo: field.bottomAnchor, constant: -6),
            sendButton.widthAnchor.constraint(equalToConstant: 26),
            sendButton.heightAnchor.constraint(equalToConstant: 26),

            stopButton.trailingAnchor.constraint(equalTo: sendButton.trailingAnchor),
            stopButton.bottomAnchor.constraint(equalTo: sendButton.bottomAnchor),
            stopButton.widthAnchor.constraint(equalToConstant: 26),
            stopButton.heightAnchor.constraint(equalToConstant: 26),

            hint.topAnchor.constraint(equalTo: field.bottomAnchor, constant: 6),
            hint.leadingAnchor.constraint(equalTo: field.leadingAnchor, constant: 4),
            hint.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -8),
        ])

        mentions.onPick = { [weak self] bot in self?.insertMention(bot) }
        updateButtons()
        updateHint()
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

    private func configureButtons() {
        sendButton.image = NSImage(systemSymbolName: "arrow.up", accessibilityDescription: "Send")
        sendButton.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 12, weight: .bold)
        sendButton.bezelStyle = .circular
        sendButton.isBordered = true
        sendButton.contentTintColor = .white
        sendButton.bezelColor = .controlAccentColor
        sendButton.target = self
        sendButton.action = #selector(send)
        sendButton.toolTip = "Send (Return)"
        sendButton.translatesAutoresizingMaskIntoConstraints = false

        stopButton.image = NSImage(systemSymbolName: "stop.fill", accessibilityDescription: "Stop")
        stopButton.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 10, weight: .bold)
        stopButton.bezelStyle = .circular
        stopButton.isBordered = true
        stopButton.target = self
        stopButton.action = #selector(stop)
        stopButton.toolTip = "Stop responding (⌘.)"
        stopButton.isHidden = true
        stopButton.translatesAutoresizingMaskIntoConstraints = false
    }

    // MARK: - Focus

    func focus() {
        window?.makeFirstResponder(textView)
    }

    func configure(placeholder: String, bots: [Bot]) {
        textView.placeholder = placeholder
        mentionableBots = bots
        updateHint()
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

    private func updateHeight() {
        guard let layoutManager = textView.layoutManager, let container = textView.textContainer
        else { return }
        layoutManager.ensureLayout(for: container)
        let used = layoutManager.usedRect(for: container).height + textView.textContainerInset.height * 2
        let clamped = min(maxHeight, max(minHeight, ceil(used)))
        guard abs(clamped - heightConstraint.constant) > 0.5 else { return }
        heightConstraint.constant = clamped
        scrollView.hasVerticalScroller = clamped >= maxHeight
    }

    private func updateButtons() {
        let hasText = !textView.string.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        sendButton.isEnabled = hasText && !isResponding
        sendButton.isHidden = isResponding
        sendButton.alphaValue = hasText ? 1 : 0.45
        stopButton.isHidden = !isResponding
    }

    private func updateHint() {
        let mentionHint = mentionableBots.count > 1 ? " · @ to mention" : ""
        hint.stringValue =
            Preferences.sendOnReturn
            ? "Return to send · Shift-Return for a new line\(mentionHint)"
            : "⌘Return to send\(mentionHint)"
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
        guard mentionableBots.count > 1, let range = mentionRange() else {
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
        updateHeight()
        updateButtons()
        updateMentions()
        textView.needsDisplay = true
    }

    func textViewDidChangeSelection(_ notification: Notification) {
        updateMentions()
    }
}
