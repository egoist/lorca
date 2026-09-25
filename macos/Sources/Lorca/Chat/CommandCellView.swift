import AppKit

/// A shell command's card, while the command needs the user (`ToolInvocation.isShown`). While
/// Auto-review asks to run it: who wants to, the command on one line in a code block that shows
/// the whole command on click, why, and Allow once, Always allow (with the rule it adds), and
/// Deny. While the command runs on after its call: Stop at the end of the title's line, the
/// command, and its last lines in a code block of their own that scrolls; waiting for input, a
/// field to answer in, which hides what is typed unless the question is a yes or no, with Send.
/// What the user types goes to the command and nowhere else: the CLI writes it to the terminal
/// and keeps nothing. In a group the card sits in the bubbles' column, the bot's avatar beside
/// its bottom edge.
final class CommandCellView: TranscriptCellView {
    static let identifier = NSUserInterfaceItemIdentifier("CommandCell")

    static let width: CGFloat = 440

    /// The box's height at `rowWidth`, starting at `indent`. The table's row height and the
    /// cell's own layout come from the same `Layout`, so a card is exactly as tall as what it shows.
    static func height(for run: CommandRun, rowWidth: CGFloat, indent: CGFloat) -> CGFloat {
        Layout(run: run, rowWidth: rowWidth, indent: indent).height
    }

    /// "Chef wants to run a command on Workbench", "Chef's command is running", or "Chef's
    /// command is waiting for input".
    static func title(for run: CommandRun, botName: String) -> String {
        switch run.state {
        case .waiting: L("%@'s command is waiting for input", botName)
        case .running: L("%@'s command is running", botName)
        // Asking: no other state shows a card.
        default: run.device.map { "\(botName) \(L("wants to run a command on %@", $0))" } ?? L("%@'s command", botName)
        }
    }

    /// For a screen reader: the title, the command, and what the card says under it.
    static func spokenText(run: CommandRun, botName: String) -> String {
        let under = run.isLive ? outputText(run) : caption(run)
        return [title(for: run, botName: botName), "$ \(run.firstLine)", under].compactMap { $0 }.joined(separator: ": ")
    }

    /// Under the command while Auto-review asks: why.
    private static func caption(_ run: CommandRun) -> String? {
        run.state == .asking ? run.reason : nil
    }

    /// At the foot of the card: the rule Always allow adds, or where an answer goes.
    private static func note(_ run: CommandRun) -> String? {
        switch run.state {
        case .asking: run.rule.map { L("Always allow adds the rule “%@”.", $0) }
        case .waiting where run.sessionID != nil: L("What you type goes straight to the command, not into the chat.")
        default: nil
        }
    }

    /// The terminal's last lines with something on them.
    private static func outputText(_ run: CommandRun) -> String? {
        let lines = (run.output ?? "").split(separator: "\n", omittingEmptySubsequences: true)
        return lines.isEmpty ? nil : lines.joined(separator: "\n")
    }

    /// How many characters of the monospaced `font` one line of a label `width` wide holds, less
    /// the label's own inset.
    private static func perLine(width: CGFloat, font: NSFont) -> Int {
        max(8, Int((width - 8) / ("M" as NSString).size(withAttributes: [.font: font]).width))
    }

    /// "$ " and the command's first line, cut to one line of `width`.
    private static func commandLine(_ run: CommandRun, fitting width: CGFloat, font: NSFont) -> String {
        let fits = perLine(width: width, font: font)
        let command = "$ \(run.firstLine)"
        guard command.count > fits else { return command }
        return String(command.prefix(fits - 1)).trimmingCharacters(in: .whitespaces) + "…"
    }

    /// Where each part of a card sits, relative to the box.
    @MainActor private struct Layout {
        static let textX: CGFloat = 38
        static let titleFont = NSFont.systemFont(ofSize: 12.5, weight: .semibold)
        static let screenFont = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        static let captionFont = NSFont.systemFont(ofSize: 11.5)
        static let noteFont = NSFont.systemFont(ofSize: 11)
        /// The output block grows to this many lines, then scrolls.
        static let outputLines = 6
        static let screenPadX: CGFloat = 8
        static let screenPadY: CGFloat = 5
        static let controlHeight: CGFloat = 22
        /// `outputLines` lines as the output's text view lays them out.
        static let outputMaxHeight = TextMeasure.textSize(
            of: NSAttributedString(string: Array(repeating: "X", count: outputLines).joined(separator: "\n"), attributes: [.font: screenFont]),
            width: 1000
        ).height

        var width: CGFloat
        var height: CGFloat = 0
        var title = NSRect.zero
        var command: NSRect?
        var commandText = ""
        var caption: NSRect?
        var output: NSRect?
        var buttonsY: CGFloat?
        var answerY: CGFloat?
        var note: NSRect?

        init(run: CommandRun, rowWidth: CGFloat, indent: CGFloat) {
            width = min(CommandCellView.width, rowWidth - indent - ChatMetrics.horizontalInset)
            let textWidth = width - Self.textX - 12
            // The title's width leaves room for Stop, which `layout()` places at its end.
            title = NSRect(x: Self.textX, y: 11, width: textWidth, height: 17)
            // The command's text and the output's share a column inside their blocks.
            let columnWidth = textWidth - Self.screenPadX * 2
            commandText = CommandCellView.commandLine(run, fitting: columnWidth, font: Self.screenFont)
            let block = NSRect(x: Self.textX, y: 36, width: textWidth, height: Self.measure("X", font: Self.screenFont, width: columnWidth) + Self.screenPadY * 2)
            command = block
            var bottom = block.maxY
            if let text = CommandCellView.caption(run) {
                let row = NSRect(x: Self.textX, y: bottom + 6, width: textWidth, height: Self.measure(text, font: Self.captionFont, width: textWidth, maxLines: 4))
                caption = row
                bottom = row.maxY
            }
            if run.isLive, let text = CommandCellView.outputText(run) {
                let full = TextMeasure.textSize(of: NSAttributedString(string: text, attributes: [.font: Self.screenFont]), width: columnWidth).height
                let lines = NSRect(x: Self.textX, y: bottom + 6, width: textWidth, height: min(full, Self.outputMaxHeight) + Self.screenPadY * 2)
                output = lines
                bottom = lines.maxY
            }
            if run.state == .asking {
                buttonsY = bottom + 9
                bottom += 9 + Self.controlHeight
            }
            if run.state == .waiting, run.sessionID != nil {
                answerY = bottom + 9
                bottom += 9 + Self.controlHeight
            }
            if let text = CommandCellView.note(run) {
                let row = NSRect(x: Self.textX, y: bottom + 7, width: textWidth, height: Self.measure(text, font: Self.noteFont, width: textWidth, maxLines: 2))
                note = row
                bottom = row.maxY
            }
            height = bottom + 12
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
    private let command = CommandBlockView(font: Layout.screenFont, lines: 1, padding: NSSize(width: Layout.screenPadX, height: Layout.screenPadY))
    private let caption = Build.label("", font: Layout.captionFont, color: .secondaryLabelColor, lines: 4)
    /// The output: a code block like the command's that grows to `Layout.outputLines` lines and
    /// then scrolls, the newest line in view. An edge with more lines past it fades out.
    private let outputBox = BackgroundView()
    private let outputScroll: NSScrollView
    private let outputText: NSTextView
    private let outputFade = CAGradientLayer()
    private var scrollOutputToEnd = false
    private let allowButton = NSButton()
    private let alwaysButton = NSButton()
    private let denyButton = NSButton()
    private let secretField = NSSecureTextField()
    private let plainField = NSTextField()
    private let sendButton = NSButton()
    private let stopButton = NSButton()
    /// The rule Always allow adds, where an answer goes, or why an answer did not go through.
    private let note = Build.label("", font: Layout.noteFont, color: .secondaryLabelColor, lines: 2)
    private var groupStart = true
    private var messageID: Message.ID?
    private var run: CommandRun?
    private var answerError: String?
    private var busy = false { didSet { updateControls() } }

    /// Answers the question: `allow`, `always`, or `deny`.
    var onDecision: ((String) -> Void)?
    /// Sends the user's answer to the command; throws why it could not go.
    var onSend: ((String) async throws -> Void)?
    var onStop: (() async throws -> Void)?
    /// The command's block was clicked: show the whole command.
    var onShowCommand: (() -> Void)?

    private var field: NSTextField { run?.asksYesOrNo == true ? plainField : secretField }

    init() {
        let scroll = NSTextView.scrollableTextView()
        outputScroll = scroll
        outputText = scroll.documentView as? NSTextView ?? NSTextView()
        super.init(frame: .zero)
        box.cornerRadius = 12
        box.fillColor = Theme.botBubble
        box.borderColor = Theme.botBubbleBorder
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
        icon.image = NSImage(systemSymbolName: "terminal", accessibilityDescription: nil)
        icon.contentTintColor = .controlAccentColor
        command.onClick = { [weak self] in self?.onShowCommand?() }
        caption.lineBreakMode = .byWordWrapping
        outputBox.fillColor = Theme.codeBackground
        outputBox.cornerRadius = 6
        outputScroll.drawsBackground = false
        outputScroll.borderType = .noBorder
        outputScroll.hasVerticalScroller = true
        outputScroll.autohidesScrollers = true
        outputText.isEditable = false
        outputText.isSelectable = true
        outputText.drawsBackground = false
        outputText.textContainer?.lineFragmentPadding = 0
        outputText.textContainerInset = NSSize(width: Layout.screenPadX, height: Layout.screenPadY)
        // The block's background is see-through, so the fade is a mask on the text, not a
        // color painted over it.
        outputScroll.wantsLayer = true
        outputScroll.contentView.postsBoundsChangedNotifications = true
        NotificationCenter.default.addObserver(self, selector: #selector(outputScrolled(_:)), name: NSView.boundsDidChangeNotification, object: outputScroll.contentView)
        for (button, label, decision) in [(allowButton, L("Allow once"), "allow"), (alwaysButton, L("Always allow"), "always"), (denyButton, L("Deny"), "deny")] {
            button.title = label
            button.bezelStyle = .rounded
            button.controlSize = .small
            button.font = .systemFont(ofSize: 11)
            button.target = self
            button.action = #selector(decide(_:))
            button.identifier = NSUserInterfaceItemIdentifier(decision)
        }
        for field in [secretField, plainField] {
            field.placeholderString = L("Type your answer")
            field.bezelStyle = .roundedBezel
            field.controlSize = .small
            field.font = .systemFont(ofSize: 12)
            field.target = self
            field.action = #selector(send(_:))
            // An answer is never a word to complete; no content type, so AutoFill offers no
            // saved password for it either.
            field.isAutomaticTextCompletionEnabled = false
        }
        for (button, label, action) in [(sendButton, L("Send"), #selector(send(_:))), (stopButton, L("Stop"), #selector(stop(_:)))] {
            button.title = label
            button.bezelStyle = .rounded
            button.controlSize = .small
            button.font = .systemFont(ofSize: 11)
            button.target = self
            button.action = action
        }
        let views: [NSView] = [
            avatar, box, icon, title, command, caption, outputBox, outputScroll, allowButton, alwaysButton, denyButton,
            secretField, plainField, sendButton, stopButton, note,
        ]
        for view in views {
            addSubview(view.framePositioned())
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    /// `avatar` is the bot's in a group, nil in a DM.
    func configure(run: CommandRun, messageID: Message.ID, botName: String, avatar avatarContent: AvatarView.Content?, groupStart: Bool) {
        let previous = self.messageID == messageID ? self.run : nil
        // Another command, in a reused cell, starts empty; the same one keeps what the user is
        // typing while its output changes.
        if previous == nil {
            secretField.stringValue = ""
            plainField.stringValue = ""
            busy = false
            answerError = nil
        } else if previous?.output != run.output || previous?.state != run.state {
            answerError = nil
        }
        if !run.takesInput {
            secretField.stringValue = ""
            plainField.stringValue = ""
        }
        if previous?.output != run.output || previous == nil {
            let text = Self.outputText(run) ?? ""
            outputText.textStorage?.setAttributedString(NSAttributedString(string: text, attributes: [.font: Layout.screenFont, .foregroundColor: NSColor.labelColor]))
            scrollOutputToEnd = true
        }
        self.messageID = messageID
        self.groupStart = groupStart
        self.run = run
        avatar.isHidden = avatarContent == nil
        if let avatarContent { avatar.content = avatarContent }
        title.stringValue = Self.title(for: run, botName: botName)
        title.toolTip = run.command
        caption.stringValue = Self.caption(run) ?? ""
        let asking = run.prompt ?? L("Type your answer")
        secretField.setAccessibilityLabel(asking)
        plainField.setAccessibilityLabel(asking)
        updateNote()
        updateControls()
        needsLayout = true
    }

    private func updateControls() {
        let answering = run?.state == .waiting && run?.sessionID != nil
        let yesOrNo = run?.asksYesOrNo ?? false
        secretField.isHidden = !answering || yesOrNo
        plainField.isHidden = !answering || !yesOrNo
        sendButton.isHidden = !answering
        stopButton.isHidden = !(run?.takesInput ?? false)
        for control in [secretField, plainField, sendButton, stopButton] as [NSControl] {
            control.isEnabled = !busy
        }
        // The question's buttons: Allow once and Deny, with Always allow when there is a rule.
        let buttons = [allowButton, alwaysButton, denyButton]
        for button in buttons { button.isHidden = true }
        if let run, run.state == .asking {
            for (button, choice) in zip(buttons, run.choices) {
                button.title = choice.0
                button.identifier = NSUserInterfaceItemIdentifier(choice.1)
                button.isHidden = false
            }
        }
    }

    /// The note says why an answer did not go through, else what the card's state says there.
    private func updateNote() {
        note.stringValue = answerError ?? run.flatMap(Self.note) ?? ""
        note.textColor = answerError == nil ? .secondaryLabelColor : .systemRed
        note.toolTip = answerError
    }

    /// Why an answer or a Stop did not go through: under the answer field when there is one,
    /// else in a sheet.
    private func show(error: String) {
        if run.flatMap(Self.note) != nil {
            answerError = error
            updateNote()
        } else if let window {
            let alert = NSAlert()
            alert.messageText = L("Could not stop the command")
            alert.informativeText = error
            alert.beginSheetModal(for: window)
        }
    }

    @objc private func outputScrolled(_ notification: Notification) {
        updateOutputFade()
    }

    /// Fades the output at each edge with more lines past it: the cue that the block scrolls,
    /// and which way.
    private func updateOutputFade() {
        guard let layer = outputScroll.layer, let document = outputScroll.documentView, layer.bounds.height > 0 else { return }
        if layer.mask !== outputFade { layer.mask = outputFade }
        // The text view is flipped, so its clip view's origin is how far down it has scrolled.
        let visible = outputScroll.contentView.bounds
        let above = visible.minY > 1
        let below = visible.maxY < document.frame.height - 1
        let edge = min(16 / layer.bounds.height, 0.5)
        // The gradient's unit space starts at the top in a flipped layer, at the bottom in another.
        let flipped = layer.contentsAreFlipped()
        let shown = NSColor.black.cgColor
        let faded = NSColor.clear.cgColor
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        outputFade.frame = layer.bounds
        outputFade.startPoint = CGPoint(x: 0.5, y: flipped ? 0 : 1)
        outputFade.endPoint = CGPoint(x: 0.5, y: flipped ? 1 : 0)
        outputFade.colors = [above ? faded : shown, shown, shown, below ? faded : shown]
        outputFade.locations = [0, NSNumber(value: Double(edge)), NSNumber(value: Double(1 - edge)), 1]
        CATransaction.commit()
    }

    @objc private func decide(_ sender: NSButton) {
        guard let decision = sender.identifier?.rawValue else { return }
        onDecision?(decision)
    }

    @objc private func send(_ sender: Any?) {
        guard let onSend, !busy, run?.takesInput == true else { return }
        let field = field
        let text = field.stringValue
        busy = true
        answerError = nil
        updateNote()
        Task { @MainActor in
            defer { busy = false }
            do {
                try await onSend(text)
                field.stringValue = ""
            } catch {
                show(error: error.localizedDescription)
            }
        }
    }

    @objc private func stop(_ sender: Any?) {
        guard let onStop, !busy else { return }
        busy = true
        Task { @MainActor in
            defer { busy = false }
            do {
                try await onStop()
            } catch {
                show(error: error.localizedDescription)
            }
        }
    }

    override func layout() {
        super.layout()
        guard let run else { return }
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let x = ChatMetrics.indent(showsAvatar: !avatar.isHidden)
        let layout = Layout(run: run, rowWidth: bounds.width, indent: x)
        func place(_ rect: NSRect) -> NSRect { rect.offsetBy(dx: x, dy: top) }
        let right = x + layout.width - 12

        // `layout.height` is the box alone; the row adds `top` above it.
        box.frame = NSRect(x: x, y: top, width: layout.width, height: layout.height)
        avatar.frame = NSRect(
            x: ChatMetrics.horizontalInset, y: top + layout.height - ChatMetrics.avatarSize,
            width: ChatMetrics.avatarSize, height: ChatMetrics.avatarSize)
        icon.frame = NSRect(x: x + 12, y: top + 12, width: 18, height: 18)
        title.frame = place(layout.title)
        if !stopButton.isHidden {
            // Stop ends the title's line, apart from Send, which answers.
            let size = stopButton.intrinsicContentSize
            stopButton.frame = NSRect(x: right - size.width - 4, y: top + 8, width: size.width + 4, height: Layout.controlHeight)
            title.frame.size.width = stopButton.frame.minX - 8 - title.frame.minX
        }
        command.isHidden = layout.command == nil
        command.frame = layout.command.map(place) ?? .zero
        command.text = layout.commandText
        caption.isHidden = layout.caption == nil
        caption.frame = layout.caption.map(place) ?? .zero
        outputBox.isHidden = layout.output == nil
        outputScroll.isHidden = layout.output == nil
        outputBox.frame = layout.output.map(place) ?? .zero
        outputScroll.frame = outputBox.frame
        if scrollOutputToEnd, layout.output != nil {
            scrollOutputToEnd = false
            outputText.scrollToEndOfDocument(nil)
        }
        if layout.output != nil {
            updateOutputFade()
        }
        if let buttonsY = layout.buttonsY {
            var buttonX = x + Layout.textX - 2
            for button in [allowButton, alwaysButton, denyButton] where !button.isHidden {
                let size = button.intrinsicContentSize
                button.frame = NSRect(x: buttonX, y: top + buttonsY, width: size.width + 4, height: Layout.controlHeight)
                buttonX += size.width + 12
            }
        }
        if let answerY = layout.answerY {
            // The field takes what Send leaves.
            let y = top + answerY
            let sendSize = sendButton.intrinsicContentSize
            sendButton.frame = NSRect(x: right - sendSize.width - 4, y: y, width: sendSize.width + 4, height: Layout.controlHeight)
            let fieldFrame = NSRect(x: x + Layout.textX, y: y, width: sendButton.frame.minX - 8 - (x + Layout.textX), height: Layout.controlHeight)
            secretField.frame = fieldFrame
            plainField.frame = fieldFrame
        }
        note.isHidden = layout.note == nil
        note.frame = layout.note.map(place) ?? .zero
    }
}
