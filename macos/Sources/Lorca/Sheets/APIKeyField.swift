import AppKit

/// A provider's API key: masked by default, with an eye button inside the field's bezel that
/// shows or hides the key and keeps the selection while it switches.
final class APIKeyField: NSView {
    private let secureField = APIKeySecureTextField()
    private let revealedField = APIKeyTextField()
    private var isRevealed = false
    private var field: NSTextField { isRevealed ? revealedField : secureField }
    private lazy var revealButton = Build.imageButton(
        symbol: "eye", tooltip: L("Show API key"), target: self, action: #selector(toggleVisibility))

    /// Called on every edit.
    var onChange: (() -> Void)?

    var stringValue: String {
        get { field.stringValue }
        set { field.stringValue = newValue }
    }

    var placeholderString: String? {
        didSet {
            secureField.placeholderString = placeholderString
            revealedField.placeholderString = placeholderString
        }
    }

    var isEnabled = true {
        didSet {
            secureField.isEnabled = isEnabled
            revealedField.isEnabled = isEnabled
            revealButton.isEnabled = isEnabled
        }
    }

    init() {
        super.init(frame: .zero)
        translatesAutoresizingMaskIntoConstraints = false
        revealedField.isHidden = true
        for field in [secureField, revealedField] {
            field.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
            field.delegate = self
            field.setAccessibilityLabel(L("API key"))
            field.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
            addSubview(field)
            field.pin(to: self)
        }
        revealButton.setAccessibilityLabel(L("Show API key"))
        addSubview(revealButton)
        NSLayoutConstraint.activate([
            revealButton.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -4),
            revealButton.centerYAnchor.constraint(equalTo: centerYAnchor),
            revealButton.widthAnchor.constraint(equalToConstant: 24),
            revealButton.heightAnchor.constraint(equalToConstant: 20),
        ])
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    /// Puts the cursor in the field showing now.
    func focus() {
        window?.makeFirstResponder(field)
    }

    /// Empties both fields, so no key stays behind in a closed sheet.
    func clear() {
        secureField.stringValue = ""
        revealedField.stringValue = ""
    }

    @objc private func toggleVisibility() {
        let oldField = field
        let selection = (oldField.currentEditor() as? NSTextView)?.selectedRange()
        if selection != nil { window?.makeFirstResponder(nil) }
        isRevealed.toggle()
        field.stringValue = oldField.stringValue
        oldField.isHidden = true
        field.isHidden = false
        oldField.stringValue = ""
        let label = isRevealed ? L("Hide API key") : L("Show API key")
        revealButton.image = NSImage(systemSymbolName: isRevealed ? "eye.slash" : "eye", accessibilityDescription: label)
        revealButton.toolTip = label
        revealButton.setAccessibilityLabel(label)
        if let selection {
            window?.makeFirstResponder(field)
            (field.currentEditor() as? NSTextView)?.setSelectedRange(selection)
        }
    }
}

extension APIKeyField: NSTextFieldDelegate {
    func controlTextDidChange(_ obj: Notification) {
        onChange?()
    }
}

// Reserve space inside the native bezel for the reveal button, including while editing.
private final class APIKeyTextFieldCell: NSTextFieldCell {
    override func drawingRect(forBounds rect: NSRect) -> NSRect {
        var rect = super.drawingRect(forBounds: rect)
        rect.size.width = max(0, rect.width - 28)
        return rect
    }

    override func resetCursorRect(_ cellFrame: NSRect, in controlView: NSView) {
        let (accessory, text) = cellFrame.divided(atDistance: min(28, cellFrame.width), from: .maxXEdge)
        super.resetCursorRect(text, in: controlView)
        controlView.addCursorRect(accessory, cursor: .arrow)
    }
}

private final class APIKeySecureTextFieldCell: NSSecureTextFieldCell {
    override func drawingRect(forBounds rect: NSRect) -> NSRect {
        var rect = super.drawingRect(forBounds: rect)
        rect.size.width = max(0, rect.width - 28)
        return rect
    }

    override func resetCursorRect(_ cellFrame: NSRect, in controlView: NSView) {
        let (accessory, text) = cellFrame.divided(atDistance: min(28, cellFrame.width), from: .maxXEdge)
        super.resetCursorRect(text, in: controlView)
        controlView.addCursorRect(accessory, cursor: .arrow)
    }
}

private final class APIKeyTextField: NSTextField {
    override class var cellClass: AnyClass? {
        get { APIKeyTextFieldCell.self }
        set {}
    }
}

private final class APIKeySecureTextField: NSSecureTextField {
    override class var cellClass: AnyClass? {
        get { APIKeySecureTextFieldCell.self }
        set {}
    }
}
