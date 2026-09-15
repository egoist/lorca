import AppKit

// MARK: - Segment stack

/// Lays out the parsed message body: wrapped text runs and fenced code blocks.
final class SegmentedTextView: NSView {
    private var labels: [NSTextField] = []
    private var codeBoxes: [BackgroundView] = []
    private var segments: [MessageSegment] = []

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(_ newSegments: [MessageSegment]) {
        segments = newSegments
        syncViews()
        needsLayout = true
    }

    private func syncViews() {
        while labels.count < segments.count {
            let field = NSTextField(labelWithString: "")
            field.maximumNumberOfLines = 0
            field.lineBreakMode = .byWordWrapping
            field.cell?.wraps = true
            field.cell?.isScrollable = false
            field.isSelectable = true
            field.allowsEditingTextAttributes = true
            addSubview(field.framePositioned())
            labels.append(field)

            let box = BackgroundView()
            box.cornerRadius = 7
            box.isHidden = true
            addSubview(box.framePositioned(), positioned: .below, relativeTo: field)
            codeBoxes.append(box)
        }
        while labels.count > segments.count {
            labels.removeLast().removeFromSuperview()
            codeBoxes.removeLast().removeFromSuperview()
        }

        for (index, segment) in segments.enumerated() {
            let label = labels[index]
            let box = codeBoxes[index]
            switch segment {
            case let .text(attributed):
                label.attributedStringValue = attributed
                box.isHidden = true
            case let .code(attributed, _):
                label.attributedStringValue = attributed
                box.isHidden = false
                box.fillColor = Theme.codeBackground
            }
        }
    }

    override func layout() {
        super.layout()
        var y: CGFloat = 0
        for (index, segment) in segments.enumerated() {
            let label = labels[index]
            let box = codeBoxes[index]

            switch segment {
            case let .text(attributed):
                let height = TextMeasure.labelSize(of: attributed, width: bounds.width).height
                label.frame = NSRect(x: 0, y: y, width: bounds.width, height: height)
                y += height

            case let .code(attributed, _):
                let inner = bounds.width - Markdown.codePaddingX * 2
                let height = TextMeasure.labelSize(of: attributed, width: inner).height
                box.frame = NSRect(
                    x: 0, y: y, width: bounds.width, height: height + Markdown.codePaddingY * 2)
                label.frame = NSRect(
                    x: Markdown.codePaddingX,
                    y: y + Markdown.codePaddingY,
                    width: inner,
                    height: height
                )
                y += height + Markdown.codePaddingY * 2
            }

            if index < segments.count - 1 { y += Markdown.segmentSpacing }
        }
    }
}

// MARK: - Typing indicator

final class TypingIndicatorView: NSView {
    private var dots: [CALayer] = []

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        layer?.masksToBounds = false
        for _ in 0..<3 {
            let dot = CALayer()
            dot.cornerRadius = 3
            dot.backgroundColor = NSColor.secondaryLabelColor.cgColor
            layer?.addSublayer(dot)
            dots.append(dot)
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func layout() {
        super.layout()
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        for (index, dot) in dots.enumerated() {
            dot.frame = CGRect(x: CGFloat(index) * 11, y: (bounds.height - 6) / 2, width: 6, height: 6)
        }
        CATransaction.commit()
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        window == nil ? stop() : start()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        for dot in dots { dot.backgroundColor = NSColor.secondaryLabelColor.cgColor }
    }

    private func start() {
        for (index, dot) in dots.enumerated() {
            let pulse = CABasicAnimation(keyPath: "opacity")
            pulse.fromValue = 0.25
            pulse.toValue = 0.95
            pulse.duration = 0.5
            pulse.autoreverses = true
            pulse.repeatCount = .infinity
            pulse.timeOffset = Double(index) * 0.18
            dot.add(pulse, forKey: "pulse")
        }
    }

    private func stop() {
        for dot in dots { dot.removeAllAnimations() }
    }
}

// MARK: - Message cell

final class MessageCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("MessageCell")

    private let avatar = AvatarView(diameter: ChatMetrics.avatarSize)
    private let bubble = BubbleView()
    private let author = Build.label("", font: Theme.Font.author)
    private let stamp = Build.label("", font: Theme.Font.caption, alignment: .right)
    private let content = SegmentedTextView()
    private let typing = TypingIndicatorView()

    private var isUser = false
    private var groupStart = true
    private var isThinking = false
    private var metrics = BubbleMetrics(
        textWidth: 0, textHeight: 0, nameWidth: 0, timeWidth: 0, showsName: false)

    init() {
        super.init(frame: .zero)
        bubble.cornerRadius = Theme.Metric.bubbleCornerRadius
        stamp.setContentCompressionResistancePriority(.required, for: .horizontal)
        stamp.setContentHuggingPriority(.required, for: .horizontal)
        stamp.lineBreakMode = .byClipping
        addSubview(avatar.framePositioned())
        addSubview(bubble.framePositioned())
        bubble.addSubview(author.framePositioned())
        bubble.addSubview(stamp.framePositioned())
        bubble.addSubview(content.framePositioned())
        bubble.addSubview(typing.framePositioned())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(
        message: Message,
        groupStart: Bool,
        authorName: String,
        nameColor: NSColor,
        avatarContent: AvatarView.Content,
        segments: [MessageSegment],
        metrics: BubbleMetrics
    ) {
        isUser = message.author.isYou
        self.groupStart = groupStart
        self.metrics = metrics
        isThinking = message.state == .thinking

        avatar.content = avatarContent
        avatar.isHidden = isUser || !groupStart

        author.stringValue = authorName
        author.textColor = nameColor
        author.isHidden = !metrics.showsName
        stamp.stringValue = Preferences.showTimestamps ? Format.time(message.createdAt) : ""
        stamp.isHidden = stamp.stringValue.isEmpty

        typing.isHidden = !isThinking
        content.isHidden = isThinking
        if !isThinking { content.configure(segments) }

        if isUser {
            bubble.fillColor = Theme.userBubble
            bubble.borderColor = nil
            stamp.textColor = Theme.userBubbleText.withAlphaComponent(0.62)
        } else {
            bubble.fillColor = Theme.botBubble
            bubble.borderColor = Theme.botBubbleBorder
            stamp.textColor = .tertiaryLabelColor
        }

        setAccessibilityLabel("\(authorName): \(message.text)")
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding

        let bubbleHeight: CGFloat
        let bubbleWidth: CGFloat
        if isThinking {
            let typingRow = 28 + metrics.timeGutter
            bubbleWidth = ChatMetrics.bubblePadX * 2 + max(metrics.nameWidth, typingRow)
            bubbleHeight =
                ChatMetrics.bubblePadY + metrics.headerHeight + ChatMetrics.thinkingBubbleHeight
                + ChatMetrics.bubblePadY
        } else {
            bubbleWidth = metrics.bubbleWidth
            bubbleHeight = metrics.bubbleHeight
        }
        let x =
            isUser
            ? bounds.width - ChatMetrics.horizontalInset - bubbleWidth
            : ChatMetrics.bubbleIndent

        bubble.frame = NSRect(x: x, y: top, width: bubbleWidth, height: bubbleHeight)
        avatar.frame = NSRect(
            x: ChatMetrics.horizontalInset,
            y: top,
            width: ChatMetrics.avatarSize,
            height: ChatMetrics.avatarSize
        )

        let inner = bubble.bounds.insetBy(dx: ChatMetrics.bubblePadX, dy: ChatMetrics.bubblePadY)
        author.frame = NSRect(
            x: inner.minX, y: inner.minY, width: inner.width, height: ChatMetrics.headerLineHeight)

        let bodyY = inner.minY + metrics.headerHeight
        let bodyHeight = isThinking ? ChatMetrics.thinkingBubbleHeight : metrics.textHeight
        content.frame = NSRect(
            x: inner.minX, y: bodyY, width: metrics.textWidth, height: metrics.textHeight)
        typing.frame = NSRect(x: inner.minX, y: bodyY, width: 28, height: bodyHeight)

        let stampWidth = metrics.timeWidth
        stamp.frame = NSRect(
            x: inner.maxX - stampWidth,
            y: bodyY + max(0, bodyHeight - ChatMetrics.timeLineHeight),
            width: stampWidth,
            height: ChatMetrics.timeLineHeight)
    }
}

// MARK: - Tool cell

final class ToolCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("ToolCell")

    private let chip = BackgroundView()
    private let icon = NSImageView()
    private let summary = Build.label("", font: .systemFont(ofSize: 11.5, weight: .medium), color: .secondaryLabelColor)
    private let chevron = NSImageView()
    private let spinner = NSProgressIndicator()
    private let detailBox = BackgroundView()
    private let detail = Build.label("", font: Theme.Font.code, color: .labelColor, lines: 0)
    private let button = NSButton()

    private var isExpanded = false
    private var groupStart = true
    var onToggle: (() -> Void)?

    init() {
        super.init(frame: .zero)

        chip.cornerRadius = ChatMetrics.chipHeight / 2
        chip.fillColor = Theme.chipBackground

        icon.contentTintColor = .secondaryLabelColor

        chevron.image = NSImage(systemSymbolName: "chevron.down", accessibilityDescription: nil)
        chevron.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 9, weight: .semibold)
        chevron.contentTintColor = .tertiaryLabelColor

        spinner.style = .spinning
        spinner.controlSize = .small
        spinner.isDisplayedWhenStopped = false

        detailBox.cornerRadius = 8
        detailBox.fillColor = Theme.codeBackground
        detailBox.isHidden = true

        detail.isSelectable = true

        button.title = ""
        button.isBordered = false
        button.isTransparent = true
        button.target = self
        button.action = #selector(toggle)

        addSubview(chip.framePositioned())
        chip.addSubview(icon.framePositioned())
        chip.addSubview(summary.framePositioned())
        chip.addSubview(chevron.framePositioned())
        chip.addSubview(spinner.framePositioned())
        addSubview(detailBox.framePositioned())
        detailBox.addSubview(detail.framePositioned())
        addSubview(button.framePositioned())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(invocation: ToolInvocation, groupStart: Bool, expanded: Bool) {
        self.groupStart = groupStart
        isExpanded = expanded

        icon.image = NSImage(systemSymbolName: invocation.symbolName, accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .medium)
        icon.isHidden = invocation.isRunning

        summary.stringValue = invocation.isRunning ? "Running \(invocation.name)…" : invocation.summary
        detail.stringValue = invocation.detail

        chevron.isHidden = invocation.isRunning || invocation.detail.isEmpty
        chevron.image = NSImage(
            systemSymbolName: expanded ? "chevron.up" : "chevron.down", accessibilityDescription: nil)
        detailBox.isHidden = !expanded
        button.isEnabled = !invocation.detail.isEmpty

        if invocation.isRunning { spinner.startAnimation(nil) } else { spinner.stopAnimation(nil) }
        needsLayout = true
    }

    @objc private func toggle() {
        onToggle?()
    }

    override func layout() {
        super.layout()
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding

        let textWidth = TextMeasure.labelSize(of: summary.attributedStringValue).width
        let x = ChatMetrics.bubbleIndent
        let chipWidth = min(
            bounds.width - x - ChatMetrics.horizontalInset,
            textWidth + 30 + (chevron.isHidden ? 10 : 24)
        )
        chip.frame = NSRect(x: x, y: top, width: max(120, chipWidth), height: ChatMetrics.chipHeight)
        button.frame = chip.frame

        icon.frame = NSRect(x: 10, y: (ChatMetrics.chipHeight - 14) / 2, width: 14, height: 14)
        spinner.frame = NSRect(x: 10, y: (ChatMetrics.chipHeight - 12) / 2, width: 12, height: 12)
        summary.frame = NSRect(
            x: 30, y: (ChatMetrics.chipHeight - 15) / 2,
            width: max(0, chip.frame.width - 30 - (chevron.isHidden ? 8 : 24)), height: 15)
        chevron.frame = NSRect(
            x: chip.frame.width - 20, y: (ChatMetrics.chipHeight - 12) / 2, width: 12, height: 12)

        guard isExpanded else { return }
        let boxWidth = ChatMetrics.toolDetailWidth(tableWidth: bounds.width)
        let textBoxWidth = boxWidth - 24
        let text = detail.attributedStringValue
        let height = TextMeasure.labelSize(of: text, width: textBoxWidth).height
        detailBox.frame = NSRect(
            x: x, y: top + ChatMetrics.chipHeight + 6, width: boxWidth, height: height + 16)
        detail.frame = NSRect(x: 12, y: 8, width: textBoxWidth, height: height)
    }
}

// MARK: - Handoff cell

final class HandoffCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("HandoffCell")

    private let track = BackgroundView()
    private let fromAvatar = AvatarView(diameter: 18)
    private let arrow = NSImageView()
    private let toAvatar = AvatarView(diameter: 18)
    private let label = Build.label("", font: .systemFont(ofSize: 11.5), color: .secondaryLabelColor)
    private var groupStart = true

    init() {
        super.init(frame: .zero)
        track.cornerRadius = 9
        track.fillColor = .clear

        arrow.image = NSImage(systemSymbolName: "arrow.right", accessibilityDescription: nil)
        arrow.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 9, weight: .semibold)
        arrow.contentTintColor = .tertiaryLabelColor

        addSubview(track.framePositioned())
        addSubview(fromAvatar.framePositioned())
        addSubview(arrow.framePositioned())
        addSubview(toAvatar.framePositioned())
        addSubview(label.framePositioned())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(from: Bot?, to: Bot?, reason: String, groupStart: Bool) {
        self.groupStart = groupStart
        fromAvatar.content =
            from.map { .bot(symbolName: $0.symbolName, accent: $0.accent) } ?? .system
        toAvatar.content = to.map { .bot(symbolName: $0.symbolName, accent: $0.accent) } ?? .system
        label.stringValue = "\(from?.name ?? "?") handed off to \(to?.name ?? "?") · \(reason)"
        setAccessibilityLabel(label.stringValue)
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let x = ChatMetrics.bubbleIndent
        let centerY = top + 17

        fromAvatar.frame = NSRect(x: x, y: centerY - 9, width: 18, height: 18)
        arrow.frame = NSRect(x: x + 22, y: centerY - 6, width: 12, height: 12)
        toAvatar.frame = NSRect(x: x + 38, y: centerY - 9, width: 18, height: 18)
        label.frame = NSRect(
            x: x + 64, y: centerY - 8,
            width: max(0, bounds.width - x - 64 - ChatMetrics.horizontalInset), height: 16)
        track.frame = NSRect(x: x - 6, y: top, width: bounds.width - x, height: 34)
    }
}

// MARK: - Notice cell

final class NoticeCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("NoticeCell")

    private let box = BackgroundView()
    private let icon = NSImageView()
    private let label = Build.label(
        "", font: Theme.Font.notice, color: .secondaryLabelColor, lines: 0)
    private var groupStart = true
    private var metrics = NoticeMetrics(boxSize: .zero, iconFrame: .zero, labelFrame: .zero)

    init() {
        super.init(frame: .zero)
        box.cornerRadius = 9
        box.fillColor = Theme.chipBackground
        icon.image = NSImage(systemSymbolName: "clock.badge.questionmark", accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .medium)
        icon.contentTintColor = .tertiaryLabelColor
        // Beside the box, not inside it, so icon and label use the cell's flipped coordinates.
        addSubview(box.framePositioned())
        addSubview(icon.framePositioned())
        addSubview(label.framePositioned())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(text: String, groupStart: Bool, metrics: NoticeMetrics) {
        self.groupStart = groupStart
        self.metrics = metrics
        label.stringValue = text
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let x = ((bounds.width - metrics.boxSize.width) / 2).rounded()
        box.frame = NSRect(origin: NSPoint(x: x, y: top), size: metrics.boxSize)
        icon.frame = metrics.iconFrame.offsetBy(dx: x, dy: top)
        label.frame = metrics.labelFrame.offsetBy(dx: x, dy: top)
    }
}

// MARK: - Day separator

final class DayCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("DayCell")

    private let pill = BackgroundView()
    private let label = Build.label(
        "", font: .systemFont(ofSize: 10.5, weight: .medium), color: .secondaryLabelColor,
        alignment: .center)

    init() {
        super.init(frame: .zero)
        pill.cornerRadius = 8
        pill.fillColor = Theme.chipBackground
        addSubview(pill.framePositioned())
        pill.addSubview(label.framePositioned())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(_ date: Date) {
        label.stringValue = Format.daySeparator(date)
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let width = TextMeasure.width(of: label.attributedStringValue) + 22
        pill.frame = NSRect(
            x: (bounds.width - width) / 2,
            y: (ChatMetrics.dayRowHeight - 16) / 2,
            width: width,
            height: 16
        )
        label.frame = NSRect(x: 0, y: 1, width: width, height: 14)
    }
}
