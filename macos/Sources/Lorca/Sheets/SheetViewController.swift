import AppKit

/// Shared sheet chrome: title, optional subtitle, content, and a trailing button row.
class SheetViewController: NSViewController {
    let contentStack = Build.stack([], spacing: 12)

    private let titleLabel = Build.label("", font: .systemFont(ofSize: 15, weight: .semibold))
    private let subtitleLabel = Build.label(
        "", font: .systemFont(ofSize: 12), color: .secondaryLabelColor, lines: 0)
    private let buttonRow = Build.stack([], orientation: .horizontal, spacing: 10)

    private(set) var confirmButton = NSButton()
    private(set) var cancelButton: NSButton?
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

    private var buttonRowLeading: NSLayoutConstraint?

    /// `cancel: nil` leaves only the confirm button, which then answers Escape as well. Called
    /// again, it replaces the buttons, for a sheet that moves on to another step.
    func setButtons(confirm: String, cancel: String? = L("Cancel"), leading: NSView? = nil) {
        buttonRow.arrangedSubviews.forEach { $0.removeFromSuperview() }
        buttonRowLeading?.isActive = false
        buttonRowLeading = nil
        confirmButton = NSButton(title: confirm, target: self, action: #selector(confirmTapped))
        confirmButton.bezelStyle = .rounded
        confirmButton.keyEquivalent = "\r"

        if let leading {
            buttonRowLeading = buttonRow.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20)
            buttonRowLeading?.isActive = true
            buttonRow.addArrangedSubview(leading)
            buttonRow.addArrangedSubview(NSView())
        }

        if let cancel {
            let cancelButton = NSButton(title: cancel, target: self, action: #selector(dismissSheet))
            cancelButton.bezelStyle = .rounded
            cancelButton.keyEquivalent = "\u{1b}"
            buttonRow.addArrangedSubview(cancelButton)
            self.cancelButton = cancelButton
        }
        buttonRow.addArrangedSubview(confirmButton)
    }

    override func cancelOperation(_ sender: Any?) {
        dismissSheet()
    }

    /// A label in the labels' column and a control filling the rest of the line, as New Bot's
    /// form lays them out. A control taller than a line keeps its label by its first line.
    func formRow(_ title: String, _ control: NSView, topAligned: Bool = false) -> NSView {
        let container = NSView()
        container.translatesAutoresizingMaskIntoConstraints = false
        control.translatesAutoresizingMaskIntoConstraints = false
        let label = Build.label(title, font: .systemFont(ofSize: 12), color: .secondaryLabelColor)
        container.addSubview(label)
        container.addSubview(control)
        let labelAlignment = topAligned
            ? label.topAnchor.constraint(equalTo: control.topAnchor, constant: 6)
            : label.centerYAnchor.constraint(equalTo: control.centerYAnchor)
        NSLayoutConstraint.activate([
            label.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            labelAlignment,
            label.widthAnchor.constraint(equalToConstant: 76),
            control.leadingAnchor.constraint(equalTo: label.trailingAnchor, constant: 10),
            control.trailingAnchor.constraint(lessThanOrEqualTo: container.trailingAnchor),
            control.topAnchor.constraint(equalTo: container.topAnchor),
            control.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        if control is NSTextField || control is NSPopUpButton {
            control.trailingAnchor.constraint(equalTo: container.trailingAnchor).isActive = true
        }
        return container
    }

    /// A new title, for a sheet that now shows what it just added.
    func setSheetTitle(_ title: String, subtitle: String? = nil) {
        titleLabel.stringValue = title
        if let subtitle {
            subtitleLabel.stringValue = subtitle
            subtitleLabel.isHidden = subtitle.isEmpty
        }
    }

    func setSheetSubtitle(_ subtitle: String) {
        subtitleLabel.stringValue = subtitle
        subtitleLabel.isHidden = subtitle.isEmpty
    }

    /// AppKit sizes a presented sheet once and afterwards only lets it grow with its content.
    /// Hiding content leaves slack that the row stacks pour into their first row, so shrink the
    /// sheet explicitly after showing or hiding anything. While the sheet is up its window holds
    /// the view's fitting size at the current one, so the content stack says how much is slack.
    func fitSheetToContent() {
        view.layoutSubtreeIfNeeded()
        guard view.window != nil else {
            preferredContentSize = view.fittingSize
            return
        }
        let slack = contentStack.frame.height - contentStack.fittingSize.height
        preferredContentSize = NSSize(width: view.frame.width, height: view.frame.height - slack)
    }

    @objc func dismissSheet() {
        dismiss(nil)
    }

    @objc func confirmTapped() {
        dismiss(nil)
    }
}
