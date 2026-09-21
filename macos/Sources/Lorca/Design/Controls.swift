import AppKit

enum Build {
    static func label(
        _ text: String,
        font: NSFont,
        color: NSColor = .labelColor,
        lines: Int = 1,
        alignment: NSTextAlignment = .natural
    ) -> NSTextField {
        let field = NSTextField(labelWithString: text)
        field.font = font
        field.textColor = color
        field.maximumNumberOfLines = lines
        field.lineBreakMode = lines == 1 ? .byTruncatingTail : .byWordWrapping
        field.alignment = alignment
        field.translatesAutoresizingMaskIntoConstraints = false
        field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
        return field
    }

    static func stack(
        _ views: [NSView],
        orientation: NSUserInterfaceLayoutOrientation = .vertical,
        spacing: CGFloat = 8,
        alignment: NSLayoutConstraint.Attribute? = nil
    ) -> NSStackView {
        let stack = NSStackView(views: views)
        stack.orientation = orientation
        stack.spacing = spacing
        stack.translatesAutoresizingMaskIntoConstraints = false
        if let alignment { stack.alignment = alignment }
        else { stack.alignment = orientation == .vertical ? .leading : .centerY }
        return stack
    }

    static func imageButton(
        symbol: String,
        pointSize: CGFloat = 13,
        tooltip: String,
        target: AnyObject?,
        action: Selector
    ) -> NSButton {
        let button = NSButton()
        button.image = NSImage(systemSymbolName: symbol, accessibilityDescription: tooltip)
        button.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: pointSize, weight: .medium)
        button.bezelStyle = .accessoryBarAction
        button.isBordered = false
        button.toolTip = tooltip
        button.target = target
        button.action = action
        button.translatesAutoresizingMaskIntoConstraints = false
        return button
    }

    static func pill(_ text: String, color: NSColor) -> NSView {
        let container = BackgroundView()
        container.fillColor = color.withAlphaComponent(0.14)
        container.cornerRadius = 5
        let label = label(text, font: .systemFont(ofSize: 10.5, weight: .medium), color: color)
        container.addSubview(label)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 6),
            label.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -6),
            label.topAnchor.constraint(equalTo: container.topAnchor, constant: 2),
            label.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -2),
        ])
        return container
    }
}

/// A button that briefly confirms a successful clipboard write without needing a separate
/// status label beside every place the app offers Copy.
final class CopyFeedbackButton: NSButton {
    private struct Appearance {
        let title: String
        let image: NSImage?
        let imagePosition: NSControl.ImagePosition
        let tint: NSColor?
        let toolTip: String?
    }

    private var normalAppearance: Appearance?
    private var resetWork: DispatchWorkItem?

    func showCopied() {
        resetCopyFeedback()
        normalAppearance = Appearance(
            title: title,
            image: image,
            imagePosition: imagePosition,
            tint: contentTintColor,
            toolTip: toolTip
        )

        let copied = L("Copied")
        image = NSImage(systemSymbolName: "checkmark", accessibilityDescription: copied)
        if title.isEmpty {
            imagePosition = .imageOnly
        } else {
            title = copied
            imagePosition = .imageLeading
        }
        contentTintColor = .systemGreen
        toolTip = copied

        let work = DispatchWorkItem { [weak self] in
            self?.resetCopyFeedback()
        }
        resetWork = work
        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5, execute: work)
    }

    /// Cell reuse and other state changes can restore the ordinary label immediately.
    func resetCopyFeedback() {
        resetWork?.cancel()
        resetWork = nil
        guard let normalAppearance else { return }
        title = normalAppearance.title
        image = normalAppearance.image
        imagePosition = normalAppearance.imagePosition
        contentTintColor = normalAppearance.tint
        toolTip = normalAppearance.toolTip
        self.normalAppearance = nil
    }
}

/// Layer-backed rectangle with an optional border; used for bubbles, chips and cards.
class BackgroundView: NSView {
    var fillColor: NSColor = .clear { didSet { needsDisplay = true } }
    var borderColor: NSColor? { didSet { needsDisplay = true } }
    var borderWidth: CGFloat = 1 { didSet { needsDisplay = true } }
    var cornerRadius: CGFloat = 8 { didSet { needsDisplay = true } }

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var allowsVibrancy: Bool { false }

    override func draw(_ dirtyRect: NSRect) {
        let inset = borderColor == nil ? 0 : borderWidth / 2
        let path = NSBezierPath(
            roundedRect: bounds.insetBy(dx: inset, dy: inset),
            xRadius: cornerRadius,
            yRadius: cornerRadius
        )
        fillColor.setFill()
        path.fill()
        if let borderColor {
            borderColor.setStroke()
            path.lineWidth = borderWidth
            path.stroke()
        }
    }
}

/// Chat bubble: same drawing as `BackgroundView`, but y grows downward so
/// the header and body can share one layout with the flipped table cell.
final class BubbleView: BackgroundView {
    override var isFlipped: Bool { true }

    override init() {
        super.init()
        layer?.masksToBounds = true
    }
}

/// Plain container whose y grows downward, for scroll views whose content is shorter than the pane.
final class FlippedView: NSView {
    override var isFlipped: Bool { true }
}

/// One-pixel rule that stays crisp on Retina and follows the appearance.
final class HairlineView: NSView {
    var color: NSColor = .separatorColor { didSet { needsDisplay = true } }
    private let axis: NSUserInterfaceLayoutOrientation

    init(axis: NSUserInterfaceLayoutOrientation = .horizontal) {
        self.axis = axis
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        let thickness: CGFloat = 1
        if axis == .horizontal {
            heightAnchor.constraint(equalToConstant: thickness).isActive = true
        } else {
            widthAnchor.constraint(equalToConstant: thickness).isActive = true
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var allowsVibrancy: Bool { false }

    override func draw(_ dirtyRect: NSRect) {
        color.setFill()
        let scale = window?.backingScaleFactor ?? 2
        let thin = 1 / scale
        let rect =
            axis == .horizontal
            ? NSRect(x: 0, y: bounds.height - thin, width: bounds.width, height: thin)
            : NSRect(x: 0, y: 0, width: thin, height: bounds.height)
        rect.fill()
    }
}

/// Small colored dot for presence.
final class StatusDotView: NSView {
    var status: Device.Status = .offline { didSet { needsDisplay = true } }
    private let size: CGFloat

    init(size: CGFloat = 7) {
        self.size = size
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        wantsLayer = true
        widthAnchor.constraint(equalToConstant: size).isActive = true
        heightAnchor.constraint(equalToConstant: size).isActive = true
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var allowsVibrancy: Bool { false }

    var color: NSColor {
        switch status {
        case .online: .systemGreen
        case .pairing: .systemOrange
        case .offline: .tertiaryLabelColor
        }
    }

    override func draw(_ dirtyRect: NSRect) {
        let box = NSRect(
            x: (bounds.width - size) / 2, y: (bounds.height - size) / 2, width: size, height: size)
        color.setFill()
        NSBezierPath(ovalIn: box).fill()
    }
}

/// Borderless symbol button that fills a rounded rect under the pointer.
final class HoverButton: NSButton {
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { needsDisplay = true } }

    /// With a `title` the button is the symbol and the word, as wide as they need.
    init(
        symbol: String, pointSize: CGFloat = 15, title: String? = nil, tooltip: String, target: AnyObject?,
        action: Selector
    ) {
        super.init(frame: .zero)
        image = NSImage(systemSymbolName: symbol, accessibilityDescription: tooltip)
        symbolConfiguration = NSImage.SymbolConfiguration(pointSize: pointSize, weight: .regular)
        label = title
        imagePosition = .imageOnly
        if let title { setAccessibilityTitle(title) }
        isBordered = false
        contentTintColor = .secondaryLabelColor
        toolTip = tooltip
        self.target = target
        self.action = action
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// The word after the symbol. The two are drawn here as one line of text, the symbol as an
    /// attachment: the text system sets it on the capitals' center line and in the text's color,
    /// where the button cell places and tints a symbol and a title each on its own.
    private var label: String?
    private static let labelFont = NSFont.systemFont(ofSize: 13)
    private static let labelPadding: CGFloat = 8
    private static let symbolLift: CGFloat = 0.5

    private var labelText: NSAttributedString? {
        guard let label, let image else { return nil }
        let attachment = NSTextAttachment()
        attachment.image = image.withSymbolConfiguration(symbolConfiguration ?? .init())
        let text = NSMutableAttributedString(attachment: attachment)
        // The text system sets the symbol under the capitals' center by this much.
        text.addAttribute(.baselineOffset, value: Self.symbolLift, range: NSRange(location: 0, length: text.length))
        text.append(NSAttributedString(string: " ", attributes: [.kern: 3]))
        text.append(NSAttributedString(string: label))
        text.addAttributes(
            [.font: Self.labelFont, .foregroundColor: NSColor.secondaryLabelColor],
            range: NSRange(location: 0, length: text.length))
        return text
    }

    override var intrinsicContentSize: NSSize {
        guard let labelText else { return NSSize(width: 28, height: 28) }
        return NSSize(width: ceil(labelText.size().width) + 2 * Self.labelPadding, height: 28)
    }

    // The push bezel's layout padding would stretch the square hover fill into a rectangle.
    override var alignmentRectInsets: NSEdgeInsets { NSEdgeInsets() }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self)
        addTrackingArea(area)
        tracking = area
        // A button hidden mid-hover (a collapsed sidebar) never gets its exit event.
        if let window {
            isHovered =
                window.isKeyWindow
                && bounds.contains(convert(window.mouseLocationOutsideOfEventStream, from: nil))
        }
    }

    // A button whose click removes it from the window (the sidebar swap) never gets its exit event
    // either, and would come back drawn hovered.
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        if window == nil { isHovered = false }
    }

    override func mouseEntered(with event: NSEvent) { isHovered = true }
    override func mouseExited(with event: NSEvent) { isHovered = false }

    // With a menu attached the button is a pull-down: the menu opens on press, its top-left corner
    // at the button's bottom-left. NSButton is flipped, so the bottom edge is at bounds.maxY.
    override func mouseDown(with event: NSEvent) {
        guard let menu else { return super.mouseDown(with: event) }
        isHighlighted = true
        menu.popUp(positioning: nil, at: NSPoint(x: 0, y: bounds.maxY), in: self)
        isHighlighted = false
        // Menu tracking swallows the exit event when the pointer leaves while the menu is open.
        isHovered = window.map { bounds.contains(convert($0.mouseLocationOutsideOfEventStream, from: nil)) } ?? false
    }

    override func draw(_ dirtyRect: NSRect) {
        if isHovered || isHighlighted {
            NSColor.labelColor.withAlphaComponent(isHighlighted ? 0.14 : 0.08).setFill()
            NSBezierPath(roundedRect: bounds, xRadius: 6, yRadius: 6).fill()
        }
        guard let labelText else { return super.draw(dirtyRect) }
        // The capitals sit on the button's center line. NSButton is flipped, so the line's top is
        // the baseline less the ascender.
        let font = Self.labelFont
        let baseline = bounds.midY + font.capHeight / 2
        // The lifted symbol makes the line that much taller, above the baseline.
        let top = baseline - font.ascender - Self.symbolLift
        let scale = window?.backingScaleFactor ?? 2
        labelText.draw(at: NSPoint(x: Self.labelPadding, y: (top * scale).rounded() / scale))
    }
}

extension NSView {
    /// Opts a subview out of Auto Layout so its parent can position it in `layout()`.
    /// Without this, a constraint-less subview keeps the window asking for more
    /// layout passes until AppKit throws.
    @discardableResult
    func framePositioned() -> Self {
        translatesAutoresizingMaskIntoConstraints = true
        return self
    }

    func pin(to other: NSView, insets: NSEdgeInsets = NSEdgeInsets()) {
        translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            leadingAnchor.constraint(equalTo: other.leadingAnchor, constant: insets.left),
            trailingAnchor.constraint(equalTo: other.trailingAnchor, constant: -insets.right),
            topAnchor.constraint(equalTo: other.topAnchor, constant: insets.top),
            bottomAnchor.constraint(equalTo: other.bottomAnchor, constant: -insets.bottom),
        ])
    }
}

// MARK: - Settings pop-up

/// A pop-up button as System Settings' rows have it: the choice as plain text, as wide as it
/// needs, then the up-down chevrons on a small rounded platter. AppKit's bezel styles all draw a
/// full-width capsule; this look is SwiftUI's grouped-form picker, so it is drawn here.
final class SettingsPopUpButton: NSPopUpButton {
    private static let platter = NSSize(width: 19, height: 19)
    private static let gap: CGFloat = 8
    /// Room ahead of the title for the platter that spans the whole button under the pointer.
    private static let leading: CGFloat = 9
    private static let titleFont = NSFont.systemFont(ofSize: 13)
    private var tracking: NSTrackingArea?
    private var isHovered = false { didSet { if isHovered != oldValue { needsDisplay = true } } }

    init() {
        super.init(frame: .zero, pullsDown: false)
        isBordered = false
        font = Self.titleFont
        (cell as? NSPopUpButtonCell)?.arrowPosition = .noArrow
        setContentHuggingPriority(.required, for: .horizontal)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    private var titleText: NSAttributedString {
        NSAttributedString(
            string: titleOfSelectedItem ?? "",
            attributes: [
                .font: Self.titleFont,
                .foregroundColor: isEnabled ? NSColor.labelColor : NSColor.disabledControlTextColor,
            ])
    }

    override var intrinsicContentSize: NSSize {
        NSSize(
            width: Self.leading + ceil(titleText.size().width) + Self.gap + Self.platter.width, height: 24)
    }

    override func updateTrackingAreas() {
        super.updateTrackingAreas()
        if let tracking { removeTrackingArea(tracking) }
        let area = NSTrackingArea(
            rect: bounds, options: [.mouseEnteredAndExited, .activeInKeyWindow], owner: self)
        addTrackingArea(area)
        tracking = area
    }

    override func mouseEntered(with event: NSEvent) { isHovered = isEnabled }
    override func mouseExited(with event: NSEvent) { isHovered = false }

    // Menu tracking swallows the exit event when the pointer leaves while the menu is open.
    override func mouseDown(with event: NSEvent) {
        super.mouseDown(with: event)
        isHovered = window.map { bounds.contains(convert($0.mouseLocationOutsideOfEventStream, from: nil)) } ?? false
    }

    /// The resting platter lines up with the row's margin; the one under the pointer runs a
    /// little past it, so the frame is that much wider than what layout aligns.
    private static let overhang: CGFloat = 4
    override var alignmentRectInsets: NSEdgeInsets {
        NSEdgeInsets(top: 0, left: 0, bottom: 0, right: Self.overhang)
    }

    // The title follows the selection, and the button's width follows the title.
    override func synchronizeTitleAndSelectedItem() {
        super.synchronizeTitleAndSelectedItem()
        invalidateIntrinsicContentSize()
        needsDisplay = true
    }

    override func draw(_ dirtyRect: NSRect) {
        let platter = NSRect(
            x: bounds.maxX - Self.overhang - Self.platter.width, y: ((bounds.height - Self.platter.height) / 2).rounded(),
            width: Self.platter.width, height: Self.platter.height)
        // Under the pointer the platter grows to hold the title too, as System Settings' does.
        NSColor.labelColor.withAlphaComponent(isHighlighted ? 0.16 : 0.08).setFill()
        NSBezierPath(roundedRect: isHovered || isHighlighted ? bounds : platter, xRadius: 8, yRadius: 8).fill()

        let configuration = NSImage.SymbolConfiguration(pointSize: 9.5, weight: .semibold)
            .applying(.init(paletteColors: [isEnabled ? .labelColor : .disabledControlTextColor]))
        if let chevrons = NSImage(systemSymbolName: "chevron.up.chevron.down", accessibilityDescription: nil)?
            .withSymbolConfiguration(configuration)
        {
            let size = chevrons.size
            chevrons.draw(
                in: NSRect(
                    x: platter.midX - size.width / 2, y: platter.midY - size.height / 2, width: size.width,
                    height: size.height),
                from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
        }

        let title = titleText
        let size = title.size()
        title.draw(
            at: NSPoint(
                x: max(0, platter.minX - Self.gap - size.width), y: ((bounds.height - size.height) / 2).rounded()))
    }
}
