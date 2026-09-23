import AppKit

// MARK: - Segment stack

/// Lays out the parsed message body: text runs in selectable text views, tables as grids.
final class SegmentedTextView: NSView {
    private var textViews: [MarkdownTextView] = []
    private var tableViews: [MarkdownTableView] = []
    private var segments: [MessageSegment] = []
    private var textColor: NSColor = .labelColor

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(_ newSegments: [MessageSegment], textColor: NSColor) {
        segments = newSegments
        self.textColor = textColor
        syncViews()
        needsLayout = true
    }

    /// Text views are rebuilt per configure: their link color follows the text color, and a
    /// recycled cell can swap between a user and a bot bubble.
    private func syncViews() {
        for view in textViews { view.removeFromSuperview() }
        for view in tableViews { view.removeFromSuperview() }
        textViews = []
        tableViews = []
        for segment in segments {
            switch segment {
            case let .text(attributed, top, bottom):
                let view = MarkdownTextView(textColor: textColor)
                view.show(attributed, topInset: top, bottomInset: bottom)
                addSubview(view.framePositioned())
                textViews.append(view)
            case .table:
                let view = MarkdownTableView()
                addSubview(view.framePositioned())
                tableViews.append(view)
            }
        }
    }

    override func layout() {
        super.layout()
        var y: CGFloat = 0
        var texts = 0
        var tables = 0
        for (index, segment) in segments.enumerated() {
            switch segment {
            case let .text(attributed, top, bottom):
                let height = TextMeasure.textSize(of: attributed, width: bounds.width).height + top + bottom
                textViews[texts].frame = NSRect(x: 0, y: y, width: bounds.width, height: height)
                texts += 1
                y += height
            case let .table(content):
                let table = TableLayout.make(content, maxWidth: bounds.width)
                let view = tableViews[tables]
                view.show(table, textColor: textColor)
                view.frame = NSRect(x: 0, y: y, width: min(table.width, bounds.width), height: table.height)
                tables += 1
                y += table.height
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

        content.configure(segments, textColor: isUser ? Theme.userBubbleText : .labelColor)

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
            case 1: text = L("%@ is working…", names[0])
            default: text = L("%@ and %@ are working…", names.dropLast().joined(separator: L(", ")), names.last ?? "")
            }
        }
        label.stringValue = showsName || activity != nil ? text : ""
        setAccessibilityLabel(names.count == 1 ? L("%@ is working", names[0]) : text)
        needsLayout = true
    }

    /// What a tool row means while it runs, in the words of the status line. `pluginName` is
    /// the plugin behind a `<plugin>__<tool>` row, so the line reads "Using GitHub" between
    /// two calls as well as during one.
    static func activity(for tool: ToolInvocation, targetName: String?, pluginName: String? = nil) -> String {
        switch tool.name {
        case "read": return L("Reading a file")
        case "write", "edit": return L("Drafting a file")
        case "bash": return L("Running commands")
        case "web_search": return L("Searching the web")
        case "web_fetch": return L("Reading the web")
        case "grep", "find", "ls": return L("Searching files")
        case "message_bot": return targetName.map { L("Messaging %@", $0) } ?? L("Messaging another bot")
        case "list_teammates": return L("Checking the team")
        case "create_bot": return L("Creating a bot")
        case "edit_bot": return L("Updating a bot")
        case "remember": return L("Taking a note")
        case "search_plugins": return L("Searching plugins")
        case "install_plugin": return L("Installing a plugin")
        default:
            // A plugin tool: "Using GitHub", whether the call is running or just finished, so
            // a run of quick calls never flashes back to "Working" between them.
            if let pluginName {
                return L("Using %@", pluginName)
            }
            if tool.name.contains("__"), tool.summary.hasPrefix("Using ") {
                return String(tool.summary.dropLast(tool.summary.hasSuffix("…") ? 1 : 0))
            }
            return L("Working")
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
        lead.stringValue = L("Message from")

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
            if case .incoming = mode { verb = L("Message from") } else { verb = L("Messaged") }
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
            label.stringValue = L("%@ handed off to %@", from?.name ?? "?", to?.name ?? "?")
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

/// A bot asking before a plugin tool runs, a shell command runs, or a plugin is installed. While
/// it waits: the question, the call (a shell command in a code block that opens the whole command
/// on click), why Auto-review paused it, the answers, and under them the rule Always allow adds.
/// Once answered, the answer and the call; an Always allow keeps the rule it added.
final class PermissionCellView: NSTableCellView {
    static let identifier = NSUserInterfaceItemIdentifier("PermissionCell")

    static let width: CGFloat = 440

    /// The box's height at `rowWidth`. The table's row height and the cell's own layout come
    /// from the same `Layout`, so a card is exactly as tall as what it shows.
    static func height(for request: PermissionRequest, rowWidth: CGFloat) -> CGFloat {
        Layout(request: request, rowWidth: rowWidth).height
    }

    /// What a card says about its rule: the one Always allow would add, or the one it added.
    static func ruleNote(for request: PermissionRequest) -> String? {
        guard let rule = request.rule else { return nil }
        if request.isPending { return L("Always allow adds the rule “%@”.", rule) }
        return request.decision == .always ? L("Added the rule “%@” to Auto-review.", rule) : nil
    }

    /// Where each part of a card sits, relative to the box.
    @MainActor private struct Layout {
        static let textX: CGFloat = 38
        static let titleFont = NSFont.systemFont(ofSize: 12.5, weight: .semibold)
        static let commandFont = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)
        static let reasonFont = NSFont.systemFont(ofSize: 11.5)
        static let noteFont = NSFont.systemFont(ofSize: 11)
        /// The card shows the start of a long command; a click opens all of it.
        static let commandLines = 2
        static let commandPadX: CGFloat = 8
        static let commandPadY: CGFloat = 5

        var width: CGFloat
        var height: CGFloat = 0
        var summary = NSRect.zero
        var command: NSRect?
        var reason: NSRect?
        var buttonY: CGFloat?
        var note: NSRect?

        init(request: PermissionRequest, rowWidth: CGFloat) {
            width = min(PermissionCellView.width, rowWidth - ChatMetrics.horizontalInset * 2)
            let textWidth = width - Self.textX - 12
            if request.decision == .allowed && request.code != nil {
                // Signing in with a code: the step, then the code and its button on one row.
                summary = NSRect(x: Self.textX, y: 30, width: textWidth, height: 16)
                height = 90
                return
            }
            var bottom: CGFloat
            if request.isPending && request.isShell {
                let lines = Self.measure(request.fullCommand, font: Self.commandFont, width: textWidth - Self.commandPadX * 2, maxLines: Self.commandLines)
                let block = NSRect(x: Self.textX, y: 35, width: textWidth, height: lines + Self.commandPadY * 2)
                command = block
                bottom = block.maxY
            } else {
                let text = request.isPending ? request.summary : "\(request.decisionText) · \(request.summary)"
                let lines = Self.measure(text, font: Theme.Font.caption, width: textWidth, maxLines: request.isPending ? 2 : 3)
                summary = NSRect(x: Self.textX, y: 30, width: textWidth, height: lines)
                bottom = summary.maxY
            }
            if request.isPending, let text = request.reason {
                let lines = Self.measure(text, font: Self.reasonFont, width: textWidth)
                let row = NSRect(x: Self.textX, y: bottom + 7, width: textWidth, height: lines)
                reason = row
                bottom = row.maxY
            }
            if request.isPending {
                buttonY = bottom + 10
                bottom += 10 + 22
            }
            if let text = PermissionCellView.ruleNote(for: request) {
                let lines = Self.measure(text, font: Self.noteFont, width: textWidth)
                let row = NSRect(x: Self.textX, y: bottom + (request.isPending ? 8 : 5), width: textWidth, height: lines)
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

    private let box = BackgroundView()
    private let icon = NSImageView()
    private let title = Build.label("", font: Layout.titleFont)
    private let summary = Build.label("", font: Theme.Font.caption, color: .secondaryLabelColor, lines: 3)
    private let command = CommandBlockView(font: Layout.commandFont, lines: Layout.commandLines, padding: NSSize(width: Layout.commandPadX, height: Layout.commandPadY))
    private let reason = Build.label("", font: Layout.reasonFont, color: .secondaryLabelColor, lines: 0)
    private let note = Build.label("", font: Layout.noteFont, color: .secondaryLabelColor, lines: 0)
    private let allowButton = NSButton()
    private let alwaysButton = NSButton()
    private let denyButton = NSButton()
    private let codeLabel = Build.label("", font: .monospacedSystemFont(ofSize: 15, weight: .semibold))
    private let openButton = CopyFeedbackButton()
    private var groupStart = true
    private var request: PermissionRequest?
    private var link: String?
    private var code: String?

    var onDecision: ((String) -> Void)?
    /// The command block was clicked: show the whole command.
    var onShowCommand: (() -> Void)?

    init() {
        super.init(frame: .zero)
        box.cornerRadius = 12
        box.fillColor = Theme.botBubble
        box.borderColor = Theme.botBubbleBorder
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 14, weight: .medium)
        icon.contentTintColor = .controlAccentColor
        summary.lineBreakMode = .byWordWrapping
        command.onClick = { [weak self] in self?.onShowCommand?() }
        for (button, label, decision) in [(allowButton, L("Allow once"), "allow"), (alwaysButton, L("Always allow"), "always"), (denyButton, L("Deny"), "deny")] {
            button.title = label
            button.bezelStyle = .rounded
            button.controlSize = .small
            button.font = .systemFont(ofSize: 11)
            button.target = self
            button.action = #selector(decide(_:))
            button.identifier = NSUserInterfaceItemIdentifier(decision)
        }
        for view in [box, icon, title, summary, command, reason, note, allowButton, alwaysButton, denyButton] as [NSView] {
            addSubview(view.framePositioned())
        }
        codeLabel.isSelectable = true
        openButton.bezelStyle = .rounded
        openButton.controlSize = .small
        openButton.font = .systemFont(ofSize: 11)
        openButton.target = self
        openButton.action = #selector(openLink(_:))
        addSubview(codeLabel.framePositioned())
        addSubview(openButton.framePositioned())
    }

    /// Copies the code and opens the sign-in page, so the code is one paste away.
    @objc private func openLink(_ sender: CopyFeedbackButton) {
        if let code {
            NSPasteboard.general.clearContents()
            if NSPasteboard.general.setString(code, forType: .string) {
                sender.showCopied()
            }
        }
        if let link, let url = URL(string: link) { NSWorkspace.shared.open(url) }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    func configure(request: PermissionRequest, botName: String, groupStart: Bool) {
        openButton.resetCopyFeedback()
        self.groupStart = groupStart
        self.request = request
        let hasCode = request.decision == .allowed && request.code != nil
        icon.image = NSImage(systemSymbolName: request.isConnect ? "person.crop.circle.badge.checkmark" : (request.isInstall ? "puzzlepiece.extension" : "hand.raised"), accessibilityDescription: nil)
        icon.contentTintColor = request.decision == .failed ? .systemRed : (request.decision == .connected ? .systemGreen : .controlAccentColor)
        title.stringValue = "\(botName) \(request.verbPhrase)"
        title.toolTip = title.stringValue
        link = request.link
        code = request.code
        summary.stringValue = request.isPending
            ? request.summary
            : (hasCode ? L("Enter this code at %@, then come back.", URL(string: request.link ?? "")?.host ?? L("the link")) : "\(request.decisionText) · \(request.summary)")
        summary.lineBreakMode = request.isPending ? .byTruncatingTail : .byWordWrapping
        summary.toolTip = request.summary
        command.text = request.fullCommand
        reason.stringValue = request.reason ?? ""
        note.stringValue = Self.ruleNote(for: request) ?? ""
        codeLabel.stringValue = request.code ?? ""
        codeLabel.isHidden = !hasCode
        openButton.title = L("Copy code and open %@", URL(string: request.link ?? "")?.host ?? L("link"))
        openButton.isHidden = !hasCode
        // The buttons follow the card's kind: Sign in / Not now, Allow / Deny, or the three.
        let buttons = [allowButton, alwaysButton, denyButton]
        for button in buttons { button.isHidden = true }
        if request.isPending {
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
        guard let request else { return }
        let top = groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let x = ChatMetrics.horizontalInset
        let layout = Layout(request: request, rowWidth: bounds.width)
        func place(_ rect: NSRect) -> NSRect { rect.offsetBy(dx: x, dy: top) }

        // `layout.height` is the box alone; the row adds `top` above it.
        box.frame = NSRect(x: x, y: top, width: layout.width, height: layout.height)
        icon.frame = NSRect(x: x + 12, y: top + 12, width: 18, height: 18)
        title.frame = place(NSRect(x: Layout.textX, y: 11, width: layout.width - Layout.textX - 12, height: 17))
        summary.isHidden = layout.command != nil
        summary.frame = place(layout.summary)
        command.isHidden = layout.command == nil
        command.frame = layout.command.map(place) ?? .zero
        reason.isHidden = layout.reason == nil
        reason.frame = layout.reason.map(place) ?? .zero
        note.isHidden = layout.note == nil
        note.frame = layout.note.map(place) ?? .zero
        if !codeLabel.isHidden {
            let codeSize = codeLabel.intrinsicContentSize
            codeLabel.frame = NSRect(x: x + 38, y: top + 52, width: codeSize.width + 4, height: 24)
            let openSize = openButton.intrinsicContentSize
            openButton.frame = NSRect(x: x + 38 + codeSize.width + 14, y: top + 53, width: openSize.width + 4, height: 22)
        }
        if let buttonY = layout.buttonY {
            var buttonX = x + 36
            for button in [allowButton, alwaysButton, denyButton] where !button.isHidden {
                let size = button.intrinsicContentSize
                button.frame = NSRect(x: buttonX, y: top + buttonY, width: size.width + 4, height: 22)
                buttonX += size.width + 12
            }
        }
    }
}

/// A shell command on a permission card: its first lines in a code block that shows the whole
/// command on click, highlighting under the pointer like a button.
final class CommandBlockView: NSView {
    var onClick: (() -> Void)?
    var text: String {
        get { label.stringValue }
        set { label.stringValue = newValue }
    }

    private let background = BackgroundView()
    private let label: NSTextField
    private let padding: NSSize
    private var hovering = false { didSet { background.fillColor = hovering ? Theme.codeBackgroundHover : Theme.codeBackground } }

    init(font: NSFont, lines: Int, padding: NSSize) {
        self.padding = padding
        label = Build.label("", font: font, lines: lines)
        super.init(frame: .zero)
        label.lineBreakMode = .byWordWrapping
        label.cell?.truncatesLastVisibleLine = true
        background.fillColor = Theme.codeBackground
        background.cornerRadius = 6
        addSubview(background.framePositioned())
        addSubview(label.framePositioned())
        toolTip = L("Show the full command")
        setAccessibilityElement(true)
        setAccessibilityRole(.button)
        setAccessibilityLabel(L("Show the full command"))
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    override func layout() {
        super.layout()
        background.frame = bounds
        label.frame = bounds.insetBy(dx: padding.width, dy: padding.height)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        for area in trackingAreas { removeTrackingArea(area) }
        addTrackingArea(NSTrackingArea(rect: .zero, options: [.mouseEnteredAndExited, .activeInKeyWindow, .inVisibleRect], owner: self))
    }

    override func mouseEntered(with event: NSEvent) { hovering = true }
    override func mouseExited(with event: NSEvent) { hovering = false }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }

    override func hitTest(_ point: NSPoint) -> NSView? {
        frame.contains(point) ? self : nil
    }

    override func mouseDown(with event: NSEvent) {}

    override func mouseUp(with event: NSEvent) {
        if bounds.contains(convert(event.locationInWindow, from: nil)) { onClick?() }
    }

    override func accessibilityPerformPress() -> Bool {
        onClick?()
        return true
    }
}
