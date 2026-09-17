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
            avatar.content = AvatarView.content(for: first)
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

    /// What a tool row means while it runs, in the words of the status line. `pluginName` is
    /// the plugin behind a `<plugin>__<tool>` row, so the line reads "Using GitHub" between
    /// two calls as well as during one.
    static func activity(for tool: ToolInvocation, targetName: String?, pluginName: String? = nil) -> String {
        switch tool.name {
        case "read": return "Reading a file"
        case "write", "edit": return "Drafting a file"
        case "bash": return "Running commands"
        case "web_search": return "Searching the web"
        case "web_fetch": return "Reading the web"
        case "grep", "find", "ls": return "Searching files"
        case "message_bot": return targetName.map { "Messaging \($0)" } ?? "Messaging another bot"
        case "list_teammates": return "Checking the team"
        case "create_bot": return "Creating a bot"
        case "edit_bot": return "Updating a bot"
        case "remember": return "Taking a note"
        case "search_plugins": return "Searching plugins"
        case "install_plugin": return "Installing a plugin"
        default:
            // A plugin tool: "Using GitHub", whether the call is running or just finished, so
            // a run of quick calls never flashes back to "Working" between them.
            if let pluginName {
                return "Using \(pluginName)"
            }
            if tool.name.contains("__"), tool.summary.hasPrefix("Using ") {
                return String(tool.summary.dropLast(tool.summary.hasSuffix("…") ? 1 : 0))
            }
            return "Working"
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

/// Bot-to-bot messages as centered markers: "Message from ◉ Name · first line" where one arrived,
/// "Messaged ◉ Name · first line" where one was sent. The preview is one truncated line; a click on
/// the marker opens the whole message in a popover. A handoff between two bots in the same chat
/// keeps both avatars on one line with the same preview and popover.
final class HandoffCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("HandoffCell")

    /// The preview line never grows past this, so a long message still reads as a marker.
    static let previewMaxWidth: CGFloat = 260

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
    private let preview = Build.label("", font: .systemFont(ofSize: 11.5), color: .secondaryLabelColor)
    private var groupStart = true
    private var incoming = false
    private var fullText = ""
    /// The marker's bounds after layout: the click target and pointer cursor region.
    private var markerRect = NSRect.zero

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
        addSubview(preview.framePositioned())
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(mode: Mode, reason: String, groupStart: Bool) {
        self.groupStart = groupStart
        fullText = reason.trimmingCharacters(in: .whitespacesAndNewlines)
        let firstLine = TextPopover.firstLine(of: fullText)
        preview.stringValue = firstLine.isEmpty ? "" : "· \(firstLine)"
        preview.isHidden = firstLine.isEmpty
        let avatar = { (bot: Bot?) -> AvatarView.Content in
            bot.map { AvatarView.content(for: $0) } ?? .system
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
            setAccessibilityLabel("\(verb) \(from?.name ?? "?"): \(fullText)")
        case let .handoff(from, to):
            incoming = false
            fromAvatar.content = avatar(from)
            toAvatar.content = avatar(to)
            lead.isHidden = true
            arrow.isHidden = false
            toAvatar.isHidden = false
            label.stringValue = "\(from?.name ?? "?") handed off to \(to?.name ?? "?")"
            label.textColor = .secondaryLabelColor
            label.font = .systemFont(ofSize: 11.5)
            setAccessibilityLabel("\(label.stringValue): \(fullText)")
        }
        needsLayout = true
    }

    override func layout() {
        super.layout()
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let centerY = top + 17
        let previewGap: CGFloat = 5
        let available = max(0, bounds.width - ChatMetrics.horizontalInset * 2)

        if incoming {
            // Cell sizes, not glyph bounds: a text field pads its text and clips without it.
            let leadWidth = TextMeasure.labelSize(of: lead.attributedStringValue).width
            let nameWidth = TextMeasure.labelSize(of: label.attributedStringValue).width
            let avatarSize: CGFloat = 16
            let fixed = leadWidth + 6 + avatarSize + 5 + nameWidth
            let previewWidth = preview.isHidden ? 0 : min(
                Self.previewMaxWidth,
                TextMeasure.labelSize(of: preview.attributedStringValue).width,
                max(0, available - fixed - previewGap))
            let total = fixed + (previewWidth > 0 ? previewGap + previewWidth : 0)
            var x = (bounds.width - total) / 2
            markerRect = NSRect(x: x - 4, y: centerY - 11, width: total + 8, height: 22)
            lead.frame = NSRect(x: x, y: centerY - 8, width: leadWidth, height: 16)
            x += leadWidth + 6
            fromAvatar.frame = NSRect(x: x, y: centerY - avatarSize / 2, width: avatarSize, height: avatarSize)
            x += avatarSize + 5
            label.frame = NSRect(x: x, y: centerY - 8, width: nameWidth, height: 16)
            x += nameWidth + previewGap
            preview.frame = NSRect(x: x, y: centerY - 8, width: previewWidth, height: 16)
            window?.invalidateCursorRects(for: self)
            return
        }

        let x = ChatMetrics.bubbleIndent
        fromAvatar.frame = NSRect(x: x, y: centerY - 9, width: 18, height: 18)
        arrow.frame = NSRect(x: x + 22, y: centerY - 6, width: 12, height: 12)
        toAvatar.frame = NSRect(x: x + 38, y: centerY - 9, width: 18, height: 18)
        let textMax = max(0, bounds.width - x - 64 - ChatMetrics.horizontalInset)
        let nameWidth = min(textMax, TextMeasure.labelSize(of: label.attributedStringValue).width)
        label.frame = NSRect(x: x + 64, y: centerY - 8, width: nameWidth, height: 16)
        let previewWidth = preview.isHidden ? 0 : min(
            Self.previewMaxWidth,
            TextMeasure.labelSize(of: preview.attributedStringValue).width,
            max(0, textMax - nameWidth - previewGap))
        preview.frame = NSRect(x: x + 64 + nameWidth + previewGap, y: centerY - 8, width: previewWidth, height: 16)
        markerRect = NSRect(
            x: x - 4, y: centerY - 11,
            width: 64 + nameWidth + (previewWidth > 0 ? previewGap + previewWidth : 0) + 8, height: 22)
        window?.invalidateCursorRects(for: self)
    }

    override func resetCursorRects() {
        super.resetCursorRects()
        if !fullText.isEmpty { addCursorRect(markerRect, cursor: .pointingHand) }
    }

    override func mouseDown(with event: NSEvent) {
        let point = convert(event.locationInWindow, from: nil)
        guard !fullText.isEmpty, markerRect.contains(point) else { return super.mouseDown(with: event) }
        TextPopover.show(fullText, relativeTo: markerRect, of: self)
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


// MARK: - Permission card

/// A bot asking before a plugin tool runs (or before a plugin is installed): the question,
/// the call in one line, and Allow once / Always allow / Deny while it waits, then the answer.
final class PermissionCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("PermissionCell")

    static let width: CGFloat = 420

    /// A decided card is one line; a long answer (a sign-in failure with advice) gets two more,
    /// and a card showing a code to enter has a button row like a pending one. A pending card
    /// with Auto-review's reason has a second line above the buttons.
    static func height(pending: Bool, summary: String, hasCode: Bool = false, hasReason: Bool = false) -> CGFloat {
        if pending && hasReason { return 106 }
        if pending || hasCode { return 90 }
        return summary.count > 70 ? 86 : 58
    }

    private let box = BackgroundView()
    private let icon = NSImageView()
    private let title = Build.label("", font: .systemFont(ofSize: 12.5, weight: .semibold))
    private let summary = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 3)
    private let allowButton = NSButton()
    private let alwaysButton = NSButton()
    private let denyButton = NSButton()
    private let codeLabel = Build.label("", font: .monospacedSystemFont(ofSize: 15, weight: .semibold))
    private let openButton = NSButton()
    private var groupStart = true
    private var pending = true
    private var summaryText = ""
    private var hasCode = false
    private var hasReason = false
    private var link: String?
    private var code: String?

    var onDecision: ((String) -> Void)?

    init() {
        super.init(frame: .zero)
        box.cornerRadius = 12
        box.fillColor = Theme.botBubble
        box.borderColor = Theme.botBubbleBorder
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
        icon.contentTintColor = .controlAccentColor
        summary.lineBreakMode = .byWordWrapping
        for (button, label, decision) in [(allowButton, "Allow once", "allow"), (alwaysButton, "Always allow", "always"), (denyButton, "Deny", "deny")] {
            button.title = label
            button.bezelStyle = .rounded
            button.controlSize = .small
            button.font = .systemFont(ofSize: 11)
            button.target = self
            button.action = #selector(decide(_:))
            button.identifier = NSUserInterfaceItemIdentifier(decision)
        }
        addSubview(box.framePositioned())
        addSubview(icon.framePositioned())
        addSubview(title.framePositioned())
        addSubview(summary.framePositioned())
        addSubview(allowButton.framePositioned())
        addSubview(alwaysButton.framePositioned())
        addSubview(denyButton.framePositioned())
        codeLabel.isSelectable = true
        openButton.bezelStyle = .rounded
        openButton.controlSize = .small
        openButton.font = .systemFont(ofSize: 11)
        openButton.target = self
        openButton.action = #selector(openLink)
        addSubview(codeLabel.framePositioned())
        addSubview(openButton.framePositioned())
    }

    /// Copies the code and opens the sign-in page, so the code is one paste away.
    @objc private func openLink() {
        if let code {
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(code, forType: .string)
        }
        if let link, let url = URL(string: link) { NSWorkspace.shared.open(url) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(request: PermissionRequest, botName: String, groupStart: Bool) {
        self.groupStart = groupStart
        pending = request.isPending
        icon.image = NSImage(systemSymbolName: request.isConnect ? "person.crop.circle.badge.checkmark" : (request.isInstall ? "puzzlepiece.extension" : "hand.raised"), accessibilityDescription: nil)
        icon.contentTintColor = request.decision == .failed ? .systemRed : (request.decision == .connected ? .systemGreen : .controlAccentColor)
        title.stringValue = "\(botName) \(request.verbPhrase)"
        hasCode = request.decision == .allowed && request.code != nil
        link = request.link
        code = request.code
        hasReason = request.isPending && request.reason != nil
        summaryText = request.isPending ? request.summary : (hasCode ? "Enter this code at \(URL(string: request.link ?? "")?.host ?? "the link"), then come back." : "\(request.decisionText) · \(request.summary)")
        summary.stringValue = hasReason ? "\(summaryText)\nAuto-review: \(request.reason ?? "")" : summaryText
        summary.toolTip = request.summary
        codeLabel.stringValue = request.code ?? ""
        codeLabel.isHidden = !hasCode
        openButton.title = "Copy code and open \(URL(string: request.link ?? "")?.host ?? "link")"
        openButton.isHidden = !hasCode
        // The buttons follow the card's kind: Sign in / Not now, Allow / Deny, or the three.
        let buttons = [allowButton, alwaysButton, denyButton]
        for button in buttons { button.isHidden = true }
        if pending {
            for (button, choice) in zip(buttons, request.choices) {
                button.title = choice.0
                button.identifier = NSUserInterfaceItemIdentifier(choice.1)
                button.isHidden = false
            }
        }
        needsLayout = true
    }

    @objc private func decide(_ sender: NSButton) {
        guard let decision = sender.identifier?.rawValue else { return }
        onDecision?(decision)
    }

    override func layout() {
        super.layout()
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let width = min(Self.width, bounds.width - ChatMetrics.horizontalInset * 2)
        let x = ChatMetrics.horizontalInset
        // `height` is the box alone; the row adds `top` above it.
        let height = Self.height(pending: pending, summary: summaryText, hasCode: hasCode, hasReason: hasReason)
        box.frame = NSRect(x: x, y: top, width: width, height: height)
        icon.frame = NSRect(x: x + 12, y: top + 12, width: 18, height: 18)
        title.frame = NSRect(x: x + 38, y: top + 11, width: width - 50, height: 17)
        summary.frame = NSRect(x: x + 38, y: top + 30, width: width - 50, height: hasReason ? 32 : (pending || hasCode ? 16 : height - 30 - 10))
        let buttonY = top + (hasReason ? 70 : 54)
        if hasCode {
            let codeSize = codeLabel.intrinsicContentSize
            codeLabel.frame = NSRect(x: x + 38, y: top + 52, width: codeSize.width + 4, height: 24)
            let openSize = openButton.intrinsicContentSize
            openButton.frame = NSRect(x: x + 38 + codeSize.width + 14, y: top + 53, width: openSize.width + 4, height: 22)
        }
        var buttonX = x + 36
        for button in [allowButton, alwaysButton, denyButton] where !button.isHidden {
            let size = button.intrinsicContentSize
            button.frame = NSRect(x: buttonX, y: buttonY, width: size.width + 4, height: 22)
            buttonX += size.width + 12
        }
    }
}
