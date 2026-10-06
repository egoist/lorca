import AppKit

/// The line above a reply's bubble: an arrow, then who wrote the message it answers and how
/// that message opens, on one line that truncates. It hugs the bubble's side; a click brings
/// the original into view.
final class ReplyQuoteView: NSView {
    var onClick: (() -> Void)?
    /// The side the line hugs: the trailing edge over a user bubble.
    var alignment: NSTextAlignment = .right { didSet { needsDisplay = true } }

    private var name = ""
    private var text = ""
    private static let iconSize: CGFloat = 11
    private static let gap: CGFloat = 4

    override var isFlipped: Bool { true }

    func configure(name: String, text: String) {
        self.name = name
        self.text = text
        setAccessibilityLabel(L("In reply to %@: %@", name, text))
        needsDisplay = true
    }

    private var line: NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.lineBreakMode = .byTruncatingTail
        let line = NSMutableAttributedString(
            string: "\(name): ",
            attributes: [.font: NSFont.systemFont(ofSize: 11.5, weight: .semibold), .foregroundColor: NSColor.secondaryLabelColor, .paragraphStyle: paragraph])
        line.append(NSAttributedString(
            string: text,
            attributes: [.font: NSFont.systemFont(ofSize: 11.5), .foregroundColor: NSColor.secondaryLabelColor, .paragraphStyle: paragraph]))
        return line
    }

    override func draw(_ dirtyRect: NSRect) {
        let line = self.line
        let room = max(0, bounds.width - Self.iconSize - Self.gap)
        let width = min(ceil(line.size().width), room)
        let textX = alignment == .right ? bounds.maxX - width : Self.iconSize + Self.gap
        let height = ceil(line.size().height)
        line.draw(with: NSRect(x: textX, y: (bounds.height - height) / 2, width: width, height: height), options: [.usesLineFragmentOrigin, .truncatesLastVisibleLine])

        let config = NSImage.SymbolConfiguration(pointSize: 9.5, weight: .semibold)
            .applying(NSImage.SymbolConfiguration(paletteColors: [.tertiaryLabelColor]))
        guard let icon = NSImage(systemSymbolName: "arrowshape.turn.up.left.fill", accessibilityDescription: nil)?.withSymbolConfiguration(config)
        else { return }
        let iconX = alignment == .right ? textX - Self.gap - Self.iconSize : 0
        let iconRect = NSRect(
            x: iconX + (Self.iconSize - icon.size.width) / 2, y: (bounds.height - icon.size.height) / 2,
            width: icon.size.width, height: icon.size.height)
        icon.draw(in: iconRect, from: .zero, operation: .sourceOver, fraction: 1, respectFlipped: true, hints: nil)
    }

    override func resetCursorRects() {
        addCursorRect(bounds, cursor: .pointingHand)
    }

    // Claimed here, so the click never goes to the table underneath.
    override func mouseDown(with event: NSEvent) {}

    override func mouseUp(with event: NSEvent) {
        guard bounds.contains(convert(event.locationInWindow, from: nil)) else { return }
        onClick?()
    }

    override func isAccessibilityElement() -> Bool { true }
    override func accessibilityRole() -> NSAccessibility.Role? { .button }
    override func accessibilityPerformPress() -> Bool {
        onClick?()
        return true
    }
}

/// Above the composer's text while the draft answers a message: an arrow, "Replying to Scout"
/// with the message's opening words, and a button that drops the reply, as Escape does.
final class ComposerReplyBar: NSView {
    var onCancel: (() -> Void)?

    private let icon = NSImageView()
    private let label = NSTextField(labelWithString: "")
    private let close = NSButton()

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false

        icon.image = NSImage(systemSymbolName: "arrowshape.turn.up.left.fill", accessibilityDescription: nil)
        icon.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 11, weight: .semibold)
        icon.contentTintColor = .secondaryLabelColor
        icon.translatesAutoresizingMaskIntoConstraints = false
        icon.setContentHuggingPriority(.required, for: .horizontal)

        label.lineBreakMode = .byTruncatingTail
        label.maximumNumberOfLines = 1
        label.cell?.truncatesLastVisibleLine = true
        label.translatesAutoresizingMaskIntoConstraints = false
        label.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

        close.image = NSImage(systemSymbolName: "xmark.circle.fill", accessibilityDescription: L("Cancel reply"))
        close.symbolConfiguration = NSImage.SymbolConfiguration(pointSize: 13, weight: .regular)
        close.contentTintColor = .tertiaryLabelColor
        close.isBordered = false
        close.toolTip = L("Cancel reply")
        close.target = self
        close.action = #selector(cancel)
        close.translatesAutoresizingMaskIntoConstraints = false
        close.setContentHuggingPriority(.required, for: .horizontal)

        addSubview(icon)
        addSubview(label)
        addSubview(close)
        NSLayoutConstraint.activate([
            icon.leadingAnchor.constraint(equalTo: leadingAnchor),
            icon.centerYAnchor.constraint(equalTo: centerYAnchor),
            label.leadingAnchor.constraint(equalTo: icon.trailingAnchor, constant: 6),
            label.centerYAnchor.constraint(equalTo: centerYAnchor),
            close.leadingAnchor.constraint(greaterThanOrEqualTo: label.trailingAnchor, constant: 8),
            close.trailingAnchor.constraint(equalTo: trailingAnchor),
            close.centerYAnchor.constraint(equalTo: centerYAnchor),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    func configure(name: String, text: String) {
        let line = NSMutableAttributedString(
            string: L("Replying to %@", name),
            attributes: [.font: NSFont.systemFont(ofSize: 12, weight: .semibold), .foregroundColor: NSColor.labelColor])
        line.append(NSAttributedString(
            string: "  \(text)", attributes: [.font: NSFont.systemFont(ofSize: 12), .foregroundColor: NSColor.secondaryLabelColor]))
        label.attributedStringValue = line
    }

    @objc private func cancel() {
        onCancel?()
    }
}
