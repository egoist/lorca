import AppKit

/// A published output in the transcript, standing without a bubble or a time: an image's
/// preview, the file's icon, name, type and size, and what the bot checked. A click opens the
/// output's sheet, or retries a file that could not be fetched; the menu opens or saves the
/// file, and replies to it. In a group the bot's avatar sits beside its bottom edge. A run of one
/// bot's outputs joins into one list, a compact row each: the icon (an image's own picture), name,
/// type and size, and the check's symbol; the check's words are in the output's sheet.
final class OutputCellView: TranscriptCellView {
    static let identifier = NSUserInterfaceItemIdentifier("OutputCell")

    static let width: CGFloat = 320
    /// A row's height in a run of outputs.
    static let stackedHeight: CGFloat = 44

    /// The card's height at `rowWidth`, starting at `indent`. The table's row height and the
    /// cell's own layout come from the same `Layout`, so a card is exactly as tall as what it shows.
    static func height(for message: Message, rowWidth: CGFloat, indent: CGFloat) -> CGFloat {
        guard let output = message.output else { return 0 }
        return Layout(output: output, attachment: message.attachments.first, rowWidth: rowWidth, indent: indent).height
    }

    /// For a screen reader: the file, and how its check went.
    static func spokenText(output: Output) -> String {
        guard let evidence = output.evidence else { return output.name }
        return "\(output.name). \(evidence.title) · \(evidence.statusText): \(evidence.summary)"
    }

    /// "HTML · 18 KB", or the link's host.
    static func detail(output: Output, attachment: Attachment?) -> String {
        [output.typeName, attachment.map { Attachment.sizeText($0.size) } ?? ""]
            .filter { !$0.isEmpty }
            .joined(separator: " · ")
    }

    /// Where each part of a card sits, relative to the card.
    @MainActor private struct Layout {
        static let pad: CGFloat = 12
        static let iconSize: CGFloat = 26
        static let textX: CGFloat = pad + iconSize + 10
        static let stackedIconSize: CGFloat = 24
        static let stackedTextX: CGFloat = pad + stackedIconSize + 10
        static let headerHeight: CGFloat = 54
        static let checkIconSize: CGFloat = 15
        static let checkX: CGFloat = pad + checkIconSize + 8
        static let previewMax: CGFloat = 200
        static let previewMin: CGFloat = 96
        static let nameFont = NSFont.systemFont(ofSize: 13, weight: .semibold)
        static let detailFont = NSFont.systemFont(ofSize: 11.5)
        static let checkTitleFont = NSFont.systemFont(ofSize: 12, weight: .semibold)
        static let summaryFont = NSFont.systemFont(ofSize: 12)
        static let commandFont = NSFont.monospacedSystemFont(ofSize: 11, weight: .regular)

        var width: CGFloat
        var height: CGFloat = 0
        var preview: NSRect?
        var headerY: CGFloat = 0
        var separatorY: CGFloat?
        var checkTitle = NSRect.zero
        var summary = NSRect.zero
        var command = NSRect.zero

        init(output: Output, attachment: Attachment?, rowWidth: CGFloat, indent: CGFloat) {
            width = min(OutputCellView.width, rowWidth - indent - ChatMetrics.horizontalInset)
            // An image keeps its box while its bytes are fetched or could not be, so the
            // transcript never moves.
            if output.kind == .image, let attachment {
                let full = Self.fullHeight(of: attachment, width: width)
                preview = NSRect(x: 0, y: 0, width: width, height: min(max(full, Self.previewMin), Self.previewMax))
            }
            headerY = preview?.maxY ?? 0
            var bottom = headerY + Self.headerHeight
            if let evidence = output.evidence {
                separatorY = bottom
                let textWidth = width - Self.checkX - Self.pad
                checkTitle = NSRect(x: Self.checkX, y: bottom + 9, width: textWidth, height: Self.measure("X", font: Self.checkTitleFont, width: textWidth))
                bottom = checkTitle.maxY
                let summaryText = evidence.summary.trimmingCharacters(in: .whitespacesAndNewlines)
                if !summaryText.isEmpty {
                    summary = NSRect(x: Self.checkX, y: bottom + 3, width: textWidth, height: Self.measure(summaryText, font: Self.summaryFont, width: textWidth))
                    bottom = summary.maxY
                }
                if let command = evidence.command, !command.isEmpty {
                    self.command = NSRect(x: Self.checkX, y: bottom + 3, width: textWidth, height: Self.measure("X", font: Self.commandFont, width: textWidth))
                    bottom = self.command.maxY
                }
                bottom += 10
            }
            height = bottom
        }

        /// The picture's height at the card's width.
        static func fullHeight(of attachment: Attachment, width: CGFloat) -> CGFloat {
            let pixels = attachment.width.flatMap { w in attachment.height.map { (w, $0) } } ?? (4, 3)
            return width * CGFloat(max(pixels.1, 1)) / CGFloat(max(pixels.0, 1))
        }

        static func measure(_ text: String, font: NSFont, width: CGFloat) -> CGFloat {
            TextMeasure.labelSize(of: NSAttributedString(string: text, attributes: [.font: font]), width: width).height
        }
    }

    private let avatar = AvatarView(diameter: ChatMetrics.avatarSize)
    private let card = BubbleView()
    private let stackShape = StackShapeView()
    private let preview = OutputPreviewView()
    private let previewRule = BackgroundView()
    private let outline = OutlineView()
    private let icon = NSImageView()
    private let name = Build.label("", font: Layout.nameFont)
    private let detail = Build.label("", font: Layout.detailFont, color: .secondaryLabelColor)
    private let separator = BackgroundView()
    private let checkIcon = NSImageView()
    private let checkTitle = Build.label("", font: Layout.checkTitleFont)
    private let summary = Build.label("", font: Layout.summaryFont, color: .secondaryLabelColor, lines: 0)
    private let command = Build.label("", font: Layout.commandFont, color: .tertiaryLabelColor)
    private var output: Output?
    private var attachment: Attachment?
    private var failed = false
    private var groupStart = true
    private var stack = OutputStack()
    /// In a group every row keeps the avatar's room, though only a run's last row shows it.
    private var hasAvatar = false

    /// A click on the card: the output's sheet.
    var onOpen: (() -> Void)?
    /// A click on a card whose file could not be fetched.
    var onRetry: (() -> Void)?
    /// Open and Save As… (or Open Link and Copy Link), on a right-click.
    var fileMenu: (() -> NSMenu)?
    var onReply: (() -> Void)?
    var onFeedback: (() -> Void)?

    init() {
        super.init(frame: .zero)
        card.cornerRadius = 12
        card.fillColor = Theme.outputCard
        card.layer?.cornerRadius = 12
        card.layer?.masksToBounds = true
        // A tall picture draws past the box; a view no longer clips its drawing by default.
        preview.clipsToBounds = true
        preview.wantsLayer = true
        preview.layer?.masksToBounds = true
        previewRule.cornerRadius = 0
        previewRule.fillColor = .separatorColor
        separator.cornerRadius = 0
        separator.fillColor = .separatorColor
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 19, weight: .regular)
        checkIcon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 13, weight: .medium)
        name.lineBreakMode = .byTruncatingMiddle
        detail.lineBreakMode = .byTruncatingTail
        summary.lineBreakMode = .byWordWrapping
        command.lineBreakMode = .byTruncatingTail
        card.addSubview(stackShape.framePositioned())
        for view in [preview, previewRule, icon, name, detail, separator, checkIcon, checkTitle, summary, command, outline] as [NSView] {
            card.addSubview(view.framePositioned())
        }
        addSubview(avatar.framePositioned())
        addSubview(card.framePositioned())
        card.addGestureRecognizer(NSClickGestureRecognizer(target: self, action: #selector(click)))
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    /// `avatar` is the bot's in a group, nil in a DM. `url` is the file's bytes on this Mac, nil
    /// until they are fetched; `error` is why they could not be.
    func configure(
        output: Output, attachment: Attachment?, url: URL?, error: String?, avatar avatarContent: AvatarView.Content?,
        groupStart: Bool, stack: OutputStack = OutputStack()
    ) {
        self.output = output
        self.attachment = attachment
        self.groupStart = groupStart
        self.stack = stack
        failed = error != nil
        // In a run, the avatar sits beside the last row.
        hasAvatar = avatarContent != nil
        avatar.isHidden = !hasAvatar || stack.below
        if let avatarContent { avatar.content = avatarContent }
        // A run's rows draw the list's shape themselves, rounded only at its ends.
        card.fillColor = stack.isStacked ? .clear : Theme.outputCard
        card.layer?.cornerRadius = stack.isStacked ? 0 : 12
        stackShape.stack = stack
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: stack.isStacked ? 15 : 19, weight: .regular)
        icon.image = NSImage(systemSymbolName: output.symbolName, accessibilityDescription: nil)
        icon.contentTintColor = output.tint
        name.stringValue = output.name
        if let error {
            detail.stringValue = L("Couldn't download · Retry")
            detail.textColor = .systemRed
            card.toolTip = "\(output.name): \(error)"
        } else {
            detail.stringValue = Self.detail(output: output, attachment: attachment)
            detail.textColor = .secondaryLabelColor
            card.toolTip = url == nil && attachment != nil ? L("%@ · fetching…", output.name) : output.name
        }
        if let evidence = output.evidence, stack.isStacked, error == nil {
            card.toolTip = "\(output.name)\n\(evidence.title) · \(evidence.statusText)"
        }
        if output.kind == .image, let attachment, error == nil, let url {
            // Sharp at the card's widest, on a Retina screen: the longest side the picture
            // takes at that width, doubled. In a run, it stands in for the icon.
            let full = Layout.fullHeight(of: attachment, width: Self.width)
            let side = stack.isStacked ? 128 : Int(min(4096, ceil(max(Self.width, full) * 2)))
            preview.image = Thumbnails.image(at: url, maxPixels: side)
        } else {
            preview.image = nil
        }
        if let evidence = output.evidence {
            let (symbol, tint): (String, NSColor) = switch evidence.status {
            case "passed": ("checkmark.circle.fill", .systemGreen)
            case "failed": ("xmark.circle.fill", .systemRed)
            default: ("circle.dashed", .secondaryLabelColor)
            }
            checkIcon.image = NSImage(systemSymbolName: symbol, accessibilityDescription: nil)
            checkIcon.contentTintColor = tint
            checkTitle.stringValue = "\(evidence.title) · \(evidence.statusText)"
            checkTitle.textColor = tint
            summary.stringValue = evidence.summary.trimmingCharacters(in: .whitespacesAndNewlines)
            command.stringValue = evidence.command ?? ""
            command.toolTip = evidence.command
        }
        needsLayout = true
    }

    @objc private func click() {
        failed ? onRetry?() : onOpen?()
    }

    override func menu(for event: NSEvent) -> NSMenu? {
        let menu = NSMenu()
        if failed {
            menu.addItem(ClosureMenuItem(L("Try Again")) { [weak self] in self?.onRetry?() })
        } else if let fileMenu {
            for item in fileMenu().items {
                item.menu?.removeItem(item)
                menu.addItem(item)
            }
        }
        if let onReply {
            if !menu.items.isEmpty { menu.addItem(.separator()) }
            menu.addItem(ClosureMenuItem(L("Reply"), handler: onReply))
            if let onFeedback { menu.addItem(ClosureMenuItem(L("Give Feedback…"), handler: onFeedback)) }
        }
        return menu.items.isEmpty ? nil : menu
    }

    func flash() {
        let pulse = CAKeyframeAnimation(keyPath: "opacity")
        pulse.values = [1, 0.35, 1, 0.35, 1]
        pulse.duration = 1.1
        card.layer?.add(pulse, forKey: "flash")
    }

    override func layout() {
        super.layout()
        guard let output else { return }
        let top = stack.above ? 0 : groupStart ? ChatMetrics.groupTopPadding : ChatMetrics.tightTopPadding
        let x = ChatMetrics.indent(showsAvatar: hasAvatar)
        if stack.isStacked {
            layoutStacked(output: output, top: top, x: x)
            return
        }
        let layout = Layout(output: output, attachment: attachment, rowWidth: bounds.width, indent: x)

        card.frame = NSRect(x: x, y: top, width: layout.width, height: layout.height)
        stackShape.isHidden = true
        outline.isHidden = false
        outline.frame = card.bounds
        avatar.frame = NSRect(
            x: ChatMetrics.horizontalInset, y: top + layout.height - ChatMetrics.avatarSize,
            width: ChatMetrics.avatarSize, height: ChatMetrics.avatarSize)

        preview.isHidden = layout.preview == nil
        previewRule.isHidden = layout.preview == nil
        preview.frame = layout.preview ?? .zero
        preview.layer?.cornerRadius = 0
        previewRule.frame = NSRect(x: 0, y: layout.headerY - 1, width: layout.width, height: 1)

        let textWidth = layout.width - Layout.textX - Layout.pad
        icon.isHidden = false
        icon.frame = NSRect(
            x: Layout.pad, y: layout.headerY + (Layout.headerHeight - Layout.iconSize) / 2,
            width: Layout.iconSize, height: Layout.iconSize)
        name.frame = NSRect(x: Layout.textX, y: layout.headerY + 10, width: textWidth, height: 17)
        detail.frame = NSRect(x: Layout.textX, y: layout.headerY + 28, width: textWidth, height: 15)

        let hasCheck = layout.separatorY != nil
        for view in [separator, checkIcon, checkTitle, summary, command] as [NSView] { view.isHidden = !hasCheck }
        guard let separatorY = layout.separatorY else { return }
        separator.frame = NSRect(x: 0, y: separatorY, width: layout.width, height: 1)
        checkIcon.frame = NSRect(
            x: Layout.pad, y: layout.checkTitle.midY - Layout.checkIconSize / 2,
            width: Layout.checkIconSize, height: Layout.checkIconSize)
        checkTitle.frame = layout.checkTitle
        summary.frame = layout.summary
        summary.isHidden = layout.summary.height == 0
        command.frame = layout.command
        command.isHidden = layout.command.height == 0
    }

    /// A row in a run of outputs: the icon, name, type and size, and the check's symbol at the end.
    private func layoutStacked(output: Output, top: CGFloat, x: CGFloat) {
        let width = min(Self.width, bounds.width - x - ChatMetrics.horizontalInset)
        let height = Self.stackedHeight
        card.frame = NSRect(x: x, y: top, width: width, height: height)
        stackShape.isHidden = false
        stackShape.frame = card.bounds
        stackShape.separatorX = Layout.stackedTextX
        outline.isHidden = true
        avatar.frame = NSRect(
            x: ChatMetrics.horizontalInset, y: top + height - ChatMetrics.avatarSize,
            width: ChatMetrics.avatarSize, height: ChatMetrics.avatarSize)
        for view in [previewRule, separator, checkTitle, summary, command] as [NSView] { view.isHidden = true }

        let hasCheck = output.evidence != nil
        checkIcon.isHidden = !hasCheck
        let checkX = width - Layout.pad - Layout.checkIconSize
        checkIcon.frame = NSRect(x: checkX, y: (height - Layout.checkIconSize) / 2, width: Layout.checkIconSize, height: Layout.checkIconSize)
        let textWidth = (hasCheck ? checkX - 8 : width - Layout.pad) - Layout.stackedTextX
        let iconFrame = NSRect(
            x: Layout.pad, y: (height - Layout.stackedIconSize) / 2,
            width: Layout.stackedIconSize, height: Layout.stackedIconSize)
        // An image's own picture, once its bytes are here, in place of the icon.
        let thumbnail = preview.image != nil
        preview.isHidden = !thumbnail
        preview.frame = iconFrame
        preview.layer?.cornerRadius = 5
        icon.isHidden = thumbnail
        icon.frame = iconFrame
        name.frame = NSRect(x: Layout.stackedTextX, y: 6, width: textWidth, height: 17)
        detail.frame = NSRect(x: Layout.stackedTextX, y: 23, width: textWidth, height: 15)
    }
}

/// An image output's picture across the top of its card: the whole picture when it is wide, the
/// top of it when it is tall, as a page's screenshot starts at its top. A quiet box until the
/// bytes are here.
private final class OutputPreviewView: NSView {
    var image: NSImage? { didSet { needsDisplay = true } }

    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        Theme.codeBackground.setFill()
        bounds.fill()
        guard let image, image.size.width > 0, image.size.height > 0 else { return }
        let full = bounds.width * image.size.height / image.size.width
        let rect: NSRect
        if full >= bounds.height {
            rect = NSRect(x: 0, y: 0, width: bounds.width, height: full)
        } else {
            let scale = max(bounds.width / image.size.width, bounds.height / image.size.height)
            let size = NSSize(width: image.size.width * scale, height: image.size.height * scale)
            rect = NSRect(x: (bounds.width - size.width) / 2, y: (bounds.height - size.height) / 2, width: size.width, height: size.height)
        }
        image.draw(in: rect, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: [.interpolation: NSImageInterpolation.high.rawValue])
    }
}

/// A row's part of a run's list: the card's fill and hairline, rounded at the run's ends, open
/// where the row joins the next, and a rule between rows from the text's edge.
private final class StackShapeView: NSView {
    var stack = OutputStack() { didSet { needsDisplay = true } }
    var separatorX: CGFloat = 0 { didSet { needsDisplay = true } }

    override init(frame: NSRect) {
        super.init(frame: frame)
        clipsToBounds = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        // A joined edge runs past the view, corners and all, so only the outer ends draw them.
        var rect = bounds.insetBy(dx: 0.5, dy: 0.5)
        let past: CGFloat = 14
        if stack.above { rect.origin.y -= past; rect.size.height += past }
        if stack.below { rect.size.height += past }
        let path = NSBezierPath(roundedRect: rect, xRadius: 11.5, yRadius: 11.5)
        Theme.outputCard.setFill()
        path.fill()
        NSColor.separatorColor.setStroke()
        path.lineWidth = 1
        path.stroke()
        if stack.above {
            NSColor.separatorColor.setFill()
            NSRect(x: separatorX, y: 0, width: bounds.width - separatorX, height: 1).fill()
        }
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}

/// The card's hairline, drawn over the preview so the picture's top edge keeps the border.
private final class OutlineView: NSView {
    override var isFlipped: Bool { true }

    override func draw(_ dirtyRect: NSRect) {
        let path = NSBezierPath(roundedRect: bounds.insetBy(dx: 0.5, dy: 0.5), xRadius: 11.5, yRadius: 11.5)
        NSColor.separatorColor.setStroke()
        path.lineWidth = 1
        path.stroke()
    }

    override func hitTest(_ point: NSPoint) -> NSView? { nil }
}
