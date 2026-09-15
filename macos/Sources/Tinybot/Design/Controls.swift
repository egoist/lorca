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
    var status: Computer.Status = .offline { didSet { needsDisplay = true } }
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

    init(symbol: String, pointSize: CGFloat = 15, tooltip: String, target: AnyObject?, action: Selector) {
        super.init(frame: .zero)
        image = NSImage(systemSymbolName: symbol, accessibilityDescription: tooltip)
        symbolConfiguration = NSImage.SymbolConfiguration(pointSize: pointSize, weight: .regular)
        imagePosition = .imageOnly
        isBordered = false
        contentTintColor = .secondaryLabelColor
        toolTip = tooltip
        self.target = target
        self.action = action
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override var intrinsicContentSize: NSSize { NSSize(width: 28, height: 28) }

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

    override func mouseEntered(with event: NSEvent) { isHovered = true }
    override func mouseExited(with event: NSEvent) { isHovered = false }

    override func draw(_ dirtyRect: NSRect) {
        if isHovered || isHighlighted {
            NSColor.labelColor.withAlphaComponent(isHighlighted ? 0.14 : 0.08).setFill()
            NSBezierPath(roundedRect: bounds, xRadius: 6, yRadius: 6).fill()
        }
        super.draw(dirtyRect)
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
