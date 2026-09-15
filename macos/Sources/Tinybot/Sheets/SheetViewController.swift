import AppKit

/// Shared sheet chrome: title, optional subtitle, content, and a trailing button row.
class SheetViewController: NSViewController {
    let contentStack = Build.stack([], spacing: 12)

    private let titleLabel = Build.label("", font: .systemFont(ofSize: 15, weight: .semibold))
    private let subtitleLabel = Build.label(
        "", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private let buttonRow = Build.stack([], orientation: .horizontal, spacing: 10)

    private(set) var confirmButton = NSButton()
    private var sheetWidth: CGFloat = 420

    init(title: String, subtitle: String, width: CGFloat = 420) {
        super.init(nibName: nil, bundle: nil)
        sheetWidth = width
        titleLabel.stringValue = title
        subtitleLabel.stringValue = subtitle
        subtitleLabel.isHidden = subtitle.isEmpty
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false

        contentStack.orientation = .vertical
        contentStack.alignment = .leading

        let header = Build.stack([titleLabel, subtitleLabel], spacing: 4)

        container.addSubview(header)
        container.addSubview(contentStack)
        container.addSubview(buttonRow)

        NSLayoutConstraint.activate([
            container.widthAnchor.constraint(equalToConstant: sheetWidth),

            header.topAnchor.constraint(equalTo: container.topAnchor, constant: 20),
            header.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 20),
            header.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -20),

            contentStack.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 16),
            contentStack.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: 20),
            contentStack.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -20),

            buttonRow.topAnchor.constraint(equalTo: contentStack.bottomAnchor, constant: 20),
            buttonRow.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -20),
            buttonRow.bottomAnchor.constraint(equalTo: container.bottomAnchor, constant: -20),
        ])

        view = container
    }

    func setButtons(confirm: String, cancel: String = "Cancel") {
        let cancelButton = NSButton(title: cancel, target: self, action: #selector(dismissSheet))
        cancelButton.bezelStyle = .rounded
        cancelButton.keyEquivalent = "\u{1b}"

        confirmButton = NSButton(title: confirm, target: self, action: #selector(confirmTapped))
        confirmButton.bezelStyle = .rounded
        confirmButton.keyEquivalent = "\r"

        buttonRow.addArrangedSubview(cancelButton)
        buttonRow.addArrangedSubview(confirmButton)
    }

    /// AppKit sizes a presented sheet once and afterwards only lets it grow with its content.
    /// Hiding content leaves slack that the row stacks pour into their first row, so shrink the
    /// sheet explicitly after showing or hiding anything.
    func fitSheetToContent() {
        view.layoutSubtreeIfNeeded()
        preferredContentSize = view.fittingSize
    }

    @objc func dismissSheet() {
        dismiss(nil)
    }

    @objc func confirmTapped() {
        dismiss(nil)
    }
}
