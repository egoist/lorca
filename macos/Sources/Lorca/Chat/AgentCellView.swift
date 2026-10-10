import AppKit

/// A coding agent's card (`AgentRun`), on the row of the call that started it, from Auto-review's
/// question to how it ended. The agent's name with Stop at the end of the line while it runs,
/// what it was asked, how it stands and where it works, and while it works its last lines in a
/// code block that scrolls. When it asks: Allow once, Always allow, and Deny before it starts or
/// runs a command; a pop-up of the choices its pane offers; or a field for an answer to type.
/// A click anywhere else opens its transcript. In a group the card sits in the bubbles' column,
/// the bot's avatar beside its bottom edge.
final class AgentCellView: TranscriptCellView {
    static let identifier = NSUserInterfaceItemIdentifier("AgentCell")

    static let width: CGFloat = 440

    /// The box's height at `rowWidth`, starting at `indent`, from the same `Layout` the cell lays
    /// itself out with.
    static func height(for agent: AgentRun, rowWidth: CGFloat, indent: CGFloat) -> CGFloat {
        Layout(agent: agent, rowWidth: rowWidth, indent: indent).height
    }

    /// "Chef wants to start Claude Code on Workbench", "Claude Code wants to run a command",
    /// "Claude Code asks", or the agent's name.
    static func title(for agent: AgentRun, botName: String) -> String {
        switch agent.question?.kind {
        case .start: agent.device.map { L("%@ wants to start %@ on %@", botName, agent.name, $0) } ?? L("%@ wants to start %@", botName, agent.name)
        case .command: L("%@ wants to run a command", agent.name)
        case .choices, .text: L("%@ asks", agent.name)
        case nil: agent.name
        }
    }

    /// Under the task: how it stands and where it works, or how it ended.
    static func detail(for agent: AgentRun) -> String? {
        if agent.question != nil { return nil }
        var parts = [agent.status]
        if agent.state == .failed, let outcome = agent.outcome { parts = [L("Failed: %@", outcome)] }
        if !agent.place.isEmpty, agent.state != .denied { parts.append(agent.place) }
        if let host = agent.hostName, agent.isOpen { parts.append(L("in %@", host)) }
        return parts.joined(separator: " · ")
    }

    /// For a screen reader: the title, the task, how it stands, and its last lines.
    static func spokenText(agent: AgentRun, botName: String) -> String {
        [title(for: agent, botName: botName), agent.task, detail(for: agent), agent.question?.command, agent.question?.text, outputText(agent)]
            .compactMap { $0 }.filter { !$0.isEmpty }.joined(separator: ": ")
    }

    /// Its last lines with something on them, while it works.
    private static func outputText(_ agent: AgentRun) -> String? {
        guard agent.question == nil, [.starting, .working].contains(agent.state) else { return nil }
        let lines = (agent.output ?? "").split(separator: "\n", omittingEmptySubsequences: true)
        return lines.isEmpty ? nil : lines.joined(separator: "\n")
    }

    /// What its pane asks, to read before answering.
    private static func questionText(_ agent: AgentRun) -> String? {
        guard let question = agent.question, question.kind == .choices || question.kind == .text else { return nil }
        return question.text.isEmpty ? nil : question.text
    }

    /// At the foot of the card: the rule Always allow adds, or where a typed answer goes.
    private static func note(_ agent: AgentRun) -> String? {
        switch agent.question?.kind {
        case .start, .command: agent.question?.rule.map { L("Always allow adds the rule “%@”.", $0) }
        case .text: L("What you type goes to %@, not into the chat.", agent.name)
        default: nil
        }
    }

    @MainActor private struct Layout {
        static let textX: CGFloat = 38
        static let titleFont = NSFont.systemFont(ofSize: 12.5, weight: .semibold)
        static let taskFont = NSFont.systemFont(ofSize: 12)
        static let detailFont = NSFont.systemFont(ofSize: 11)
        static let screenFont = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        static let captionFont = NSFont.systemFont(ofSize: 11.5)
        static let outputLines = 6
        static let questionLines = 8
        static let screenPadX: CGFloat = 8
        static let screenPadY: CGFloat = 5
        static let controlHeight: CGFloat = 22

        var width: CGFloat
        var height: CGFloat = 0
        var title = NSRect.zero
        var task: NSRect?
        var detail: NSRect?
        var command: NSRect?
        var commandText = ""
        var screen: NSRect?
        var caption: NSRect?
        var output: NSRect?
        var buttonsY: CGFloat?
        var choiceY: CGFloat?
        var answerY: CGFloat?
        var note: NSRect?

        init(agent: AgentRun, rowWidth: CGFloat, indent: CGFloat) {
            width = min(AgentCellView.width, rowWidth - indent - ChatMetrics.horizontalInset)
            let textWidth = width - Self.textX - 12
            title = NSRect(x: Self.textX, y: 11, width: textWidth, height: 17)
            var bottom = title.maxY
            if !agent.task.isEmpty {
                let row = NSRect(x: Self.textX, y: bottom + 3, width: textWidth, height: Self.measure(agent.task, font: Self.taskFont, width: textWidth, maxLines: 2))
                task = row
                bottom = row.maxY
            }
            if let text = AgentCellView.detail(for: agent) {
                let row = NSRect(x: Self.textX, y: bottom + 3, width: textWidth, height: Self.measure(text, font: Self.detailFont, width: textWidth, maxLines: 1))
                detail = row
                bottom = row.maxY
            }
            let columnWidth = textWidth - Self.screenPadX * 2
            if agent.question?.kind == .command, let command = agent.question?.command {
                let firstLine = command.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty } ?? command
                commandText = CommandCellView.commandLine(firstLine, fitting: columnWidth, font: Self.screenFont)
                let block = NSRect(x: Self.textX, y: bottom + 8, width: textWidth, height: Self.measure("X", font: Self.screenFont, width: columnWidth) + Self.screenPadY * 2)
                self.command = block
                bottom = block.maxY
            }
            if let text = AgentCellView.questionText(agent) {
                let block = NSRect(x: Self.textX, y: bottom + 8, width: textWidth, height: CommandOutputView.height(of: text, width: textWidth, lines: Self.questionLines))
                screen = block
                bottom = block.maxY
            }
            if let reason = agent.question?.reason, agent.question?.isPermission == true {
                let row = NSRect(x: Self.textX, y: bottom + 6, width: textWidth, height: Self.measure(reason, font: Self.captionFont, width: textWidth, maxLines: 4))
                caption = row
                bottom = row.maxY
            }
            if let text = AgentCellView.outputText(agent) {
                let block = NSRect(x: Self.textX, y: bottom + 8, width: textWidth, height: CommandOutputView.height(of: text, width: textWidth, lines: Self.outputLines))
                output = block
                bottom = block.maxY
            }
            switch agent.question?.kind {
            case .start, .command:
                buttonsY = bottom + 9
                bottom += 9 + Self.controlHeight
            case .choices:
                choiceY = bottom + 9
                bottom += 9 + Self.controlHeight
            case .text:
                answerY = bottom + 9
                bottom += 9 + Self.controlHeight
            case nil:
                break
            }
            if let text = AgentCellView.note(agent) {
                let row = NSRect(x: Self.textX, y: bottom + 7, width: textWidth, height: Self.measure(text, font: Self.detailFont, width: textWidth, maxLines: 2))
                note = row
                bottom = row.maxY
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
    private let task = Build.label("", font: Layout.taskFont, lines: 2)
    private let detail = Build.label("", font: Layout.detailFont, color: .secondaryLabelColor)
    private let command = CommandBlockView(font: Layout.screenFont, lines: 1, padding: NSSize(width: Layout.screenPadX, height: Layout.screenPadY))
    /// What its pane asks, grown to `Layout.questionLines` lines and then scrolling.
    private let screen = CommandOutputView()
    private let caption = Build.label("", font: Layout.captionFont, color: .secondaryLabelColor, lines: 4)
    private let output = CommandOutputView()
    private let allowButton = NSButton()
    private let alwaysButton = NSButton()
    private let denyButton = NSButton()
    private let choices = NSPopUpButton()
    private let answerButton = NSButton()
    private let field = NSTextField()
    private let sendButton = NSButton()
    private let stopButton = NSButton()
    private let note = Build.label("", font: Layout.detailFont, color: .secondaryLabelColor, lines: 2)
    private var groupStart = true
    private var messageID: Message.ID?
    private var agent: AgentRun?
    private var answerError: String?
    private var busy = false { didSet { updateControls() } }

    /// Answers a question about starting it or a command it wants to run: `allow`, `always`, or
    /// `deny`.
    var onDecision: ((String) -> Void)?
    /// Answers what its pane asks with one of its choices; throws why it could not.
    var onChoice: ((Int) async throws -> Void)?
    /// Answers what its pane asks with text; throws why it could not.
    var onSend: ((String) async throws -> Void)?
    var onStop: (() async throws -> Void)?
    /// The command's block was clicked: show the whole command.
    var onShowCommand: (() -> Void)?
    /// The card was clicked: show its transcript.
    var onOpen: (() -> Void)?

    init() {
        super.init(frame: .zero)
        box.cornerRadius = 12
        box.fillColor = Theme.botBubble
        box.borderColor = Theme.botBubbleBorder
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 13, weight: .medium)
        icon.image = NSImage(systemSymbolName: "chevron.left.forwardslash.chevron.right", accessibilityDescription: nil)
        icon.contentTintColor = .controlAccentColor
        task.lineBreakMode = .byWordWrapping
        detail.lineBreakMode = .byTruncatingTail
        caption.lineBreakMode = .byWordWrapping
        command.onClick = { [weak self] in self?.onShowCommand?() }
        for (button, label, decision) in [(allowButton, L("Allow once"), "allow"), (alwaysButton, L("Always allow"), "always"), (denyButton, L("Deny"), "deny")] {
            button.title = label
            button.identifier = NSUserInterfaceItemIdentifier(decision)
            small(button, action: #selector(decide(_:)))
        }
        choices.controlSize = .small
        choices.font = .systemFont(ofSize: 11)
        choices.setAccessibilityLabel(L("Answer"))
        field.placeholderString = L("Type your answer")
        field.bezelStyle = .roundedBezel
        field.controlSize = .small
        field.font = .systemFont(ofSize: 12)
        field.target = self
        field.action = #selector(send(_:))
        field.isAutomaticTextCompletionEnabled = false
        answerButton.title = L("Answer")
        small(answerButton, action: #selector(choose(_:)))
        sendButton.title = L("Send")
        small(sendButton, action: #selector(send(_:)))
        stopButton.title = L("Stop")
        small(stopButton, action: #selector(stop(_:)))
        let views: [NSView] = [
            avatar, box, icon, title, task, detail, command, screen, caption, output, allowButton, alwaysButton, denyButton,
            choices, answerButton, field, sendButton, stopButton, note,
        ]
        for view in views {
            addSubview(view.framePositioned())
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    private func small(_ button: NSButton, action: Selector) {
        button.bezelStyle = .rounded
        button.controlSize = .small
        button.font = .systemFont(ofSize: 11)
        button.target = self
        button.action = action
    }

    /// `avatar` is the bot's in a group, nil in a DM.
    func configure(agent: AgentRun, messageID: Message.ID, botName: String, avatar avatarContent: AvatarView.Content?, groupStart: Bool) {
        let previous = self.messageID == messageID ? self.agent : nil
        if previous == nil || previous?.question != agent.question {
            field.stringValue = ""
            answerError = nil
            busy = false
        }
        if previous?.output != agent.output || previous == nil {
            output.text = Self.outputText(agent) ?? ""
        }
        if previous?.question != agent.question || previous == nil {
            screen.text = Self.questionText(agent) ?? ""
            choices.removeAllItems()
            choices.addItems(withTitles: agent.question?.choices ?? [])
        }
        self.messageID = messageID
        self.groupStart = groupStart
        self.agent = agent
        avatar.isHidden = avatarContent == nil
        if let avatarContent { avatar.content = avatarContent }
        title.stringValue = Self.title(for: agent, botName: botName)
        task.stringValue = agent.task
        task.toolTip = agent.task
        detail.stringValue = Self.detail(for: agent) ?? ""
        detail.toolTip = agent.outcome
        caption.stringValue = agent.question?.reason ?? ""
        field.setAccessibilityLabel(agent.question?.text ?? L("Type your answer"))
        setAccessibilityElement(true)
        setAccessibilityRole(.group)
        setAccessibilityLabel(Self.spokenText(agent: agent, botName: botName))
        updateNote()
        updateControls()
        needsLayout = true
    }

    private func updateControls() {
        let kind = agent?.question?.kind
        for button in [allowButton, alwaysButton, denyButton] { button.isHidden = true }
        if let question = agent?.question, question.isPermission {
            for (button, decision) in zip([allowButton, alwaysButton, denyButton], question.decisions) {
                button.title = decision.0
                button.identifier = NSUserInterfaceItemIdentifier(decision.1)
                button.isHidden = false
            }
        }
        choices.isHidden = kind != .choices
        answerButton.isHidden = kind != .choices
        field.isHidden = kind != .text
        sendButton.isHidden = kind != .text
        stopButton.isHidden = !(agent?.isRunning ?? false)
        for control in [choices, answerButton, field, sendButton, stopButton] as [NSControl] {
            control.isEnabled = !busy
        }
    }

    private func updateNote() {
        note.stringValue = answerError ?? agent.flatMap(Self.note) ?? ""
        note.textColor = answerError == nil ? .secondaryLabelColor : .systemRed
    }

    /// Why an answer or a Stop did not go through: at the card's foot where it has one, else in
    /// an alert.
    private func show(error: String) {
        if agent.flatMap(Self.note) != nil {
            answerError = error
            updateNote()
        } else if let window {
            let alert = NSAlert()
            alert.messageText = agent?.question == nil ? L("Could not stop %@", agent?.name ?? "") : L("Could not answer %@", agent?.name ?? "")
            alert.informativeText = error
            alert.beginSheetModal(for: window)
        }
    }

    @objc private func decide(_ sender: NSButton) {
        guard let decision = sender.identifier?.rawValue else { return }
        onDecision?(decision)
    }

    @objc private func choose(_ sender: Any?) {
        guard let onChoice, !busy, choices.indexOfSelectedItem >= 0 else { return }
        run { try await onChoice(self.choices.indexOfSelectedItem) }
    }

    @objc private func send(_ sender: Any?) {
        guard let onSend, !busy, agent?.question?.kind == .text else { return }
        let text = field.stringValue
        run {
            try await onSend(text)
            self.field.stringValue = ""
        }
    }

    @objc private func stop(_ sender: Any?) {
        guard let onStop, !busy else { return }
        run { try await onStop() }
    }

    private func run(_ action: @escaping @MainActor () async throws -> Void) {
        busy = true
        answerError = nil
        updateNote()
        Task { @MainActor in
            defer { busy = false }
            do {
                try await action()
            } catch {
                show(error: error.localizedDescription)
            }
        }
    }

    // A click on the card, not on one of its controls, opens the transcript. The card claims the
    // press so the transcript under it never takes it for a drag.
    override func mouseDown(with event: NSEvent) {
        guard onOpen != nil, box.frame.contains(convert(event.locationInWindow, from: nil)) else {
            super.mouseDown(with: event)
            return
        }
    }

    override func mouseUp(with event: NSEvent) {
        guard let onOpen, box.frame.contains(convert(event.locationInWindow, from: nil)) else {
            super.mouseUp(with: event)
            return
        }
        onOpen()
    }

    override func accessibilityPerformPress() -> Bool {
        guard let onOpen else { return false }
        onOpen()
        return true
    }

    override func resetCursorRects() {
        super.resetCursorRects()
        if onOpen != nil { addCursorRect(box.frame, cursor: .pointingHand) }
    }

    override func layout() {
        super.layout()
        guard let agent else { return }
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let x = ChatMetrics.indent(showsAvatar: !avatar.isHidden)
        let layout = Layout(agent: agent, rowWidth: bounds.width, indent: x)
        func place(_ rect: NSRect) -> NSRect { rect.offsetBy(dx: x, dy: top) }
        let right = x + layout.width - 12

        box.frame = NSRect(x: x, y: top, width: layout.width, height: layout.height)
        avatar.frame = NSRect(
            x: ChatMetrics.horizontalInset, y: top + layout.height - ChatMetrics.avatarSize,
            width: ChatMetrics.avatarSize, height: ChatMetrics.avatarSize)
        icon.frame = NSRect(x: x + 12, y: top + 12, width: 18, height: 18)
        title.frame = place(layout.title)
        if !stopButton.isHidden {
            let size = stopButton.intrinsicContentSize
            stopButton.frame = NSRect(x: right - size.width - 4, y: top + 8, width: size.width + 4, height: Layout.controlHeight)
            title.frame.size.width = stopButton.frame.minX - 8 - title.frame.minX
        }
        task.isHidden = layout.task == nil
        task.frame = layout.task.map(place) ?? .zero
        detail.isHidden = layout.detail == nil
        detail.frame = layout.detail.map(place) ?? .zero
        command.isHidden = layout.command == nil
        command.frame = layout.command.map(place) ?? .zero
        command.text = layout.commandText
        screen.isHidden = layout.screen == nil
        screen.frame = layout.screen.map(place) ?? .zero
        caption.isHidden = layout.caption == nil
        caption.frame = layout.caption.map(place) ?? .zero
        output.isHidden = layout.output == nil
        output.frame = layout.output.map(place) ?? .zero
        if let buttonsY = layout.buttonsY {
            var buttonX = x + Layout.textX - 2
            for button in [allowButton, alwaysButton, denyButton] where !button.isHidden {
                let size = button.intrinsicContentSize
                button.frame = NSRect(x: buttonX, y: top + buttonsY, width: size.width + 4, height: Layout.controlHeight)
                buttonX += size.width + 12
            }
        }
        if let choiceY = layout.choiceY {
            // The pop-up takes what Answer leaves.
            let y = top + choiceY
            let size = answerButton.intrinsicContentSize
            answerButton.frame = NSRect(x: right - size.width - 4, y: y, width: size.width + 4, height: Layout.controlHeight)
            choices.frame = NSRect(x: x + Layout.textX - 3, y: y, width: answerButton.frame.minX - 8 - (x + Layout.textX - 3), height: Layout.controlHeight)
        }
        if let answerY = layout.answerY {
            let y = top + answerY
            let size = sendButton.intrinsicContentSize
            sendButton.frame = NSRect(x: right - size.width - 4, y: y, width: size.width + 4, height: Layout.controlHeight)
            field.frame = NSRect(x: x + Layout.textX, y: y, width: sendButton.frame.minX - 8 - (x + Layout.textX), height: Layout.controlHeight)
        }
        note.isHidden = layout.note == nil
        note.frame = layout.note.map(place) ?? .zero
        window?.invalidateCursorRects(for: self)
    }
}
