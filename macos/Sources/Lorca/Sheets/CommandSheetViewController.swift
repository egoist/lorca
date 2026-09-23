import AppKit

/// The whole command a permission card asks about, to read or copy before answering.
final class CommandSheetViewController: SheetViewController {
    private static let font = NSFont.monospacedSystemFont(ofSize: 12, weight: .regular)
    private static let inset = NSSize(width: 10, height: 9)

    private let command: String
    private let copyButton = CopyFeedbackButton()

    init(title: String, command: String) {
        self.command = command
        super.init(title: title, subtitle: "", width: 600)
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) { fatalError() }

    override func loadView() {
        super.loadView()

        let scroll = NSTextView.scrollableTextView()
        scroll.drawsBackground = false
        scroll.borderType = .noBorder
        scroll.hasVerticalScroller = true
        scroll.autohidesScrollers = true
        scroll.translatesAutoresizingMaskIntoConstraints = false
        if let text = scroll.documentView as? NSTextView {
            text.string = command
            text.font = Self.font
            text.textColor = .labelColor
            text.isEditable = false
            text.isSelectable = true
            text.drawsBackground = false
            text.textContainerInset = Self.inset
        }
        let box = BackgroundView()
        box.fillColor = Theme.codeBackground
        box.cornerRadius = 8
        box.addSubview(scroll)

        // As tall as the command, up to a screenful; longer commands scroll.
        let textWidth: CGFloat = 600 - 40 - Self.inset.width * 2 - 10
        let textHeight = TextMeasure.labelSize(of: NSAttributedString(string: command, attributes: [.font: Self.font]), width: textWidth).height
        NSLayoutConstraint.activate([
            scroll.topAnchor.constraint(equalTo: box.topAnchor),
            scroll.leadingAnchor.constraint(equalTo: box.leadingAnchor),
            scroll.trailingAnchor.constraint(equalTo: box.trailingAnchor),
            scroll.bottomAnchor.constraint(equalTo: box.bottomAnchor),
            box.heightAnchor.constraint(equalToConstant: min(max(textHeight + Self.inset.height * 2 + 4, 44), 380)),
        ])
        contentStack.addArrangedSubview(box)
        box.widthAnchor.constraint(equalTo: contentStack.widthAnchor).isActive = true

        copyButton.title = L("Copy")
        copyButton.bezelStyle = .rounded
        copyButton.target = self
        copyButton.action = #selector(copyCommand)
        setButtons(confirm: L("Done"), cancel: nil, leading: copyButton)
    }

    @objc private func copyCommand() {
        NSPasteboard.general.clearContents()
        if NSPasteboard.general.setString(command, forType: .string) {
            copyButton.showCopied()
        }
    }
}
