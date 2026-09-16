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

// MARK: - Message cell

/// A user bubble on the right; a bot bubble on the left. In a group the bot's name sits above
/// its first bubble and its avatar beside the bubble's bottom edge; a DM shows neither.
final class MessageCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("MessageCell")

    private let avatar = AvatarView(diameter: ChatMetrics.avatarSize)
    private let bubble = BubbleView()
    private let author = Build.label("", font: Theme.Font.author)
    private let stamp = Build.label("", font: Theme.Font.caption, alignment: .right)
    private let content = SegmentedTextView()
    private let attachments = AttachmentsView()

    private var isUser = false
    private var groupStart = true
    private var metrics = BubbleMetrics(
        textWidth: 0, textHeight: 0, timeWidth: 0, showsName: false, indent: 0)

    init() {
        super.init(frame: .zero)
        bubble.cornerRadius = Theme.Metric.bubbleCornerRadius
        stamp.setContentCompressionResistancePriority(.required, for: .horizontal)
        stamp.setContentHuggingPriority(.required, for: .horizontal)
        stamp.lineBreakMode = .byClipping
        addSubview(avatar.framePositioned())
        addSubview(author.framePositioned())
        addSubview(bubble.framePositioned())
        bubble.addSubview(stamp.framePositioned())
        bubble.addSubview(attachments.framePositioned())
        bubble.addSubview(content.framePositioned())
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
        attachments items: [AttachmentsView.Item],
        metrics: BubbleMetrics
    ) {
        isUser = message.author.isYou
        self.groupStart = groupStart
        self.metrics = metrics

        attachments.configure(items, onUserBubble: isUser)
        attachments.isHidden = items.isEmpty

        avatar.content = avatarContent
        avatar.isHidden = isUser || !metrics.showsName || !groupStart

        author.stringValue = authorName
        author.textColor = nameColor
        author.isHidden = !metrics.showsName || !groupStart
        stamp.stringValue = Preferences.showTimestamps ? Format.time(message.createdAt) : ""
        stamp.isHidden = stamp.stringValue.isEmpty

        content.configure(segments)

        if isUser {
            bubble.fillColor = Theme.userBubble
            bubble.borderColor = nil
            stamp.textColor = Theme.userBubbleText.withAlphaComponent(0.62)
        } else {
            bubble.fillColor = Theme.botBubble
            bubble.borderColor = Theme.botBubbleBorder
            stamp.textColor = .tertiaryLabelColor
        }

        let spoken = message.text.isEmpty ? Attachment.summary(message.attachments) : message.text
        setAccessibilityLabel("\(authorName): \(spoken)")
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let bubbleWidth = metrics.bubbleWidth
        let bubbleHeight = metrics.bubbleHeight
        let x = isUser ? bounds.width - ChatMetrics.horizontalInset - bubbleWidth : metrics.indent
        let nameHeight = author.isHidden ? 0 : metrics.headerHeight
        let bubbleY = top + nameHeight

        author.frame = NSRect(x: x + 4, y: top, width: bounds.width - x - ChatMetrics.horizontalInset, height: ChatMetrics.headerLineHeight)
        bubble.frame = NSRect(x: x, y: bubbleY, width: bubbleWidth, height: bubbleHeight)
        avatar.frame = NSRect(
            x: ChatMetrics.horizontalInset,
            y: bubbleY + bubbleHeight - ChatMetrics.avatarSize,
            width: ChatMetrics.avatarSize,
            height: ChatMetrics.avatarSize
        )

        let inner = bubble.bounds.insetBy(dx: ChatMetrics.bubblePadX, dy: ChatMetrics.bubblePadY)
        attachments.frame = NSRect(
            x: inner.minX, y: inner.minY, width: metrics.attachmentsSize.width, height: metrics.attachmentsSize.height)
        let textY = inner.minY + metrics.attachmentsBlockHeight
        content.frame = NSRect(
            x: inner.minX, y: textY, width: metrics.textWidth, height: metrics.textHeight)

        let stampWidth = metrics.timeWidth
        stamp.frame = NSRect(
            x: inner.maxX - stampWidth,
            y: textY + max(0, metrics.textHeight - ChatMetrics.timeLineHeight),
            width: stampWidth,
            height: ChatMetrics.timeLineHeight)
    }
}

// MARK: - Working row

/// "Chef is working…" after the last message: the avatar breathes while a turn runs. A DM
/// shows the avatar alone, since only one bot can be at work there.
final class WorkingCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("WorkingCell")

    private let avatar = AvatarView(diameter: ChatMetrics.avatarSize)
    private let label = Build.label("", font: .systemFont(ofSize: 12.5), color: .secondaryLabelColor)

    init() {
        super.init(frame: .zero)
        addSubview(avatar.framePositioned())
        addSubview(label.framePositioned())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    /// `activity` is what the one working bot is doing right now ("Running commands"); with it
    /// the line reads the activity, without it the bot's name. A DM shows the avatar alone
    /// unless there is an activity to name.
    func configure(bots: [Bot], activity: String?, showsName: Bool) {
        if let first = bots.first {
            avatar.content = .bot(symbolName: first.symbolName, accent: first.accent)
        }
        let names = bots.map(\.name)
        let text: String
        if let activity, names.count == 1 {
            text = "\(activity)…"
        } else {
            switch names.count {
            case 0: text = ""
            case 1: text = "\(names[0]) is working…"
            default: text = "\(names.dropLast().joined(separator: ", ")) and \(names.last ?? "") are working…"
            }
        }
        label.stringValue = showsName || activity != nil ? text : ""
        setAccessibilityLabel(names.count == 1 ? "\(names[0]) is working" : text)
        needsLayout = true
    }

    /// What a tool row means while it runs, in the words of the status line.
    static func activity(for tool: ToolInvocation, targetName: String?) -> String {
        switch tool.name {
        case "read": return "Reading a file"
        case "write", "edit": return "Drafting a file"
        case "bash": return "Running commands"
        case "grep", "find", "ls": return "Searching files"
        case "message_bot": return targetName.map { "Messaging \($0)" } ?? "Messaging another bot"
        case "list_teammates": return "Checking the team"
        case "create_bot": return "Creating a bot"
        case "remember": return "Taking a note"
        default: return "Working"
        }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        window == nil ? stopBreathing() : startBreathing()
    }

    private func startBreathing() {
        avatar.wantsLayer = true
        guard let layer = avatar.layer, layer.animation(forKey: "breathe") == nil else { return }
        let pulse = CABasicAnimation(keyPath: "opacity")
        pulse.fromValue = 1
        pulse.toValue = 0.35
        pulse.duration = 0.9
        pulse.autoreverses = true
        pulse.repeatCount = .infinity
        pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
        layer.add(pulse, forKey: "breathe")
    }

    private func stopBreathing() {
        avatar.layer?.removeAnimation(forKey: "breathe")
    }

    override func layout() {
        super.layout()
        let y = ChatMetrics.groupTopPadding
        avatar.frame = NSRect(
            x: ChatMetrics.horizontalInset, y: y, width: ChatMetrics.avatarSize, height: ChatMetrics.avatarSize)
        label.frame = NSRect(
            x: ChatMetrics.bubbleIndent, y: y + (ChatMetrics.avatarSize - 16) / 2,
            width: max(0, bounds.width - ChatMetrics.bubbleIndent - ChatMetrics.horizontalInset), height: 16)
    }
}

// MARK: - Status row

/// A quiet centered line, such as "Chef stopped without replying".
final class StatusCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("StatusCell")

    private let label = Build.label(
        "", font: .systemFont(ofSize: 11.5), color: .tertiaryLabelColor, alignment: .center)

    init() {
        super.init(frame: .zero)
        addSubview(label.framePositioned())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(_ text: String) {
        label.stringValue = text
        setAccessibilityLabel(text)
        needsLayout = true
    }

    override func layout() {
        super.layout()
        label.frame = NSRect(
            x: ChatMetrics.horizontalInset, y: (ChatMetrics.statusRowHeight - 16) / 2,
            width: max(0, bounds.width - ChatMetrics.horizontalInset * 2), height: 16)
    }
}

// MARK: - Handoff cell

/// Bot-to-bot messages as centered markers: "Message from ◉ Name" where one arrived, "Messaged ◉
/// Name" where one was sent, with the text itself as the tooltip. A handoff between two bots in
/// the same chat keeps both avatars on one line.
final class HandoffCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("HandoffCell")

    enum Mode {
        case incoming(from: Bot?)
        case outgoing(to: Bot?)
        case handoff(from: Bot?, to: Bot?)
    }

    private let fromAvatar = AvatarView(diameter: 18)
    private let arrow = NSImageView()
    private let toAvatar = AvatarView(diameter: 18)
    private let lead = Build.label("", font: .systemFont(ofSize: 11.5), color: .secondaryLabelColor)
    private let label = Build.label("", font: .systemFont(ofSize: 11.5), color: .secondaryLabelColor)
    private var groupStart = true
    private var incoming = false

    init() {
        super.init(frame: .zero)
        arrow.image = NSImage(systemSymbolName: "arrow.right", accessibilityDescription: nil)
        arrow.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 9, weight: .semibold)
        arrow.contentTintColor = .tertiaryLabelColor
        lead.stringValue = "Message from"

        addSubview(lead.framePositioned())
        addSubview(fromAvatar.framePositioned())
        addSubview(arrow.framePositioned())
        addSubview(toAvatar.framePositioned())
        addSubview(label.framePositioned())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(mode: Mode, reason: String, groupStart: Bool) {
        self.groupStart = groupStart
        let avatar = { (bot: Bot?) -> AvatarView.Content in
            bot.map { .bot(symbolName: $0.symbolName, accent: $0.accent) } ?? .system
        }
        switch mode {
        case let .incoming(from), let .outgoing(to: from):
            incoming = true
            let verb: String
            if case .incoming = mode { verb = "Message from" } else { verb = "Messaged" }
            lead.stringValue = verb
            fromAvatar.content = avatar(from)
            lead.isHidden = false
            arrow.isHidden = true
            toAvatar.isHidden = true
            label.stringValue = from?.name ?? "?"
            label.textColor = .labelColor
            label.font = .systemFont(ofSize: 11.5, weight: .medium)
            toolTip = reason
            setAccessibilityLabel("\(verb) \(from?.name ?? "?"): \(reason)")
        case let .handoff(from, to):
            incoming = false
            fromAvatar.content = avatar(from)
            toAvatar.content = avatar(to)
            lead.isHidden = true
            arrow.isHidden = false
            toAvatar.isHidden = false
            label.stringValue = "\(from?.name ?? "?") handed off to \(to?.name ?? "?") · \(reason)"
            label.textColor = .secondaryLabelColor
            label.font = .systemFont(ofSize: 11.5)
            toolTip = nil
            setAccessibilityLabel(label.stringValue)
        }
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let centerY = top + 17

        if incoming {
            // Cell sizes, not glyph bounds: a text field pads its text and clips without it.
            let leadWidth = TextMeasure.labelSize(of: lead.attributedStringValue).width
            let nameWidth = TextMeasure.labelSize(of: label.attributedStringValue).width
            let avatarSize: CGFloat = 16
            let total = leadWidth + 6 + avatarSize + 5 + nameWidth
            var x = (bounds.width - total) / 2
            lead.frame = NSRect(x: x, y: centerY - 8, width: leadWidth, height: 16)
            x += leadWidth + 6
            fromAvatar.frame = NSRect(x: x, y: centerY - avatarSize / 2, width: avatarSize, height: avatarSize)
            x += avatarSize + 5
            label.frame = NSRect(x: x, y: centerY - 8, width: nameWidth, height: 16)
            return
        }

        let x = ChatMetrics.bubbleIndent
        fromAvatar.frame = NSRect(x: x, y: centerY - 9, width: 18, height: 18)
        arrow.frame = NSRect(x: x + 22, y: centerY - 6, width: 12, height: 12)
        toAvatar.frame = NSRect(x: x + 38, y: centerY - 9, width: 18, height: 18)
        label.frame = NSRect(
            x: x + 64, y: centerY - 8,
            width: max(0, bounds.width - x - 64 - ChatMetrics.horizontalInset), height: 16)
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
